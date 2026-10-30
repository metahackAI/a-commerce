//! The turn loop (docs/backend/agent.md §1): route → search tools → plan → run (reads, previews,
//! change sets) → check → answer with blocks and a short text, within the turn's limits.
//!
//! Writes are never run past a person. On the back office every write is previewed into one
//! change set, which is handed to people (`submit`) only when its rule checks pass; the runtime
//! never approves. On the storefront a write that needs no approval is previewed and committed
//! with its `preview_id`, and an order is only prepared: the shopper's button places it.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use acom_auth::Principal;
use acom_contract::{Change, Check, ErrorCode, Preview, ToolError};
use acom_core::Clock;
use acom_core::ids::{ConversationId, TurnId};
use acom_llm::{
    ChatModel, ChatRequest, LlmError, Message, ModelSet, RepairEnv, Repaired, UsageRecord,
};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::blocks::change_set::{ChangeSetView, ItemView};
use crate::blocks::{Block, insights, review, shop, shopper, table};
use crate::checker;
use crate::client::{ItemRef, NewChangeSet, ToolClient};
use crate::executor::{self, StepRecord};
use crate::memory::{ConversationContext, OpenChangeSet, Owner};
use crate::planner::{self, About, Plan, PlanInput, PlanStep};
use crate::profile::{AgentProfile, ChatSurface, Limits, Profiles};
use crate::router;
use crate::store::{StepRow, StoreInfo, StoreSettings, SystemStore, TurnEnd, TurnLog, TurnStart};
use crate::stream::{Command, Emitter, Event, StepKind, StepStatus, TurnUsage};
use crate::tools::{ToolKind, ToolSpec};
use crate::toolsearch;

/// A block button's intent (`ui_event`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UiEvent {
    pub block_id: String,
    pub action_id: String,
    #[serde(default)]
    pub args: Value,
}

/// A screen element the person picked as "this".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ElementContext {
    pub element_ref: String,
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub ids: BTreeMap<String, String>,
}

/// `POST /chat/{surface}/turns` (docs/api/chat.md §1).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TurnRequest {
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub message: Option<String>,
    /// Uploaded files (`{kind, upload_id}`, `upl_…`). Their ids fill a tool's `upload_id`
    /// (`batch_import` reads an attached supplier sheet).
    #[serde(default)]
    pub attachments: Vec<Value>,
    #[serde(default)]
    pub ui_event: Option<UiEvent>,
    #[serde(default)]
    pub context: Vec<ElementContext>,
}

/// The longest message a turn takes.
const MAX_MESSAGE_CHARS: usize = 4000;

impl TurnRequest {
    pub fn validate(&self) -> Result<(), ToolError> {
        let message = self.message.as_deref().map(str::trim).unwrap_or("");
        if message.is_empty() && self.ui_event.is_none() {
            return Err(ToolError::invalid_input(
                "Send a `message`, or a `ui_event` from a block's button.",
            )
            .detail(json!({"field": "message", "reason": "missing"})));
        }
        if message.chars().count() > MAX_MESSAGE_CHARS {
            return Err(ToolError::invalid_input(format!(
                "The message is longer than {MAX_MESSAGE_CHARS} characters."
            ))
            .detail(json!({"field": "message", "reason": "range"})));
        }
        if let Some(id) = &self.conversation_id {
            ConversationId::parse(id.clone()).map_err(|e| {
                ToolError::invalid_input(format!("conversation_id: {e}"))
                    .detail(json!({"field": "conversation_id", "reason": "format"}))
            })?;
        }
        Ok(())
    }

    /// The words a button says for the person: a `say` button whose `args.request` is a
    /// sentence (a `sales_explainer` next step) is taken as if the person had typed it.
    pub fn button_request(&self) -> Option<&str> {
        self.ui_event
            .as_ref()
            .and_then(|e| e.args.get("request"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|r| !r.is_empty())
    }

    /// What the planner reads as the request.
    pub fn text(&self) -> String {
        match (&self.message, &self.ui_event) {
            (Some(m), _) if !m.trim().is_empty() => m.trim().to_owned(),
            (_, Some(_)) if self.button_request().is_some() => {
                self.button_request().unwrap_or_default().to_owned()
            }
            (_, Some(e)) => format!(
                "(The person pressed the button `{}` on block {} with {})",
                e.action_id, e.block_id, e.args
            ),
            _ => String::new(),
        }
    }

    /// Kinds of the attached files (`sheet`, `image`).
    pub fn attachment_kinds(&self) -> Vec<String> {
        self.attachments
            .iter()
            .filter_map(|a| a.get("kind").and_then(Value::as_str))
            .map(str::to_owned)
            .collect()
    }

    /// Ids the person pointed at or a button carried, by field name.
    pub fn pointed(&self) -> BTreeMap<String, Vec<String>> {
        let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut add = |k: &str, v: &str| {
            let list = out.entry(k.to_owned()).or_default();
            if !list.iter().any(|x| x == v) {
                list.push(v.to_owned());
            }
        };
        for c in &self.context {
            for (k, v) in &c.ids {
                add(k, v);
            }
        }
        if let Some(e) = &self.ui_event {
            for (k, v) in e.args.as_object().into_iter().flatten() {
                if let Some(s) = v.as_str().filter(|_| k.ends_with("_id")) {
                    add(k, s);
                }
            }
        }
        for a in &self.attachments {
            if let Some(u) = a.get("upload_id").and_then(Value::as_str) {
                add("upload_id", u);
            }
        }
        out
    }
}

/// A turn ready to run: the caller, the request, and the conversation so far.
#[derive(Debug, Clone)]
pub struct Prepared {
    pub principal: Principal,
    pub request_id: String,
    pub surface: ChatSurface,
    pub request: TurnRequest,
    pub context: ConversationContext,
}

impl Prepared {
    pub fn conversation_id(&self) -> &str {
        &self.context.conversation_id
    }
}

/// The agent runtime: agent definitions, models, and where turns are recorded.
pub struct Runtime {
    profiles: Arc<Profiles>,
    models: ModelSet,
    log: Arc<dyn TurnLog>,
    store: Arc<dyn StoreInfo>,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Runtime")
            .field("agents", &self.profiles.agents.len())
            .field("models", &self.models.models.keys().collect::<Vec<_>>())
            .finish()
    }
}

fn not_found(conversation_id: &str) -> ToolError {
    ToolError::new(
        ErrorCode::NOT_FOUND,
        "There is no such conversation; start a new one without `conversation_id`.",
    )
    .detail(json!({"entity": "conversation", "id": conversation_id}))
}

impl Runtime {
    pub fn new(
        profiles: Arc<Profiles>,
        models: ModelSet,
        log: Arc<dyn TurnLog>,
        store: Arc<dyn StoreInfo>,
    ) -> Self {
        Self {
            profiles,
            models,
            log,
            store,
        }
    }

    pub fn profiles(&self) -> &Profiles {
        &self.profiles
    }

    pub fn models(&self) -> &ModelSet {
        &self.models
    }

    /// Checks the request and loads the conversation. A conversation that belongs to someone
    /// else is reported as not found.
    pub async fn prepare(
        &self,
        principal: Principal,
        request_id: String,
        surface: ChatSurface,
        request: TurnRequest,
    ) -> Result<Prepared, ToolError> {
        request.validate()?;
        let owner = Owner::of(&principal);
        let tenant = principal.tenant.as_str().to_owned();
        let context = match &request.conversation_id {
            None => ConversationContext::new(ConversationId::generate().as_str(), owner),
            Some(id) => {
                let recent = self.profiles.runtime.context.recent_turns;
                match self.log.load(&tenant, id, recent).await {
                    Ok(Some(ctx)) if ctx.owner.same_person(&owner) => ctx,
                    Ok(Some(_)) => return Err(not_found(id)),
                    Ok(None) => ConversationContext::new(id, owner),
                    Err(e) => {
                        tracing::warn!(error = %e, "conversation could not be loaded; starting without history");
                        ConversationContext::new(id, owner)
                    }
                }
            }
        };
        Ok(Prepared {
            principal,
            request_id,
            surface,
            request,
            context,
        })
    }

    /// Earlier turns with their blocks (`GET /chat/{surface}/conversations/{id}`).
    pub async fn history(
        &self,
        principal: &Principal,
        conversation_id: &str,
    ) -> Result<Value, ToolError> {
        let tenant = principal.tenant.as_str();
        match self.log.history(tenant, conversation_id).await {
            Ok(Some((owner, turns))) if owner.same_person(&Owner::of(principal)) => {
                Ok(json!({"conversation_id": conversation_id, "turns": turns}))
            }
            Ok(_) => Err(not_found(conversation_id)),
            Err(e) => {
                tracing::error!(error = %e, "conversation history");
                Err(ToolError::new(
                    ErrorCode::UPSTREAM_UNAVAILABLE,
                    "The conversation store did not answer; try again in a moment.",
                ))
            }
        }
    }

    /// Runs one turn, sending its events. Always ends with `turn.done`.
    pub async fn run(&self, p: Prepared, tools: Arc<dyn ToolClient>, emit: Emitter) {
        let tenant = p.principal.tenant.as_str().to_owned();
        let settings = match tools.store().await {
            Ok(s) => Ok(s),
            Err(e) => {
                tracing::warn!(error = %e.message, "commerce did not give the store settings");
                self.store.settings(&tenant).await
            }
        };
        let settings = match settings {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "store settings unavailable; using UTC and the system clock");
                SystemStore
                    .settings(&tenant)
                    .await
                    .unwrap_or_else(|_| unreachable_settings())
            }
        };
        let Some(profile) = self.profiles.default_for(p.surface) else {
            emit.send(Event::Error {
                code: "internal".into(),
                message: "No agent is defined for this chat; check agents/*.yml.".into(),
            });
            return;
        };
        let turn = Turn::new(self, p, tenant, profile, tools, emit, settings);
        turn.go().await;
    }
}

fn unreachable_settings() -> StoreSettings {
    StoreSettings {
        time: acom_core::StoreTime::new(chrono_tz::UTC),
        currency: String::new(),
        now: acom_core::SystemClock.now(),
    }
}

/// The store clock during one turn: the store's "now" at the start, moving with real time.
#[derive(Debug, Clone)]
struct TurnClock {
    base: DateTime<Utc>,
    started: Instant,
}

impl Clock for TurnClock {
    fn now(&self) -> DateTime<Utc> {
        self.base + chrono::Duration::from_std(self.started.elapsed()).unwrap_or_default()
    }
}

/// How a plan's execution ended.
#[derive(Debug, Clone, PartialEq)]
enum Flow {
    Done,
    /// The plan must change; the sentence says why.
    Revise(String),
    /// Stop here and tell the person.
    Stop(String),
}

/// A turn hit a limit or lost its model.
#[derive(Debug, Clone, PartialEq)]
struct Stop(String);

/// One item of the change set this turn works on.
#[derive(Debug, Clone)]
struct DraftItem {
    tool: String,
    preview_id: String,
    input: Value,
    preview: Option<Preview>,
    /// A correction may preview this item again and replace it.
    replaceable: bool,
}

#[derive(Debug, Clone)]
struct Draft {
    id: String,
    number: Option<i64>,
    version: i32,
    state: String,
    items: Vec<DraftItem>,
    /// Whether this turn changed it.
    touched: bool,
    /// Whether this turn's sentence is already in the set's request text.
    request_noted: bool,
    mismatches: Vec<String>,
}

impl Draft {
    fn from_open(cs: &OpenChangeSet) -> Self {
        Self {
            id: cs.id.clone(),
            number: cs.number,
            version: cs.version,
            state: cs.state.clone(),
            items: cs
                .items
                .iter()
                .map(|i| DraftItem {
                    tool: i.tool.clone(),
                    preview_id: i.preview_id.clone(),
                    input: i.input.clone(),
                    preview: None,
                    replaceable: true,
                })
                .collect(),
            touched: false,
            request_noted: false,
            mismatches: Vec::new(),
        }
    }
}

struct StepHandle {
    id: String,
    position: i32,
    kind: StepKind,
    tool: Option<String>,
    started_at: DateTime<Utc>,
}

/// What a step row records when it ends.
#[derive(Default)]
struct StepEnd {
    summary: String,
    input: Option<Value>,
    output: Option<Value>,
    preview_id: Option<String>,
    change_set_id: Option<String>,
    error: Option<Value>,
}

struct Turn<'r> {
    rt: &'r Runtime,
    principal: Principal,
    tenant: String,
    surface: ChatSurface,
    request_id: String,
    request: TurnRequest,
    text: String,
    pointed: BTreeMap<String, Vec<String>>,
    ctx: ConversationContext,
    turn_id: String,
    profile: &'r AgentProfile,
    emit: Emitter,
    tools: Arc<dyn ToolClient>,
    settings: StoreSettings,
    clock: TurnClock,
    started: Instant,
    limits: Limits,
    logged: bool,
    deferred: Vec<(UsageRecord, Option<String>)>,
    deferred_steps: Vec<StepRow>,
    usage: Vec<UsageRecord>,
    tool_calls: u32,
    revisions: u32,
    step_seq: i32,
    manifest: Vec<ToolSpec>,
    candidates: Vec<String>,
    plan: Option<Plan>,
    history: Vec<StepRecord>,
    /// Rows of blocks shown earlier that steps used this turn (`blk_…`).
    sources: Vec<StepRecord>,
    draft: Option<Draft>,
    view: Option<ChangeSetView>,
    blocks: Vec<Block>,
    problems: Vec<String>,
}

impl<'r> Turn<'r> {
    fn new(
        rt: &'r Runtime,
        p: Prepared,
        tenant: String,
        profile: &'r AgentProfile,
        tools: Arc<dyn ToolClient>,
        emit: Emitter,
        settings: StoreSettings,
    ) -> Self {
        let started = Instant::now();
        Self {
            rt,
            text: p.request.text(),
            pointed: p.request.pointed(),
            principal: p.principal,
            tenant,
            surface: p.surface,
            request_id: p.request_id,
            request: p.request,
            ctx: p.context,
            turn_id: TurnId::generate().as_str().to_owned(),
            profile,
            emit,
            tools,
            clock: TurnClock {
                base: settings.now,
                started,
            },
            settings,
            started,
            limits: rt.profiles.runtime.limits,
            logged: false,
            deferred: Vec::new(),
            deferred_steps: Vec::new(),
            usage: Vec::new(),
            tool_calls: 0,
            revisions: 0,
            step_seq: 0,
            manifest: Vec::new(),
            candidates: Vec::new(),
            plan: None,
            history: Vec::new(),
            sources: Vec::new(),
            draft: None,
            view: None,
            blocks: Vec::new(),
            problems: Vec::new(),
        }
    }

    async fn go(mut self) {
        self.route().await;
        if self.rt.models.is_empty() {
            let message = self.rt.models.not_configured_message();
            return self.fail("upstream_unavailable", message).await;
        }
        self.manifest = match self.tools.manifest().await {
            Ok(m) => m,
            Err(e) => {
                return self
                    .fail(
                        &e.code,
                        format!("The store's tools could not be listed: {}", e.message),
                    )
                    .await;
            }
        };
        self.refresh_open_change_set().await;
        if self.search().await {
            match self.plan_first().await {
                Ok(plan) => self.execute_with_revisions(plan).await,
                Err(Stop(why)) => self.problems.push(why),
            }
        }
        self.finish_change_set().await;
        self.build_blocks();
        let reply = self.reply().await;
        self.finish(Some(reply), None).await;
    }

    // ---- recording -----------------------------------------------------------------------

    fn elapsed(&self) -> Duration {
        self.started.elapsed()
    }

    fn time_left(&self) -> Duration {
        Duration::from_secs(self.limits.seconds).saturating_sub(self.elapsed())
    }

    fn limit_reached(&self) -> Option<String> {
        if self.emit.is_closed() {
            return Some("The person left the conversation.".into());
        }
        if self.time_left().is_zero() {
            return Some(format!(
                "I stopped after {} seconds, the limit for one turn.",
                self.limits.seconds
            ));
        }
        if self.tool_calls >= self.limits.tool_calls {
            return Some(format!(
                "I stopped after {} tool calls, the limit for one turn.",
                self.limits.tool_calls
            ));
        }
        None
    }

    async fn begin_step(
        &mut self,
        kind: StepKind,
        summary: String,
        tool: Option<&str>,
    ) -> StepHandle {
        self.step_seq += 1;
        let h = StepHandle {
            id: format!("step_{}", acom_core::id::new_suffix()),
            position: self.step_seq,
            kind,
            tool: tool.map(str::to_owned),
            started_at: self.clock.now(),
        };
        self.emit.send(Event::Step {
            step_id: h.id.clone(),
            kind,
            summary: summary.clone(),
            tool: h.tool.clone(),
            status: StepStatus::Running,
        });
        let row = self.step_row(
            &h,
            "active",
            StepEnd {
                summary,
                ..StepEnd::default()
            },
            None,
        );
        self.write_step(row).await;
        h
    }

    async fn end_step(&mut self, h: &StepHandle, status: StepStatus, end: StepEnd) {
        self.emit.send(Event::Step {
            step_id: h.id.clone(),
            kind: h.kind,
            summary: end.summary.clone(),
            tool: h.tool.clone(),
            status,
        });
        let word = if status == StepStatus::Failed {
            "failed"
        } else {
            "completed"
        };
        let ended = Some(self.clock.now());
        let row = self.step_row(h, word, end, ended);
        self.write_step(row).await;
    }

    fn step_row(
        &self,
        h: &StepHandle,
        status: &str,
        end: StepEnd,
        ended: Option<DateTime<Utc>>,
    ) -> StepRow {
        StepRow {
            tenant: self.tenant.clone(),
            step_id: h.id.clone(),
            turn_id: self.turn_id.clone(),
            position: h.position,
            kind: h.kind.as_str().to_owned(),
            instrument: h.tool.clone(),
            status: status.to_owned(),
            summary: end.summary,
            input: end.input,
            output: end.output,
            preview_id: end.preview_id,
            change_set_id: end.change_set_id,
            error: end.error,
            started_at: h.started_at,
            ended_at: ended,
        }
    }

    async fn write_step(&mut self, row: StepRow) {
        if !self.logged {
            self.deferred_steps.push(row);
            return;
        }
        if let Err(e) = self.rt.log.step(&row).await {
            tracing::warn!(error = %e, "step not recorded");
        }
    }

    async fn record_call(&mut self, r: UsageRecord, step: Option<String>) {
        self.usage.push(r.clone());
        if !self.logged {
            self.deferred.push((r, step));
            return;
        }
        let position = i32::try_from(self.usage.len()).unwrap_or(i32::MAX);
        if let Err(e) = self
            .rt
            .log
            .model_call(&self.tenant, &self.turn_id, position, step.as_deref(), &r)
            .await
        {
            tracing::warn!(error = %e, "model call not recorded");
        }
    }

    /// Writes the turn row, then what happened before it (routing).
    async fn start_log(&mut self) {
        let start = TurnStart {
            tenant: self.tenant.clone(),
            conversation_id: self.ctx.conversation_id.clone(),
            surface: self.surface.as_str().to_owned(),
            owner: self.ctx.owner.clone(),
            name: self.text.chars().take(80).collect(),
            turn_id: self.turn_id.clone(),
            text: self.text.clone(),
            context: serde_json::to_value(&self.request.context).unwrap_or(Value::Null),
            agent: self.profile.id.clone(),
            request_id: self.request_id.clone(),
            started_at: self.clock.base,
        };
        match self.rt.log.start_turn(&start).await {
            Ok(()) => self.logged = true,
            Err(e) => {
                tracing::warn!(error = %e, "turn not recorded");
                return;
            }
        }
        for row in std::mem::take(&mut self.deferred_steps) {
            self.write_step(row).await;
        }
        for (n, (r, step)) in std::mem::take(&mut self.deferred).into_iter().enumerate() {
            let position = i32::try_from(n + 1).unwrap_or(i32::MAX);
            if let Err(e) = self
                .rt
                .log
                .model_call(&self.tenant, &self.turn_id, position, step.as_deref(), &r)
                .await
            {
                tracing::warn!(error = %e, "model call not recorded");
            }
        }
    }

    // ---- models --------------------------------------------------------------------------

    /// The first configured model among `prefer`, then any other one (Spark takes over when
    /// Glimmer is missing or failing).
    fn models_for(&self, prefer: &[&str]) -> Vec<Arc<dyn ChatModel>> {
        let mut out: Vec<Arc<dyn ChatModel>> = Vec::new();
        let mut push = |m: &Arc<dyn ChatModel>| {
            if !out.iter().any(|x| x.info().alias == m.info().alias) {
                out.push(m.clone());
            }
        };
        for a in prefer {
            if let Some(m) = self.rt.models.get(a) {
                push(m);
            }
        }
        for m in self.rt.models.models.values() {
            push(m);
        }
        out
    }

    /// Asks for JSON, trying the next model when one fails. Returns the answer and the alias.
    async fn model_json(
        &mut self,
        purpose: &str,
        prefer: &[&str],
        req: ChatRequest,
        step: Option<String>,
    ) -> Result<(Value, String), Stop> {
        let mut last = String::from("no model answered");
        for model in self.models_for(prefer) {
            if self.usage.len() as u32 >= self.limits.model_calls {
                return Err(Stop(format!(
                    "I stopped after {} model calls, the limit for one turn.",
                    self.limits.model_calls
                )));
            }
            let left = self.time_left();
            if left.is_zero() {
                return Err(Stop(format!(
                    "I stopped after {} seconds, the limit for one turn.",
                    self.limits.seconds
                )));
            }
            let started_at = self.clock.now();
            let t = Instant::now();
            let res = tokio::time::timeout(left, model.chat(&req, None))
                .await
                .unwrap_or(Err(LlmError::Timeout(left.as_secs())));
            let ms = t.elapsed().as_millis() as u64;
            let mut record = UsageRecord::new(model.info(), purpose, started_at, ms, &res);
            let parsed = res
                .map_err(|e| e.to_string())
                .and_then(|r| r.json().map_err(|e| e.to_string()));
            match parsed {
                Ok(v) => {
                    self.record_call(record, step).await;
                    return Ok((v, model.info().alias.clone()));
                }
                Err(e) => {
                    record = record.failed_validation(e.clone());
                    self.record_call(record, step.clone()).await;
                    tracing::warn!(purpose, model = %model.info().model, error = %e, "model call failed");
                    last = e;
                }
            }
        }
        Err(Stop(format!("The model could not answer: {last}")))
    }

    async fn model_text(
        &mut self,
        purpose: &str,
        prefer: &[&str],
        req: ChatRequest,
    ) -> Option<String> {
        for model in self.models_for(prefer) {
            if self.usage.len() as u32 >= self.limits.model_calls || self.time_left().is_zero() {
                return None;
            }
            let left = self.time_left();
            let started_at = self.clock.now();
            let t = Instant::now();
            let res = tokio::time::timeout(left, model.chat(&req, None))
                .await
                .unwrap_or(Err(LlmError::Timeout(left.as_secs())));
            let record = UsageRecord::new(
                model.info(),
                purpose,
                started_at,
                t.elapsed().as_millis() as u64,
                &res,
            );
            self.record_call(record, None).await;
            if let Ok(r) = res
                && !r.text.trim().is_empty()
            {
                return Some(r.text.trim().to_owned());
            }
        }
        None
    }

    fn worker(&self) -> &str {
        &self.profile.models.worker
    }

    fn planner_model(&self) -> &str {
        &self.profile.models.planner
    }

    // ---- 1. route ------------------------------------------------------------------------

    async fn route(&mut self) {
        let rt = self.rt;
        let profiles = &rt.profiles;
        let agents = profiles.for_surface(self.surface);
        let default = self.profile.id.clone();
        let previous = self.ctx.agent.clone();
        let mut answer = None;
        let started_at = self.clock.now();
        // A button that says a sentence is routed like typed words; other buttons continue the
        // conversation's work. Routing hints (words or attachments that point at one agent) come
        // first, then the phrases an agent lists as its own work; the model chooses otherwise.
        let button_intent =
            self.request.ui_event.is_some() && self.request.button_request().is_none();
        let attached = self.request.attachment_kinds();
        let shortcut = if button_intent {
            None
        } else {
            router::hinted(
                &profiles.runtime.router.hints,
                &agents,
                &self.text,
                &attached,
            )
            .or_else(|| router::by_phrase(&agents, &self.text))
        };
        if shortcut.is_none()
            && !self.rt.models.is_empty()
            && router::needs_model(&agents, button_intent, previous.as_deref())
        {
            let msgs = router::messages(
                &profiles.prompts.router,
                &agents,
                &self.text,
                previous.as_deref(),
            );
            let req = ChatRequest::new("route", msgs)
                .json("route", router::schema(&agents))
                .max_tokens(100);
            let worker = self.worker().to_owned();
            answer = self
                .model_json("route", &[&worker], req, None)
                .await
                .ok()
                .map(|(v, _)| v);
        }
        let r = shortcut.unwrap_or_else(|| {
            router::decide(
                answer.as_ref(),
                &agents,
                &default,
                previous.as_deref(),
                profiles.runtime.router.min_confidence,
            )
        });
        if let Some(p) = self.rt.profiles.get(&r.agent) {
            self.profile = p;
        }
        self.emit.send(Event::TurnStarted {
            turn_id: self.turn_id.clone(),
            conversation_id: self.ctx.conversation_id.clone(),
            agent: self.profile.id.clone(),
        });
        // Routing happened before the turn row existed: record its step and call now.
        let h = StepHandle {
            id: format!("step_{}", acom_core::id::new_suffix()),
            position: {
                self.step_seq += 1;
                self.step_seq
            },
            kind: StepKind::Route,
            tool: None,
            started_at,
        };
        for (_, step) in &mut self.deferred {
            *step = Some(h.id.clone());
        }
        let summary = format!("{} ({})", self.profile.title, r.reason);
        self.emit.send(Event::Step {
            step_id: h.id.clone(),
            kind: StepKind::Route,
            summary: summary.clone(),
            tool: None,
            status: StepStatus::Done,
        });
        let row = self.step_row(
            &h,
            "completed",
            StepEnd {
                summary,
                output: Some(json!({"agent": r.agent, "confidence": r.confidence})),
                ..StepEnd::default()
            },
            Some(self.clock.now()),
        );
        self.deferred_steps.insert(0, row);
        self.start_log().await;
    }

    // ---- 2. tools ------------------------------------------------------------------------

    fn tool(&self, name: &str) -> Option<&ToolSpec> {
        self.manifest.iter().find(|t| t.tool == name)
    }

    /// Reads the conversation's change set again, so a person's approval since the last turn
    /// is seen before anything is added to it.
    async fn refresh_open_change_set(&mut self) {
        let Some(cs) = self.ctx.change_set.clone() else {
            return;
        };
        if !cs.is_open() || self.tool("get_change_set").is_none() {
            return;
        }
        let input = json!({"change_set_id": cs.id});
        self.tool_calls += 1;
        if let Ok(v) = self.tools.read("get_change_set", &input).await
            && let Some(state) = v.get("state").and_then(Value::as_str)
            && let Some(open) = self.ctx.change_set.as_mut()
        {
            state.clone_into(&mut open.state);
            if let Some(n) = v
                .get("version")
                .and_then(Value::as_i64)
                .and_then(|n| i32::try_from(n).ok())
            {
                open.version = n;
            }
        }
    }

    fn open_change_set(&self) -> Option<&OpenChangeSet> {
        self.ctx.change_set.as_ref().filter(|c| c.is_open())
    }

    async fn search(&mut self) -> bool {
        let h = self
            .begin_step(
                StepKind::SearchTools,
                "Looking for the right tools".into(),
                None,
            )
            .await;
        let mut pinned: Vec<String> = self
            .open_change_set()
            .map(|c| c.items.iter().map(|i| i.tool.clone()).collect())
            .unwrap_or_default();
        // The agent's own core reads are always at hand (`tool_scope.always`).
        pinned.extend(self.profile.tool_scope.always.iter().cloned());
        let pinned: Vec<&str> = pinned.iter().map(String::as_str).collect();
        let mut query = self.text.clone();
        for c in &self.request.context {
            query.push(' ');
            query.push_str(&c.label);
        }
        let found = toolsearch::search(
            &self.manifest,
            self.profile,
            &query,
            &pinned,
            &self.rt.profiles.runtime.tool_search,
        );
        self.candidates = found.iter().map(|c| c.tool.clone()).collect();
        let ok = !self.candidates.is_empty();
        let summary = if ok {
            format!(
                "{} candidates: {}",
                self.candidates.len(),
                self.candidates.join(", ")
            )
        } else {
            "No tool in this agent's scope fits".to_owned()
        };
        self.end_step(
            &h,
            if ok {
                StepStatus::Done
            } else {
                StepStatus::Failed
            },
            StepEnd {
                summary,
                output: Some(json!(found)),
                ..StepEnd::default()
            },
        )
        .await;
        if !ok {
            self.problems
                .push("None of the store's tools that I may use fits this request.".into());
        }
        ok
    }

    // ---- 3. plan -------------------------------------------------------------------------

    fn now_text(&self) -> String {
        let local = self.clock.now().with_timezone(&self.settings.time.zone);
        format!(
            "{} ({}, UTC{})",
            local.format("%a %Y-%m-%d %H:%M"),
            self.settings.time.zone.name(),
            local.format("%:z")
        )
    }

    fn candidate_specs(&self) -> Vec<ToolSpec> {
        self.candidates
            .iter()
            .filter_map(|c| self.tool(c).cloned())
            .collect()
    }

    fn done_summaries(&self) -> Vec<Value> {
        self.history
            .iter()
            .map(|r| {
                let mut v = json!({"tool": r.tool, "args": r.input});
                if let Some(o) = &r.output {
                    v["result"] = compact(o);
                }
                if let Some(p) = &r.preview {
                    v["preview"] = json!({"changes": p.changes.len(),
                        "failed_checks": p.checks.iter().filter(|c| !c.passed).map(|c| json!({"rule": c.rule, "detail": c.detail})).collect::<Vec<_>>()});
                }
                v
            })
            .collect()
    }

    async fn plan_once(
        &mut self,
        prefer: &[&str],
        problem: Option<String>,
        draft: Option<Value>,
    ) -> Result<(Plan, String), Stop> {
        let specs = self.candidate_specs();
        let refs: Vec<&ToolSpec> = specs.iter().collect();
        let blocks = self.profile.blocks.clone();
        let max_steps = (self.limits.tool_calls as usize).clamp(1, 12);
        let schema = planner::plan_schema(&refs, &blocks, max_steps);
        let history = self
            .ctx
            .history(self.rt.profiles.runtime.context.recent_turns);
        let pointed: Vec<Value> = self
            .request
            .context
            .iter()
            .map(|c| json!({"element": c.element_ref, "label": c.label, "ids": c.ids}))
            .chain(
                self.request
                    .ui_event
                    .iter()
                    .map(|e| json!({"button": e.action_id, "args": e.args})),
            )
            .chain(
                self.request
                    .attachments
                    .iter()
                    .map(|a| json!({"attached": a})),
            )
            .collect();
        let input = PlanInput {
            shared_prompt: &self.rt.profiles.prompts.planner,
            instructions: &self.profile.instructions_text,
            now: self.now_text(),
            currency: &self.settings.currency,
            request: self.text.clone(),
            pointed,
            history,
            open_change_set: self.open_change_set().map(OpenChangeSet::summary),
            tools: refs.clone(),
            blocks: &blocks,
            done: self.done_summaries(),
            problem,
            draft,
        };
        let req = ChatRequest::new("plan", planner::messages(&input)).json("plan", schema);
        let (raw, alias) = self.model_json("plan", prefer, req, None).await?;
        match planner::validate(&raw, &refs, &blocks) {
            Ok(p) => Ok((p, alias)),
            Err(e) => {
                if let Some(last) = self.usage.last_mut() {
                    last.succeeded = false;
                    last.error = Some(e.clone());
                }
                Err(Stop(format!("The plan could not be used: {e}")))
            }
        }
    }

    async fn plan_first(&mut self) -> Result<Plan, Stop> {
        let h = self
            .begin_step(StepKind::Plan, "Planning".into(), None)
            .await;
        let worker = self.worker().to_owned();
        let planner_alias = self.planner_model().to_owned();
        let first = match self.plan_once(&[&worker, &planner_alias], None, None).await {
            Ok(p) => Ok(p),
            // An unusable plan gets one more try, on the planner model, with the reason.
            Err(Stop(why)) if self.usage.len() < self.limits.model_calls as usize => {
                self.plan_once(&[&planner_alias, &worker], Some(why), None)
                    .await
            }
            Err(s) => Err(s),
        };
        let (mut plan, alias) = match first {
            Ok(p) => p,
            Err(Stop(why)) => {
                self.end_step(
                    &h,
                    StepStatus::Failed,
                    StepEnd {
                        summary: why.clone(),
                        ..StepEnd::default()
                    },
                )
                .await;
                return Err(Stop(why));
            }
        };
        let specs = self.candidate_specs();
        let refs: Vec<&ToolSpec> = specs.iter().collect();
        let big = plan.steps.len() > self.rt.profiles.runtime.planner.worker_max_steps
            || plan.domains(&refs).len() > 1;
        let mut by = alias.clone();
        if big && alias != planner_alias && self.rt.models.get(&planner_alias).is_some() {
            let draft = serde_json::to_value(&plan).ok();
            if let Ok((better, a)) = self.plan_once(&[&planner_alias], None, draft).await {
                plan = better;
                by = a;
            }
        }
        let summary = self.plan_summary(&plan, &by);
        self.end_step(
            &h,
            StepStatus::Done,
            StepEnd {
                summary,
                output: serde_json::to_value(&plan).ok(),
                ..StepEnd::default()
            },
        )
        .await;
        Ok(plan)
    }

    fn plan_summary(&self, plan: &Plan, alias: &str) -> String {
        let name = self
            .rt
            .models
            .get(alias)
            .map_or(alias.to_owned(), |m| m.info().display_name.clone());
        let tools: Vec<&str> = plan.steps.iter().map(|s| s.tool.as_str()).collect();
        if plan.interpretation.is_empty() {
            format!("{} steps by {name}: {}", tools.len(), tools.join(" → "))
        } else {
            let understood: Vec<&str> = plan
                .interpretation
                .iter()
                .map(|u| u.text.as_str())
                .collect();
            format!(
                "{} · {} steps by {name}",
                understood.join(" · "),
                tools.len()
            )
        }
    }

    async fn revise(&mut self, problem: String) -> Result<Plan, Stop> {
        self.revisions += 1;
        let h = self
            .begin_step(
                StepKind::Plan,
                format!("Changing the plan: {problem}"),
                None,
            )
            .await;
        let planner_alias = self.planner_model().to_owned();
        let worker = self.worker().to_owned();
        // After a failure the planner model takes over (§6).
        match self
            .plan_once(&[&planner_alias, &worker], Some(problem), None)
            .await
        {
            Ok((plan, by)) => {
                let summary = self.plan_summary(&plan, &by);
                self.end_step(
                    &h,
                    StepStatus::Done,
                    StepEnd {
                        summary,
                        output: serde_json::to_value(&plan).ok(),
                        ..StepEnd::default()
                    },
                )
                .await;
                Ok(plan)
            }
            Err(Stop(why)) => {
                self.end_step(
                    &h,
                    StepStatus::Failed,
                    StepEnd {
                        summary: why.clone(),
                        ..StepEnd::default()
                    },
                )
                .await;
                Err(Stop(why))
            }
        }
    }

    // ---- 4. run --------------------------------------------------------------------------

    async fn execute_with_revisions(&mut self, mut plan: Plan) {
        loop {
            self.plan = Some(plan.clone());
            match self.execute(&plan).await {
                Flow::Done => return,
                Flow::Stop(why) => {
                    self.problems.push(why);
                    return;
                }
                Flow::Revise(why) if self.revisions < self.limits.plan_revisions => {
                    match self.revise(why).await {
                        Ok(p) => plan = p,
                        Err(Stop(why)) => {
                            self.problems.push(why);
                            return;
                        }
                    }
                }
                Flow::Revise(why) => {
                    self.problems.push(why);
                    return;
                }
            }
        }
    }

    fn repair_env(&self) -> RepairEnv {
        RepairEnv {
            time: self.settings.time,
            now: self.clock.now(),
            currency: self.settings.currency.clone(),
        }
    }

    async fn execute(&mut self, plan: &Plan) -> Flow {
        let mut view: BTreeMap<String, StepRecord> = BTreeMap::new();
        let mut order: Vec<String> = Vec::new();
        for step in &plan.steps {
            if let Some(why) = self.limit_reached() {
                return Flow::Stop(why);
            }
            let Some(tool) = self.tool(&step.tool).cloned() else {
                return Flow::Revise(format!("{} is not available.", step.tool));
            };
            if tool.kind == ToolKind::Person || !self.profile.allows(&tool) {
                return Flow::Stop(format!(
                    "{} is not something I may do; a person does it.",
                    tool.tool
                ));
            }
            if plan.about == About::Other && tool.remembers() {
                self.problems
                    .push("I did not remember anything about the person this is for.".into());
                continue;
            }
            // Blocks shown earlier in the conversation stand in for steps (`blk_…`).
            for id in step.uses.iter().chain(step.each.iter()) {
                if id.starts_with("blk_")
                    && !view.contains_key(id)
                    && let Some(r) = self.block_source(id)
                {
                    view.insert(id.clone(), r);
                    order.push(id.clone());
                }
            }
            let result = match &step.each {
                Some(source) => self.run_each(plan, step, &tool, source, &view).await,
                None => {
                    let bound = executor::bind(step, &tool, &view, &order, &self.pointed);
                    let mut input = bound.input;
                    if tool.is_write()
                        && self.surface == ChatSurface::Bo
                        && let Some(base) = self.replaceable_input(&tool.tool)
                    {
                        input = executor::merge(&base, &input);
                    }
                    self.call(plan, &step.id, &tool, input, bound.notes)
                        .await
                        .map(Some)
                }
            };
            match result {
                Ok(Some(record)) => {
                    view.insert(step.id.clone(), record);
                    order.push(step.id.clone());
                }
                Ok(None) => {}
                // A memory that cannot be kept (a guest has none) does not stop the answer.
                Err(Flow::Stop(why)) if tool.remembers() => {
                    self.problems
                        .push(format!("I could not remember that: {why}"));
                }
                Err(flow) => return flow,
            }
        }
        Flow::Done
    }

    /// The rows of a block shown earlier in this conversation, as a step's result.
    fn block_source(&mut self, block_id: &str) -> Option<StepRecord> {
        let block = self.ctx.block(block_id)?;
        let data = block.get("data").cloned().unwrap_or(Value::Null);
        let rows = executor::rows(&data);
        let record = StepRecord {
            step: block_id.to_owned(),
            tool: block
                .get("ui")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            kind: ToolKind::Read,
            input: Value::Null,
            slots: executor::slots_from_rows(&rows),
            output: Some(json!({"items": rows, "total": rows.len()})),
            preview: None,
            item: None,
        };
        self.sources.push(record.clone());
        Some(record)
    }

    /// One tool call: repair the arguments (asking the worker once when they cannot be), reuse
    /// a read made with the same arguments this turn, else read, preview into the change set, or
    /// preview and commit the shopper's write.
    async fn call(
        &mut self,
        plan: &Plan,
        step: &str,
        tool: &ToolSpec,
        input: Value,
        notes: Vec<String>,
    ) -> Result<StepRecord, Flow> {
        let mut repaired = acom_llm::repair(&input, &tool.input_schema, &self.repair_env());
        if !repaired.is_valid()
            && let Some(fixed) = self.ask_arguments(tool, &repaired).await
        {
            repaired = fixed;
        }
        if !repaired.is_valid() {
            let mut why: Vec<String> = repaired.errors.clone();
            if !repaired.missing.is_empty() {
                why.push(format!("missing {}", repaired.missing.join(", ")));
            }
            let h = self
                .begin_step(
                    StepKind::CallTool,
                    format!("Preparing {}", tool.tool),
                    Some(&tool.tool),
                )
                .await;
            let sentence = format!(
                "The arguments for {} are not usable: {}.",
                tool.tool,
                why.join("; ")
            );
            self.end_step(
                &h,
                StepStatus::Failed,
                StepEnd {
                    summary: sentence.clone(),
                    input: Some(repaired.value),
                    ..StepEnd::default()
                },
            )
            .await;
            return Err(Flow::Revise(sentence));
        }
        let mut notes = notes;
        notes.extend(repaired.notes.iter().cloned());
        let input = repaired.value;
        // Same read with the same arguments earlier this turn: reuse its result.
        if !tool.is_write()
            && let Some(prev) = self
                .history
                .iter()
                .find(|r| r.tool == tool.tool && r.input == input)
                .cloned()
        {
            return Ok(StepRecord {
                step: step.to_owned(),
                ..prev
            });
        }
        let record = match (tool.kind, self.surface) {
            (ToolKind::Read, _) => self.run_read(step, tool, input, &notes).await,
            (_, ChatSurface::Bo) => {
                self.run_change_set_write(plan, step, tool, input, &notes)
                    .await
            }
            (_, ChatSurface::Fo) => self.run_shopper_write(step, tool, input, &notes).await,
        }?;
        self.history.push(record.clone());
        Ok(record)
    }

    /// A step with `each`: one call per row of the source (filtered by `when`), each taking its
    /// ids from its row. Returns one record whose result lists every row's result, or `None`
    /// when no row matched.
    async fn run_each(
        &mut self,
        plan: &Plan,
        step: &PlanStep,
        tool: &ToolSpec,
        source: &str,
        view: &BTreeMap<String, StepRecord>,
    ) -> Result<Option<StepRecord>, Flow> {
        let Some(src) = view.get(source) else {
            return Err(Flow::Revise(format!(
                "{} runs for each row of {source}, which has no result.",
                tool.tool
            )));
        };
        let rows: Vec<Value> = src
            .output
            .as_ref()
            .map(executor::rows)
            .unwrap_or_default()
            .into_iter()
            .filter(|r| executor::row_matches(r, step.when.as_ref()))
            .collect();
        let mut items: Vec<Value> = Vec::new();
        let mut slots: BTreeMap<String, Vec<Value>> = BTreeMap::new();
        let mut last: Option<StepRecord> = None;
        for row in rows {
            if let Some(why) = self.limit_reached() {
                return Err(Flow::Stop(why));
            }
            let bound = executor::bind_row(step, tool, &row);
            let record = self
                .call(plan, &step.id, tool, bound.input, bound.notes)
                .await?;
            if let Some(o) = &record.output {
                items.extend(executor::rows(o));
            }
            for (k, vs) in &record.slots {
                let list = slots.entry(k.clone()).or_default();
                for v in vs {
                    if !list.contains(v) {
                        list.push(v.clone());
                    }
                }
            }
            last = Some(record);
        }
        Ok(last.map(|last| {
            let total = items.len();
            StepRecord {
                step: step.id.clone(),
                output: (!tool.is_write()).then(|| json!({"items": items, "total": total})),
                slots,
                item: None,
                ..last
            }
        }))
    }

    /// The arguments the model wrote could not be repaired: ask the worker once to fill them.
    async fn ask_arguments(&mut self, tool: &ToolSpec, r: &Repaired) -> Option<Repaired> {
        if self.usage.len() as u32 >= self.limits.model_calls {
            return None;
        }
        let mut problems = r.errors.clone();
        if !r.missing.is_empty() {
            problems.push(format!("Missing: {}", r.missing.join(", ")));
        }
        let user = format!(
            "Request: {}\nTool: {}\nArguments so far: {}\nProblems: {}\nResults so far: {}\nStore clock: {}",
            self.text,
            planner::describe_tool(tool),
            r.value,
            problems.join("; "),
            Value::Array(self.done_summaries()),
            self.now_text()
        );
        let req = ChatRequest::new(
            "arguments",
            vec![
                Message::system(self.rt.profiles.prompts.arguments.trim()),
                Message::user(user),
            ],
        )
        .json("arguments", tool.input_schema.clone());
        let worker = self.worker().to_owned();
        let (v, _) = self
            .model_json("arguments", &[&worker], req, None)
            .await
            .ok()?;
        let merged = executor::merge(&r.value, &v);
        Some(acom_llm::repair(
            &merged,
            &tool.input_schema,
            &self.repair_env(),
        ))
    }

    /// The input of an item this turn may replace (a correction), when there is one.
    fn replaceable_input(&self, tool: &str) -> Option<Value> {
        if let Some(d) = &self.draft {
            return d
                .items
                .iter()
                .find(|i| i.tool == tool && i.replaceable)
                .map(|i| i.input.clone());
        }
        self.open_change_set()?
            .items
            .iter()
            .find(|i| i.tool == tool)
            .map(|i| i.input.clone())
    }

    /// A tool error: try again with the tool commerce suggests when this agent may use it,
    /// otherwise stop and show the reason.
    fn tool_failure(&mut self, tool: &str, e: &ToolError) -> Flow {
        let next = e
            .next_tool
            .as_deref()
            .filter(|n| self.tool(n).is_some_and(|t| self.profile.allows(t)));
        match next {
            Some(n) => {
                if !self.candidates.iter().any(|c| c == n) {
                    self.candidates.push(n.to_owned());
                }
                Flow::Revise(format!(
                    "{tool} failed ({}): {} The tool to use next is {n}.",
                    e.code, e.message
                ))
            }
            None => Flow::Stop(format!("{tool} failed: {}", e.message)),
        }
    }

    async fn run_read(
        &mut self,
        step: &str,
        tool: &ToolSpec,
        input: Value,
        notes: &[String],
    ) -> Result<StepRecord, Flow> {
        self.tool_calls += 1;
        let h = self
            .begin_step(
                StepKind::CallTool,
                format!("Reading with {}", tool.tool),
                Some(&tool.tool),
            )
            .await;
        match self.tools.read(&tool.tool, &input).await {
            Ok(out) => {
                let count = out.get("total").and_then(Value::as_u64).or_else(|| {
                    out.get("items")
                        .and_then(Value::as_array)
                        .map(|a| a.len() as u64)
                });
                let summary = match count {
                    Some(n) => format!("{}: {n} found", tool.tool),
                    None => format!("{}: read", tool.tool),
                };
                self.end_step(
                    &h,
                    StepStatus::Done,
                    StepEnd {
                        summary,
                        input: Some(json!({"args": input, "repairs": notes})),
                        output: Some(out.clone()),
                        ..StepEnd::default()
                    },
                )
                .await;
                Ok(StepRecord {
                    step: step.to_owned(),
                    tool: tool.tool.clone(),
                    kind: tool.kind,
                    slots: executor::slots_from_read(tool, &out),
                    input,
                    output: Some(out),
                    preview: None,
                    item: None,
                })
            }
            Err(e) => {
                self.end_step(
                    &h,
                    StepStatus::Failed,
                    StepEnd {
                        summary: e.message.clone(),
                        input: Some(input),
                        error: Some(json!(e)),
                        ..StepEnd::default()
                    },
                )
                .await;
                Err(self.tool_failure(&tool.tool, &e))
            }
        }
    }

    /// The change set this turn adds to: the conversation's open set when the plan corrects it
    /// or it is still drafting, else a new one.
    async fn ensure_change_set(&mut self, plan: &Plan) -> Result<String, Flow> {
        if let Some(d) = &self.draft {
            return Ok(d.id.clone());
        }
        if let Some(open) = self.open_change_set() {
            let corrects = plan
                .steps
                .iter()
                .any(|s| open.items.iter().any(|i| i.tool == s.tool));
            if corrects || open.state == "drafting" {
                let d = Draft::from_open(open);
                let id = d.id.clone();
                self.draft = Some(d);
                return Ok(id);
            }
        }
        let title = plan
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| self.text.chars().take(80).collect());
        let new = NewChangeSet {
            title,
            request_text: vec![self.text.clone()],
            interpretation: plan.interpretation_values(),
            conversation_id: Some(self.ctx.conversation_id.clone()),
        };
        match self.tools.create_change_set(&new).await {
            Ok(head) => {
                self.emit.send(Event::ChangeSetUpdated {
                    change_set_id: head.id.clone(),
                    version: head.version,
                    state: head.state.clone(),
                });
                self.draft = Some(Draft {
                    id: head.id.clone(),
                    number: head.number,
                    version: head.version,
                    state: head.state,
                    items: Vec::new(),
                    touched: true,
                    request_noted: true,
                    mismatches: Vec::new(),
                });
                Ok(head.id)
            }
            Err(e) => Err(Flow::Stop(format!(
                "The change set could not be created: {}",
                e.message
            ))),
        }
    }

    async fn run_change_set_write(
        &mut self,
        plan: &Plan,
        step: &str,
        tool: &ToolSpec,
        input: Value,
        notes: &[String],
    ) -> Result<StepRecord, Flow> {
        let cs = self.ensure_change_set(plan).await?;
        self.tool_calls += 1;
        let h = self
            .begin_step(
                StepKind::CallTool,
                format!("Previewing {} into the change set", tool.tool),
                Some(&tool.tool),
            )
            .await;
        let replace = self.draft.as_ref().and_then(|d| {
            d.items
                .iter()
                .position(|i| i.tool == tool.tool && i.replaceable)
        });
        let result: Result<(Preview, usize), ToolError> = match replace {
            Some(idx) => self.replace_item(&cs, idx, tool, &input).await,
            None => match self
                .tools
                .preview(&tool.tool, &input, Some(cs.as_str()))
                .await
            {
                Ok(p) => {
                    let d = self.draft.as_mut();
                    let position = d.map_or(1, |d| {
                        d.items.push(DraftItem {
                            tool: tool.tool.clone(),
                            preview_id: p.preview_id.clone(),
                            input: input.clone(),
                            preview: Some(p.clone()),
                            replaceable: false,
                        });
                        d.touched = true;
                        d.items.len()
                    });
                    Ok((p, position))
                }
                Err(e) => Err(e),
            },
        };
        let (preview, position) = match result {
            Ok(r) => r,
            Err(e) => {
                self.end_step(
                    &h,
                    StepStatus::Failed,
                    StepEnd {
                        summary: e.message.clone(),
                        input: Some(input),
                        change_set_id: Some(cs),
                        error: Some(json!(e)),
                        ..StepEnd::default()
                    },
                )
                .await;
                return Err(self.tool_failure(&tool.tool, &e));
            }
        };
        let failed = preview.checks.iter().filter(|c| !c.passed).count();
        self.end_step(
            &h,
            StepStatus::Done,
            StepEnd {
                summary: format!(
                    "{}: {} change(s), {failed} of {} check(s) failed",
                    tool.tool,
                    preview.changes.len(),
                    preview.checks.len()
                ),
                input: Some(json!({"args": input, "repairs": notes})),
                output: serde_json::to_value(&preview).ok(),
                preview_id: Some(preview.preview_id.clone()),
                change_set_id: Some(cs.clone()),
                error: None,
            },
        )
        .await;
        let mismatches =
            checker::intent_mismatches(&tool.tool, &input, &preview, &self.settings.time);
        if !mismatches.is_empty() {
            let sentences: Vec<String> =
                mismatches.iter().map(checker::Mismatch::sentence).collect();
            let text = sentences.join(" ");
            let ch = self
                .begin_step(
                    StepKind::Check,
                    format!("Comparing {} with the request", tool.tool),
                    Some(&tool.tool),
                )
                .await;
            self.end_step(
                &ch,
                StepStatus::Failed,
                StepEnd {
                    summary: text.clone(),
                    ..StepEnd::default()
                },
            )
            .await;
            if let Some(d) = self.draft.as_mut() {
                if let Some(item) = d.items.get_mut(position - 1) {
                    item.replaceable = true;
                }
                d.mismatches.push(text.clone());
            }
            return Err(Flow::Revise(text));
        }
        Ok(StepRecord {
            step: step.to_owned(),
            tool: tool.tool.clone(),
            kind: tool.kind,
            slots: executor::slots_from_item(tool, position),
            input,
            output: None,
            preview: Some(preview),
            item: Some(position),
        })
    }

    /// A correction: preview again outside the set, then replace the item (version + 1).
    async fn replace_item(
        &mut self,
        cs: &str,
        idx: usize,
        tool: &ToolSpec,
        input: &Value,
    ) -> Result<(Preview, usize), ToolError> {
        let p = self.tools.preview(&tool.tool, input, None).await?;
        let Some(d) = self.draft.as_mut() else {
            return Err(ToolError::new(
                ErrorCode::INTERNAL,
                "The change set went missing during the turn.",
            ));
        };
        if let Some(item) = d.items.get_mut(idx) {
            item.preview_id = p.preview_id.clone();
            item.input = input.clone();
            item.preview = Some(p.clone());
            item.replaceable = false;
        }
        d.touched = true;
        let refs: Vec<ItemRef> = d
            .items
            .iter()
            .map(|i| ItemRef {
                preview_id: i.preview_id.clone(),
            })
            .collect();
        let head = self.tools.replace_items(cs, &refs).await?;
        let note = self.draft.as_ref().is_some_and(|d| !d.request_noted);
        if let Some(d) = self.draft.as_mut() {
            d.version = head.version;
            d.state.clone_from(&head.state);
            d.request_noted = true;
        }
        // The correction joins what the person asked, so the card shows both sentences.
        if note && let Err(e) = self.tools.append_request(cs, &self.text).await {
            tracing::warn!(error = %e.message, "could not add the correction to the request");
        }
        self.emit.send(Event::ChangeSetUpdated {
            change_set_id: head.id,
            version: head.version,
            state: head.state,
        });
        Ok((p, idx + 1))
    }

    async fn run_shopper_write(
        &mut self,
        step: &str,
        tool: &ToolSpec,
        input: Value,
        notes: &[String],
    ) -> Result<StepRecord, Flow> {
        self.tool_calls += 1;
        let h = self
            .begin_step(
                StepKind::CallTool,
                format!("Previewing {}", tool.tool),
                Some(&tool.tool),
            )
            .await;
        let preview = match self.tools.preview(&tool.tool, &input, None).await {
            Ok(p) => p,
            Err(e) => {
                self.end_step(
                    &h,
                    StepStatus::Failed,
                    StepEnd {
                        summary: e.message.clone(),
                        input: Some(input),
                        error: Some(json!(e)),
                        ..StepEnd::default()
                    },
                )
                .await;
                return Err(self.tool_failure(&tool.tool, &e));
            }
        };
        let failed: Vec<String> = preview
            .checks
            .iter()
            .filter(|c| !c.passed)
            .map(|c| c.detail.clone().unwrap_or_else(|| c.rule.clone()))
            .collect();
        let base = StepEnd {
            input: Some(json!({"args": input, "repairs": notes})),
            preview_id: Some(preview.preview_id.clone()),
            ..StepEnd::default()
        };
        let mut record = StepRecord {
            step: step.to_owned(),
            tool: tool.tool.clone(),
            kind: tool.kind,
            input: input.clone(),
            output: None,
            preview: Some(preview.clone()),
            item: None,
            slots: BTreeMap::new(),
        };
        // Preparing an order stops at the preview: the shopper's button places it.
        if tool.preview_only() {
            record.slots = executor::slots_from_prepared(&preview);
            self.end_step(
                &h,
                StepStatus::Done,
                StepEnd {
                    summary: format!("{}: ready for you to confirm", tool.tool),
                    output: serde_json::to_value(&preview).ok(),
                    ..base
                },
            )
            .await;
            return Ok(record);
        }
        if preview.needs_approval {
            let why = format!(
                "{} needs a store person's approval, so I did not do it.",
                tool.tool
            );
            self.end_step(
                &h,
                StepStatus::Failed,
                StepEnd {
                    summary: why.clone(),
                    ..base
                },
            )
            .await;
            return Err(Flow::Stop(why));
        }
        if !failed.is_empty() {
            let why = format!("{} was not done: {}", tool.tool, failed.join("; "));
            self.end_step(
                &h,
                StepStatus::Failed,
                StepEnd {
                    summary: why.clone(),
                    output: serde_json::to_value(&preview).ok(),
                    ..base
                },
            )
            .await;
            return Err(Flow::Stop(why));
        }
        self.tool_calls += 1;
        match self.tools.commit(&tool.tool, &preview.preview_id).await {
            Ok(result) => {
                record.slots = executor::slots_from_commit(tool, &result);
                if tool.remembers()
                    && let Some((id, text)) = executor::memory_of(&result, Some(&preview))
                {
                    self.emit.send(Event::MemorySaved {
                        memory_id: id.clone(),
                        text: text.clone(),
                    });
                    let note = shop::memory_note(&id, &text);
                    self.emit.send(Event::Block(Box::new(note.clone())));
                    self.blocks.push(note);
                }
                record.output = Some(result.clone());
                self.end_step(
                    &h,
                    StepStatus::Done,
                    StepEnd {
                        summary: format!("{}: done", tool.tool),
                        output: Some(result),
                        ..base
                    },
                )
                .await;
                Ok(record)
            }
            Err(e) => {
                self.end_step(
                    &h,
                    StepStatus::Failed,
                    StepEnd {
                        summary: e.message.clone(),
                        error: Some(json!(e)),
                        ..base
                    },
                )
                .await;
                Err(self.tool_failure(&tool.tool, &e))
            }
        }
    }

    // ---- 5. check and hand to people ------------------------------------------------------

    fn label(&self, tool: &str) -> String {
        self.tool(tool)
            .map_or_else(|| tool.to_owned(), |t| t.summary.clone())
    }

    /// The set as it stands: commerce's view when this caller can read it, else the previews.
    async fn change_set_view(&mut self) -> Option<ChangeSetView> {
        let d = self.draft.clone()?;
        if self.tool("get_change_set").is_some() {
            self.tool_calls += 1;
            if let Ok(v) = self
                .tools
                .read("get_change_set", &json!({"change_set_id": d.id}))
                .await
            {
                let label = |t: &str| self.label(t);
                if let Some(view) = ChangeSetView::from_read(&v, &label) {
                    return Some(view);
                }
            }
        }
        let mut view = ChangeSetView {
            id: d.id.clone(),
            number: d.number,
            version: d.version,
            state: d.state.clone(),
            request: vec![self.text.clone()],
            interpretation: self
                .plan
                .as_ref()
                .map(|p| p.interpretation.clone())
                .unwrap_or_default(),
            items: d
                .items
                .iter()
                .enumerate()
                .map(|(n, i)| {
                    let (changes, changes_total, checks) = match &i.preview {
                        Some(p) => (p.changes.clone(), p.changes.len(), p.checks.clone()),
                        // Not previewed this turn: what the last block showed for it.
                        None => self.last_seen(&i.preview_id),
                    };
                    ItemView {
                        position: n + 1,
                        tool: i.tool.clone(),
                        label: self.label(&i.tool),
                        preview_id: i.preview_id.clone(),
                        changes,
                        changes_total,
                        checks,
                    }
                })
                .collect(),
            left_out: Vec::new(),
            undo_until: None,
        };
        view.left_out_from_items();
        Some(view)
    }

    /// An item's changes and checks as the conversation's last `change_set` block showed them.
    fn last_seen(&self, preview_id: &str) -> (Vec<Change>, usize, Vec<Check>) {
        let Some(last) = self.ctx.change_set.as_ref().map(|c| &c.last_view) else {
            return (Vec::new(), 0, Vec::new());
        };
        let group = last["groups"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|g| g["preview_id"] == preview_id);
        let Some(group) = group else {
            return (Vec::new(), 0, Vec::new());
        };
        let tool = group["op"].as_str().unwrap_or_default();
        let changes: Vec<Change> = group["rows"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| serde_json::from_value(r.clone()).ok())
            .collect();
        let total = changes.len() + group["more"].as_u64().unwrap_or(0) as usize;
        let checks: Vec<Check> = last["checks"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["tool"] == tool)
            .filter_map(|c| serde_json::from_value(c.clone()).ok())
            .collect();
        (changes, total, checks)
    }

    /// Hands the set to people when every rule check passed and the previews match the
    /// request. A set with failed checks stays a draft: the person decides what to change.
    async fn finish_change_set(&mut self) {
        let Some(d) = self.draft.clone() else {
            return;
        };
        if !d.touched {
            return;
        }
        let Some(mut view) = self.change_set_view().await else {
            return;
        };
        let failed = view.failed_checks().len();
        let h = self
            .begin_step(
                StepKind::Check,
                "Checking the change set against the request and the rules".into(),
                None,
            )
            .await;
        let summary = if !d.mismatches.is_empty() {
            format!("Still differs from the request: {}", d.mismatches.join(" "))
        } else if failed > 0 {
            format!("{failed} rule check(s) need a decision")
        } else {
            format!(
                "All {} rule checks passed; the previews match the request",
                view.check_count()
            )
        };
        let status = if d.mismatches.is_empty() && failed == 0 {
            StepStatus::Done
        } else {
            StepStatus::Failed
        };
        self.end_step(
            &h,
            status,
            StepEnd {
                summary,
                change_set_id: Some(d.id.clone()),
                ..StepEnd::default()
            },
        )
        .await;
        if status == StepStatus::Done && view.state == "drafting" && !view.items.is_empty() {
            match self.tools.submit(&view.id, view.version).await {
                Ok(head) => {
                    view.state.clone_from(&head.state);
                    view.version = head.version;
                    self.emit.send(Event::ChangeSetUpdated {
                        change_set_id: head.id,
                        version: head.version,
                        state: head.state,
                    });
                }
                Err(e) => self.problems.push(format!(
                    "The change set could not be handed over for approval: {}",
                    e.message
                )),
            }
        }
        self.view = Some(view);
    }

    // ---- 6. answer -----------------------------------------------------------------------

    fn last_output(&self, fits: impl Fn(&StepRecord) -> bool) -> Option<&StepRecord> {
        self.history
            .iter()
            .rev()
            .find(|r| r.output.is_some() && fits(r))
    }

    /// Facts the profile shows about the shopper, from any read's `key`s (`compares_specs`) and
    /// remembered values as `key=value` (`layout_preference=photos over details`), for
    /// `layout_rules`.
    fn profile_facts(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut add = |f: &Value| {
            if let Some(k) = f.get("key").and_then(Value::as_str) {
                out.push(k.to_owned());
                if let Some(v) = f.get("value").and_then(Value::as_str) {
                    out.push(format!("{k}={v}"));
                }
            }
        };
        for r in &self.history {
            let output = r.output.as_ref().unwrap_or(&Value::Null);
            for i in shop::items_of(output) {
                add(&i);
            }
            for key in ["facts", "preferences"] {
                for f in output
                    .get(key)
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    add(f);
                }
            }
        }
        out
    }

    /// The words that choose the layout: this message when it names one, else the latest
    /// earlier message of the conversation that did ("less text" keeps holding).
    fn layout_words(&self) -> String {
        let names = |t: &str| {
            let t = t.to_lowercase();
            self.profile
                .layout_rules
                .iter()
                .any(|r| r.when_said.iter().any(|p| t.contains(&p.to_lowercase())))
        };
        if names(&self.text) {
            return self.text.clone();
        }
        self.ctx
            .said_before()
            .find(|t| names(t))
            .map_or_else(|| self.text.clone(), str::to_owned)
    }

    /// Every tool result this turn saw, earlier blocks it used included, as sources a number
    /// in the model's wording must come from.
    fn number_sources(&self) -> Vec<Value> {
        let mut out: Vec<Value> = self
            .history
            .iter()
            .chain(&self.sources)
            .filter_map(|r| r.output.clone())
            .collect();
        out.extend(
            self.history
                .iter()
                .filter_map(|r| r.preview.as_ref())
                .filter_map(|p| serde_json::to_value(p).ok()),
        );
        out
    }

    /// Whether model wording uses only numbers from the tools or from what the person said.
    fn supported(&self, text: &str, sources: &[Value]) -> bool {
        let refs: Vec<&Value> = sources.iter().collect();
        let mut said: Vec<&str> = vec![self.text.as_str()];
        said.extend(self.ctx.said_before());
        checker::unsupported_numbers(text, &refs, &said).is_empty()
    }

    /// The plan's notes the block may show: `(element, text)`, numbers checked.
    fn checked_notes(&self) -> Vec<(String, String)> {
        let sources = self.number_sources();
        self.plan
            .as_ref()
            .map(|p| {
                p.notes
                    .iter()
                    .filter(|n| self.supported(&n.text, &sources))
                    .map(|n| (n.element.clone(), n.text.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The "looking for" chips: the plan's understood lines, numbers checked.
    fn looking_for_block(&self) -> Option<Block> {
        let sources = self.number_sources();
        let chips: Vec<(String, String)> = self
            .plan
            .as_ref()?
            .interpretation
            .iter()
            .filter(|u| self.supported(&u.text, &sources))
            .map(|u| {
                (
                    u.text.clone(),
                    u.source.clone().unwrap_or_else(|| "said".into()),
                )
            })
            .collect();
        shopper::looking_for(&chips)
    }

    fn fit_check_block(&self) -> Option<Block> {
        let reads: Vec<shopper::Read<'_>> = self
            .history
            .iter()
            .filter_map(|r| {
                r.output.as_ref().map(|o| shopper::Read {
                    tool: &r.tool,
                    output: o,
                })
            })
            .collect();
        let remembered: Vec<String> = self
            .blocks
            .iter()
            .filter(|b| b.ui == "memory_note")
            .filter_map(|b| b.data["text"].as_str().map(str::to_owned))
            .collect();
        shopper::fit_check(&reads, &self.checked_notes(), &remembered)
    }

    /// Products this turn read, else the rows of an earlier block of one of `uis` it used,
    /// else the latest such block in the conversation.
    fn product_rows(&self, uis: &[&str]) -> Vec<Value> {
        if let Some(r) = self.last_output(|r| {
            r.kind == ToolKind::Read
                && shop::items_of(r.output.as_ref().unwrap_or(&Value::Null))
                    .first()
                    .is_some_and(|i| i.get("name").is_some())
        }) {
            return shop::items_of(r.output.as_ref().unwrap_or(&Value::Null));
        }
        if let Some(s) = self
            .sources
            .iter()
            .rev()
            .find(|s| uis.contains(&s.tool.as_str()))
        {
            return shop::items_of(s.output.as_ref().unwrap_or(&Value::Null));
        }
        self.ctx
            .last_block(uis)
            .map(|b| shop::results_of(b.get("data").unwrap_or(&Value::Null)))
            .map(|o| shop::items_of(&o))
            .unwrap_or_default()
    }

    fn gift_picker_block(&self, review_target: Option<&str>) -> Option<Block> {
        let gift = self
            .history
            .iter()
            .rev()
            .filter(|r| self.tool(&r.tool).is_some_and(ToolSpec::preview_only))
            .filter_map(|r| r.preview.as_ref())
            .find(|p| p.changes.iter().any(|c| c.to["order_type"] == "gift"));
        let cart = self
            .last_output(|r| r.output.as_ref().is_some_and(|o| o.get("lines").is_some()))
            .and_then(|r| r.output.as_ref());
        let picks = self.product_rows(&["gift_picker", "look_gallery", "product_list"]);
        shopper::gift_picker(&picks, cart, gift, review_target)
    }

    /// The batch of return requests: this turn's queue joined with its evaluations, else the
    /// review shown earlier; with the change set when this turn drafted one.
    fn refund_review_block(&self) -> Option<Block> {
        let reads = || {
            self.history
                .iter()
                .filter(|r| r.kind == ToolKind::Read)
                .filter_map(|r| r.output.as_ref())
                .flat_map(executor::rows)
        };
        let queue: Vec<Value> = reads()
            .filter(|r| r.get("claim_id").is_some() && r.get("suggested").is_none())
            .collect();
        let evaluations: Vec<Value> = reads()
            .filter(|r| r.get("claim_id").is_some() && r.get("suggested").is_some())
            .collect();
        let mut rows = review::requests(&queue, &evaluations);
        if rows.is_empty() {
            rows = self
                .sources
                .iter()
                .rev()
                .find(|s| s.tool == "refund_review")
                .and_then(|s| s.output.as_ref())
                .map(executor::rows)
                .or_else(|| {
                    self.ctx
                        .last_block(&["refund_review"])
                        .and_then(|b| b.get("data"))
                        .map(review::rows_of)
                })
                .unwrap_or_default();
        }
        review::refund_review(&rows, self.view.as_ref(), &self.profile.check_intents)
    }

    fn build_blocks(&mut self) {
        let reply_plan = self
            .plan
            .as_ref()
            .and_then(|p| p.reply_plan.clone())
            .unwrap_or_default();
        let mut out: Vec<Block> = Vec::new();
        // A supplier sheet's drafts: the drafts block carries the change set (its save button
        // approves it), so the change set block is not shown twice.
        let drafts = (reply_plan == "product_drafts")
            .then(|| {
                self.history
                    .iter()
                    .rev()
                    .find(|r| r.tool == "batch_import")
                    .and_then(|r| {
                        crate::blocks::drafts::product_drafts(
                            r.preview.as_ref()?,
                            self.view.as_ref(),
                        )
                    })
            })
            .flatten();
        // A batch review carries its change set itself, approve button included.
        if let Some(view) = self
            .view
            .as_ref()
            .filter(|_| drafts.is_none() && reply_plan != "refund_review")
        {
            let title = self.plan.as_ref().and_then(|p| p.title.clone());
            let b = crate::blocks::change_set::block(
                view,
                &self.profile.check_intents,
                title.as_deref(),
            );
            out.push(b);
        }
        let mut cards: Vec<Block> = Vec::new();
        for r in &self.history {
            if let Some(p) = &r.preview
                && self.tool(&r.tool).is_some_and(ToolSpec::preview_only)
            {
                cards.push(shop::checkout_card(p));
            }
        }
        let allowed = |ui: &str| self.profile.blocks.iter().any(|b| b == ui);
        let block = match reply_plan.as_str() {
            "product_drafts" => drafts,
            "product_table" => self
                .last_output(|r| r.kind == ToolKind::Read)
                .and_then(|r| table::product_table(r.output.as_ref()?)),
            "action_log" => self
                .last_output(|r| r.kind == ToolKind::Read)
                .and_then(|r| table::action_log(r.output.as_ref()?)),
            "task_list" => self
                .last_output(|r| r.tool == "list_tasks")
                .and_then(|r| crate::blocks::tasks::task_list(r.output.as_ref()?)),
            "order_detail" => self
                .last_output(|_| true)
                .and_then(|r| table::order_detail(r.output.as_ref()?)),
            "cart" => self
                .last_output(|r| r.output.as_ref().is_some_and(|o| o.get("lines").is_some()))
                .and_then(|r| shop::cart(r.output.as_ref()?)),
            "sales_explainer" => self
                .last_output(|r| r.tool == "explain_sales_change")
                .and_then(|r| {
                    let trend = self
                        .last_output(|t| t.tool == "get_sales_trend")
                        .and_then(|t| t.output.as_ref());
                    insights::sales_explainer(r.output.as_ref()?, trend, &self.profile.next_actions)
                }),
            "return_options" => self
                .last_output(|r| r.kind == ToolKind::Read)
                .and_then(|r| shop::return_options(r.output.as_ref()?, &r.input)),
            "compare_table" | "look_gallery" | "product_list" => {
                let facts = self.profile_facts();
                // A first visit: a guest with no history yet. A signed-in shopper has a record.
                let first_visit = self.principal.customer_id().is_none()
                    && self.ctx.is_first_turn()
                    && facts.is_empty();
                let said = self.layout_words();
                let rule = shop::layout(&self.profile.layout_rules, &said, &facts, first_visit);
                let ui = rule
                    .map(|r| r.ui.as_str())
                    .filter(|u| allowed(u))
                    .unwrap_or(reply_plan.as_str());
                // Nothing read this turn: the results shown last, laid out again.
                let rows = self.product_rows(&["compare_table", "look_gallery", "product_list"]);
                shop::products(
                    ui,
                    &json!({"items": rows, "total": rows.len()}),
                    rule.map(|r| r.why.as_str()),
                )
            }
            "fit_check" => self.fit_check_block(),
            "looking_for" => self.looking_for_block(),
            "gift_picker" => {
                let target = cards.last().map(|c| format!("block:{}", c.block_id));
                out.extend(self.looking_for_block());
                self.gift_picker_block(target.as_deref())
            }
            "profile_card" => self
                .last_output(|r| {
                    r.output
                        .as_ref()
                        .is_some_and(|o| o.get("preferences").is_some())
                })
                .and_then(|r| shopper::profile_card(r.output.as_ref()?)),
            "refund_review" => self.refund_review_block(),
            _ => None,
        };
        // A gift is reviewed in its picker first; the card with the Place order button follows.
        if reply_plan == "gift_picker" {
            out.extend(block);
            out.extend(cards);
        } else {
            out.extend(cards);
            out.extend(block);
        }
        for b in out {
            self.emit.send(Event::Block(Box::new(b.clone())));
            if matches!(b.ui.as_str(), "change_set" | "product_drafts")
                && let Some(failed) = b.data["checks"].as_array().and_then(|c| {
                    c.iter()
                        .find(|c| c["passed"] == false || c["status"] == "open")
                })
                && let Some(element) = failed["element"].as_str()
            {
                let note = failed["detail"].as_str().map(str::to_owned);
                self.emit
                    .command(Command::Highlight, &b.element_ref(element), note);
            }
            self.blocks.push(b);
        }
    }

    /// What the reply is written from; every number the reply may use is in here or in the
    /// tool results.
    fn facts(&self) -> Value {
        let cs = self.view.as_ref().map(|v| {
            json!({"change_set": v.number.map_or_else(|| v.id.clone(), |n| format!("#{n}")),
                   "version": v.version, "state": v.state, "items": v.items.len(),
                   "checks": v.check_count(),
                   "failed_checks": v.failed_checks().iter().map(|c| json!({"rule": c.rule, "detail": c.detail})).collect::<Vec<_>>(),
                   "left_out": v.left_out.len()})
        });
        let reads: Vec<Value> = self
            .history
            .iter()
            .filter(|r| r.kind == ToolKind::Read)
            .map(|r| json!({"tool": r.tool, "result": compact(r.output.as_ref().unwrap_or(&Value::Null))}))
            .collect();
        // A block's summary (a review's counts and refund total) is what the reply may cite.
        let blocks: Vec<Value> = self
            .blocks
            .iter()
            .map(|b| {
                let mut v = json!({"ui": b.ui, "title": b.title});
                if let Some(s) = b.data.get("summary").filter(|s| s.is_object()) {
                    v["summary"] = s.clone();
                }
                v
            })
            .collect();
        json!({"request": self.text, "change_set": cs, "reads": reads, "blocks_shown": blocks,
               "problems": self.problems,
               "interpretation": self.plan.as_ref().map(|p| p.interpretation.iter().map(|u| u.text.clone()).collect::<Vec<_>>())})
    }

    fn fallback_text(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if let Some(v) = &self.view {
            let name = v.number.map_or_else(|| v.id.clone(), |n| format!("#{n}"));
            parts.push(format!(
                "I drafted change set {name} (version {}) with {} item{}.",
                v.version,
                v.items.len(),
                if v.items.len() == 1 { "" } else { "s" }
            ));
            let failed = v.failed_checks();
            if !failed.is_empty() {
                let rules: Vec<String> = failed
                    .iter()
                    .map(|c| crate::blocks::words(&c.rule))
                    .collect();
                parts.push(format!(
                    "These checks need your decision: {}.",
                    rules.join(", ")
                ));
            } else if v.state == "waiting_for_approval" {
                parts.push("It is waiting for your approval.".into());
            }
        }
        for b in &self.blocks {
            match b.ui.as_str() {
                "checkout_card" => {
                    parts.push("Your order is ready to review; only you can place it.".into())
                }
                "compare_table" | "look_gallery" | "product_list" => {
                    parts.push("Here is what I found.".into())
                }
                "cart" => parts.push("Here is your cart.".into()),
                "sales_explainer" => parts.push(
                    "Here is how sales compare and what the data shows behind the change.".into(),
                ),
                "fit_check" => parts.push("Here is how the sizes compare.".into()),
                "gift_picker" => parts.push(
                    "Here are a few picks; nothing about the person it is for is saved.".into(),
                ),
                "refund_review" if self.view.is_none() => {
                    parts.push("Here is how the store's policy sorts these requests.".into())
                }
                "profile_card" => parts.push("Here is what we know about you.".into()),
                "return_options" => {
                    parts.push("Here are your options; only you can send the request.".into())
                }
                "product_drafts" => parts.push(
                    "I turned the sheet into drafts. Shoppers do not see them; publishing is a separate change set."
                        .into(),
                ),
                _ => {}
            }
        }
        parts.extend(self.problems.iter().cloned());
        if parts.is_empty() {
            parts.push("I could not find anything to show for that.".into());
        }
        parts.join(" ")
    }

    async fn reply(&mut self) -> String {
        let fallback = self.fallback_text();
        if !self.problems.is_empty() && self.blocks.is_empty() {
            return fallback;
        }
        let facts = self.facts();
        let prompt = format!(
            "{}\n\n{}",
            self.rt.profiles.prompts.reply.trim(),
            self.profile.instructions_text.trim()
        );
        let req = ChatRequest::new(
            "reply",
            vec![
                Message::system(prompt),
                Message::user(format!("Results of this turn: {facts}")),
            ],
        )
        .max_tokens(200);
        let worker = self.worker().to_owned();
        let Some(text) = self.model_text("reply", &[&worker], req).await else {
            return fallback;
        };
        let outputs: Vec<&Value> = self
            .history
            .iter()
            .filter_map(|r| r.output.as_ref())
            .chain(std::iter::once(&facts))
            .collect();
        let previews: Vec<Value> = self
            .history
            .iter()
            .filter_map(|r| r.preview.as_ref())
            .filter_map(|p| serde_json::to_value(p).ok())
            .collect();
        let mut sources = outputs;
        sources.extend(previews.iter());
        let unsupported = checker::unsupported_numbers(&text, &sources, &[&self.text]);
        if unsupported.is_empty() {
            text
        } else {
            tracing::info!(numbers = ?unsupported, "reply used numbers not in the results; using the plain reply");
            if let Some(last) = self.usage.last_mut() {
                last.succeeded = false;
                last.error = Some(format!(
                    "numbers not in the results: {}",
                    unsupported.join(", ")
                ));
            }
            fallback
        }
    }

    // ---- 7. end --------------------------------------------------------------------------

    async fn fail(self, code: &str, message: String) {
        self.emit.send(Event::Error {
            code: code.to_owned(),
            message: message.clone(),
        });
        self.finish(None, Some(json!({"code": code, "message": message})))
            .await;
    }

    async fn finish(self, reply: Option<String>, error: Option<Value>) {
        if let Some(text) = &reply {
            for piece in sentences(text) {
                self.emit.send(Event::MessageDelta { text: piece });
            }
            self.emit.send(Event::MessageDone {
                message_id: format!("msg_{}", acom_core::id::new_suffix()),
                text: text.clone(),
            });
        }
        let usage = self.totals();
        let mut models: Vec<String> = Vec::new();
        for r in &self.usage {
            let name = self
                .rt
                .models
                .get(&r.alias)
                .map_or_else(|| r.model.clone(), |m| m.info().display_name.clone());
            if !models.contains(&name) {
                models.push(name);
            }
        }
        if self.logged {
            let end = TurnEnd {
                tenant: self.tenant.clone(),
                turn_id: self.turn_id.clone(),
                agent: self.profile.id.clone(),
                reply_text: reply,
                blocks: self
                    .blocks
                    .iter()
                    .filter_map(|b| serde_json::to_value(b).ok())
                    .collect(),
                usage: serde_json::to_value(&usage).unwrap_or(Value::Null),
                error,
                ended_at: self.clock.now(),
            };
            if let Err(e) = self.rt.log.finish_turn(&end).await {
                tracing::warn!(error = %e, "turn end not recorded");
            }
        }
        tracing::info!(turn = %self.turn_id, request_id = %self.request_id, agent = %self.profile.id,
            subject = %self.principal.subject, model_calls = usage.model_calls,
            tool_calls = usage.tool_calls, ms = usage.ms, "turn done");
        self.emit.send(Event::TurnDone {
            turn_id: self.turn_id.clone(),
            usage,
            models,
        });
    }

    fn totals(&self) -> TurnUsage {
        let priced = self
            .usage
            .iter()
            .all(|r| r.cost.is_some() && r.cost_currency.as_deref() == Some("USD"));
        let cost = priced.then(|| {
            self.usage
                .iter()
                .filter_map(|r| r.cost)
                .sum::<Decimal>()
                .normalize()
        });
        TurnUsage {
            model_calls: self.usage.len() as u32,
            tool_calls: self.tool_calls,
            input_tokens: self.usage.iter().map(|r| u64::from(r.input_tokens)).sum(),
            output_tokens: self.usage.iter().map(|r| u64::from(r.output_tokens)).sum(),
            ms: self.elapsed().as_millis() as u64,
            cost_usd: cost.and_then(|c| c.to_string().parse().ok()),
        }
    }
}

/// A result cut down for prompts: lists keep their first items and say how many there were.
fn compact(v: &Value) -> Value {
    match v {
        Value::Object(o) => {
            let mut out = serde_json::Map::new();
            for (k, x) in o {
                out.insert(k.clone(), compact(x));
            }
            Value::Object(out)
        }
        Value::Array(a) if a.len() > 5 => {
            let mut kept: Vec<Value> = a.iter().take(5).map(compact).collect();
            kept.push(json!(format!("… {} more", a.len() - 5)));
            Value::Array(kept)
        }
        Value::Array(a) => Value::Array(a.iter().map(compact).collect()),
        Value::String(s) if s.chars().count() > 200 => {
            json!(format!("{}…", s.chars().take(200).collect::<String>()))
        }
        other => other.clone(),
    }
}

/// The reply in sentence-sized pieces for `message.delta`.
fn sentences(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    for c in text.chars() {
        current.push(c);
        if matches!(c, '.' | '!' | '?' | '\n') && current.trim().len() > 1 {
            out.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        out.push(current);
    }
    out
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod scenes;
