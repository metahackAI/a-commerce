//! The turn loop (docs/backend/agent.md §1): route → search tools → plan → run (reads, previews,
//! change sets) → check → answer with blocks and a short text, within the turn's limits.
//!
//! Writes are never run past a person. On the back office every write is previewed into one
//! change set, which is handed to people (`submit`) only when its rule checks pass; the runtime
//! never approves. On the storefront a write that needs no approval is previewed and committed
//! with its `preview_id`, and an order is only prepared: the shopper's button places it.
//!
//! For the evaluation (docs/design.md §7) a turn may use the screen-shaped facade instead
//! (`tool_set: facade`, demo mode only): the same routing, tool search, planning and execution,
//! over the facade's manifest, where every call runs at once (reads and direct writes alike).
//! The turn's stored usage records which tool set it used.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use acom_auth::Principal;
use acom_contract::{Change, Check, ErrorCode, Preview, ToolError};
use acom_core::Clock;
use acom_core::ids::{ConversationId, TurnId};
use acom_llm::{
    ChatModel, ChatRequest, LlmError, Message, ModelInfo, ModelSet, RepairEnv, Repaired,
    StreamVariant, UsageRecord,
};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::blocks::change_set::{ChangeSetView, ItemView};
use crate::blocks::{Action, Block, insights, records, review, shop, shopper, table};
use crate::checker;
use crate::client::{ItemRef, NewChangeSet, ToolClient};
use crate::executor::{self, StepRecord};
use crate::memory::{ConversationContext, OpenChangeSet, Owner};
use crate::planner::{self, About, Plan, PlanInput, PlanStep};
use crate::profile::{AgentProfile, ChatSurface, LayoutRule, Limits, Profiles};
use crate::reply_stream::ReplyStream;
use crate::router;
use crate::store::{StepRow, StoreInfo, StoreSettings, SystemStore, TurnEnd, TurnLog, TurnStart};
use crate::stream::{Command, Emitter, Event, StepKind, StepStatus, TurnUsage};
use crate::tools::{ToolKind, ToolSet, ToolSpec};
use crate::toolsearch;

/// The reply up to where it starts copying its instructions; nothing when that is all it was.
fn without_echo(text: String, prompt: &str) -> Option<String> {
    match checker::echo_start(&text, prompt) {
        None => Some(text),
        Some(at) => {
            tracing::info!("reply copied its instructions; cut there");
            let kept = text[..at].trim();
            (!kept.is_empty()).then(|| kept.to_owned())
        }
    }
}

/// The longest reply text the schema takes. A reply that reaches it was stopped there.
const REPLY_MAX_CHARS: usize = 600;

/// The reply's shape: two or three sentences in one field.
fn reply_schema() -> Value {
    json!({"type": "object", "additionalProperties": false, "required": ["text"],
           "properties": {"text": {"type": "string", "maxLength": REPLY_MAX_CHARS,
                                   "description": "Two or three plain sentences for the person."}}})
}

/// Whether `variant` streams an answer from `model`: a request shaped like a real reply (the
/// reply prompt and a turn's results), which must come back in pieces that already read as
/// text, and whole at the end.
async fn probe_once(model: &dyn acom_llm::ChatModel, variant: StreamVariant, prompt: &str) -> bool {
    let raw = std::sync::Mutex::new(String::new());
    let sink = |d: &str| {
        if let Ok(mut r) = raw.lock() {
            r.push_str(d);
        }
    };
    let results = json!({"reads": [{"tool": "search_products", "result": {"items": [
        {"id": "prd_windbreaker", "name": "Packable windbreaker", "price": 89.0, "price_currency": "USD", "availability": "in_stock"},
        {"id": "prd_field_jacket", "name": "Linen blend field jacket", "price": 128.0, "price_currency": "USD", "availability": "in_stock"}],
        "total": 2}}], "request": "I need a light jacket for spring trips."});
    let req = ChatRequest::new(
        "stream_check",
        vec![
            Message::system(prompt.trim().to_owned()),
            Message::user(format!("Results of this turn: {results}")),
        ],
    )
    .json("reply", reply_schema())
    .max_tokens(500)
    .stream_variant(variant);
    let res = tokio::time::timeout(Duration::from_secs(45), model.chat(&req, Some(&sink))).await;
    let streamed = raw
        .lock()
        .map(|r| crate::reply_stream::visible_text(&r))
        .unwrap_or_default();
    matches!(res, Ok(Ok(r)) if !r.text.trim().is_empty()) && !streamed.trim().is_empty()
}

/// A reply the model wrote, and whether a length limit stopped it before it finished.
struct ReplyDraft {
    text: String,
    cut: bool,
}

impl ReplyDraft {
    /// The text to show: all of it, or up to its last finished sentence when it was cut.
    fn finished(self) -> Option<String> {
        if self.cut {
            checker::whole_sentences(&self.text)
        } else {
            Some(self.text)
        }
    }
}

/// What a turn says when no model answers at all: the pages still work without the assistant.
const MODEL_DOWN: &str = "The assistant's model is not answering right now, so I can't do this yet. Everything on the pages still works; please try again in a few minutes.";

/// What the reply model is told when its reply was cut at the length limit.
const CUT_REPLY: &str = "That was cut off at the length limit. Write it again in two or three \
short sentences; the blocks on screen show the details.";

/// What the reply model is told when its reply named tools or result fields.
const SYSTEM_WORDS: &str = "That names tools or result fields the person never sees";

/// What the reply model is told when its reply repeated the request.
const REPEATED_REQUEST: &str = "That repeats the request and describes the results instead of \
answering. Write only the answer to the person, in two or three sentences.";

/// What the planner is told when its plan has no steps.
const EMPTY_PLAN: &str = "The plan has no steps, so the answer would come from nothing. \
The person asked about the store: plan at least one read from the tools listed (search, list or get) \
that answers it, and pick the block that shows the result. Only a greeting or thanks needs no step.";

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
    /// The tools this turn uses: A-Commerce's (default) or, for the evaluation, the
    /// screen-shaped facade. The agent service accepts `facade` only in demo mode.
    #[serde(default, skip_serializing_if = "ToolSet::is_commerce")]
    pub tool_set: ToolSet,
    /// The task this turn works on (`task_…`, from "Ask the agent to work on it" or a watcher):
    /// a change set the turn drafts is linked to it (docs/api/chat.md §1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
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
        if let Some(id) = &self.task_id {
            let ok = id.strip_prefix("task_").is_some_and(|rest| {
                !rest.is_empty()
                    && rest
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
            });
            if !ok {
                return Err(
                    ToolError::invalid_input("task_id: not a task id (`task_…`).")
                        .detail(json!({"field": "task_id", "reason": "format"})),
                );
            }
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
    /// For each model (alias), whether its replies stream and how ([`StreamState`]). Found out
    /// in the background ([`Runtime::keep_streaming_probed`]), never in a person's turn.
    streaming: std::sync::Mutex<BTreeMap<String, StreamState>>,
    /// Wakes the background check when a way that worked stops working in a turn.
    reprobe: tokio::sync::Notify,
    /// Ways that came back empty in a person's turn, per model, and when: a real reply outweighs
    /// the check, so the check does not trust them again for [`TURN_FAILURE_HOLDS`].
    failed_in_turn: std::sync::Mutex<BTreeMap<String, Vec<(StreamVariant, Instant)>>>,
}

/// What the runtime knows about streaming one model's reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize)]
#[serde(tag = "state", content = "way", rename_all = "snake_case")]
pub enum StreamState {
    /// Not checked yet (or a way stopped working): replies come whole until it is.
    #[default]
    Unknown,
    /// Replies stream this way: the background check saw an answer arrive in pieces.
    Works(StreamVariant),
    /// No way gave a streamed answer: replies come whole, and it is checked again later.
    Off,
}

/// How often the background check looks again at a model whose replies do not stream (a server
/// that gets fixed starts streaming without a restart), and at one whose do.
const PROBE_AGAIN_OFF: Duration = Duration::from_secs(10 * 60);
const PROBE_AGAIN_WORKS: Duration = Duration::from_secs(60 * 60);
/// How long a way that failed in a person's turn is left out of the check.
const TURN_FAILURE_HOLDS: Duration = Duration::from_secs(6 * 60 * 60);

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
            streaming: std::sync::Mutex::new(BTreeMap::new()),
            reprobe: tokio::sync::Notify::new(),
            failed_in_turn: std::sync::Mutex::new(BTreeMap::new()),
        }
    }

    /// The way to stream a reply from this model now: only one the background check saw work.
    fn stream_variant(&self, model: &ModelInfo) -> Option<StreamVariant> {
        match self.stream_state(&model.alias) {
            StreamState::Works(v) => Some(v),
            StreamState::Unknown | StreamState::Off => None,
        }
    }

    /// A way that worked came back without text in a turn: replies come whole again until the
    /// background check, woken now, has looked again.
    fn stream_failed(&self, model: &ModelInfo, variant: StreamVariant) {
        if let Ok(mut f) = self.failed_in_turn.lock() {
            f.entry(model.alias.clone())
                .or_default()
                .push((variant, Instant::now()));
        }
        if let Ok(mut s) = self.streaming.lock()
            && s.get(&model.alias) == Some(&StreamState::Works(variant))
        {
            s.insert(model.alias.clone(), StreamState::Unknown);
            self.reprobe.notify_one();
        }
    }

    /// Whether `variant` failed in a person's turn lately (the check leaves it out).
    fn failed_lately(&self, alias: &str, variant: StreamVariant) -> bool {
        self.failed_in_turn.lock().is_ok_and(|f| {
            f.get(alias).is_some_and(|ways| {
                ways.iter()
                    .any(|(v, at)| *v == variant && at.elapsed() < TURN_FAILURE_HOLDS)
            })
        })
    }

    pub fn stream_state(&self, alias: &str) -> StreamState {
        self.streaming.lock().map_or(StreamState::Unknown, |s| {
            s.get(alias).copied().unwrap_or_default()
        })
    }

    /// Every configured model and whether its replies stream (`GET /chat/{surface}/streaming`).
    pub fn stream_states(&self) -> Vec<(String, StreamState)> {
        self.models
            .models
            .keys()
            .map(|alias| (alias.clone(), self.stream_state(alias)))
            .collect()
    }

    #[cfg(test)]
    pub(crate) fn assume_streaming(&self, alias: &str, state: StreamState) {
        if let Ok(mut s) = self.streaming.lock() {
            s.insert(alias.to_owned(), state);
        }
    }

    /// One look at every model whose replies do not stream yet: each of its ways
    /// (`stream_variants`) in turn, with a short request shaped like a reply, until one answers
    /// in pieces. Returns how long to wait before the next look.
    pub async fn probe_streaming(&self) -> Duration {
        let mut wait = PROBE_AGAIN_WORKS;
        for (alias, model) in &self.models.models {
            if matches!(self.stream_state(alias), StreamState::Works(_)) {
                continue;
            }
            let mut found = StreamState::Off;
            let prompt = self.profiles.prompts.reply.clone();
            for &v in &model.info().stream_variants {
                if self.failed_lately(alias, v) {
                    continue;
                }
                if probe_once(model.as_ref(), v, &prompt).await {
                    found = StreamState::Works(v);
                    break;
                }
            }
            tracing::info!(model = %alias, state = ?found, "reply streaming checked");
            if found == StreamState::Off {
                wait = PROBE_AGAIN_OFF;
            }
            if let Ok(mut s) = self.streaming.lock() {
                s.insert(alias.clone(), found);
            }
        }
        wait
    }

    /// Checks streaming at start, again on a schedule, and at once when a way stops working.
    pub async fn keep_streaming_probed(self: Arc<Self>) {
        loop {
            let wait = self.probe_streaming().await;
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                () = self.reprobe.notified() => {}
            }
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
        with_log: bool,
    ) -> Result<Value, ToolError> {
        let tenant = principal.tenant.as_str();
        match self.log.history(tenant, conversation_id).await {
            Ok(Some((owner, mut turns))) if owner.same_person(&Owner::of(principal)) => {
                // The turn log: what each step read and wrote, and every model call with its
                // error (an unusable plan, a reply that cited numbers no tool returned).
                if with_log {
                    let logs = self
                        .log
                        .turn_logs(tenant, conversation_id)
                        .await
                        .unwrap_or_default();
                    for t in &mut turns {
                        let id = t["turn_id"].as_str().unwrap_or_default().to_owned();
                        if let Some((_, log)) = logs.iter().find(|(turn, _)| *turn == id) {
                            t["log"] = log.clone();
                        }
                    }
                }
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
    tool_set: ToolSet,
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
    /// Fields of the step about to run that binding could not choose a value for.
    undecided: Vec<String>,
    /// The tools of the plan's steps from the one that failed on, for the next plan.
    unrun: Vec<String>,
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
    /// Steps not called because what they build on came back empty: the tool and why.
    skipped: Vec<(String, String)>,
    /// The reply as the model writes it (`message.delta` while it is written).
    reply_stream: Arc<ReplyStream>,
    /// Whether the person saw a reply stream in this turn (then `message.done` alone ends it).
    streamed: bool,
    /// Blocks on screen this turn, by place (`products#0`, `change_set#0`): the id they were
    /// sent with and what they showed, so a block that changes keeps its id and replaces itself.
    shown: Vec<ShownBlock>,
}

/// A block sent this turn: where it sits, its id, and what it showed (to skip sending it again).
#[derive(Debug, Clone)]
struct ShownBlock {
    place: String,
    block_id: String,
    sent: Value,
}

/// Where a block sits on the canvas: the product views are one place (the same results laid
/// out as a list, a gallery or a comparison), anything else by its kind; then its order.
/// Whether a listed item is a product (`prd_…` as its `product_id` or `id`).
fn is_product_item(item: &Value) -> bool {
    item.get("product_id")
        .or_else(|| item.get("id"))
        .and_then(Value::as_str)
        .is_some_and(|id| id.starts_with("prd_"))
}

fn place_of(ui: &str, nth: usize) -> String {
    let family = match ui {
        "compare_table" | "look_gallery" | "product_list" => "products",
        other => other,
    };
    format!("{family}#{nth}")
}

/// Points block references (`block:blk_…` in a block's data or an Open action's target) at the
/// ids blocks keep.
fn rename_block_ids(v: &mut Value, renamed: &[(String, String)]) {
    match v {
        Value::String(text) => {
            for (from, to) in renamed {
                if text.contains(from.as_str()) {
                    *text = text.replace(from.as_str(), to);
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|i| rename_block_ids(i, renamed)),
        Value::Object(map) => map.values_mut().for_each(|i| rename_block_ids(i, renamed)),
        _ => {}
    }
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
            tool_set: p.request.tool_set,
            principal: p.principal,
            tenant,
            surface: p.surface,
            request_id: p.request_id,
            request: p.request,
            ctx: p.context,
            turn_id: TurnId::generate().as_str().to_owned(),
            profile,
            reply_stream: Arc::new(ReplyStream::new(emit.clone())),
            streamed: false,
            shown: Vec::new(),
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
            undecided: Vec::new(),
            unrun: Vec::new(),
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
            skipped: Vec::new(),
        }
    }

    async fn go(mut self) {
        self.route().await;
        if self.rt.models.is_empty() {
            let message = self.rt.models.not_configured_message();
            return self.fail("upstream_unavailable", message).await;
        }
        let manifest = match self.tool_set {
            ToolSet::Commerce => self.tools.manifest().await,
            ToolSet::Facade => self.tools.facade_manifest().await,
        };
        self.manifest = match manifest {
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
                Err(Stop(why)) => self.note_problem(why),
            }
        }
        self.finish_change_set().await;
        self.compare_for_layout().await;
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
        // Whether every model tried was down (silent, unreachable, failing), not just wrong.
        let mut all_down = true;
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
            all_down &= res.as_ref().err().is_some_and(LlmError::is_down);
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
        if all_down && last != "no model answered" {
            tracing::warn!(purpose, error = %last, "every model is down");
            return Err(Stop(MODEL_DOWN.into()));
        }
        Err(Stop(format!("The model could not answer: {last}")))
    }

    async fn model_text(
        &mut self,
        purpose: &str,
        prefer: &[&str],
        req: ChatRequest,
    ) -> Option<(String, bool)> {
        for model in self.models_for(prefer) {
            if self.usage.len() as u32 >= self.limits.model_calls || self.time_left().is_zero() {
                return None;
            }
            // Each attempt (another model, or a check asking again) starts a fresh reply.
            self.reply_stream.reset("The reply is being written again.");
            let variant = self.rt.stream_variant(model.info());
            let left = self.time_left();
            let started_at = self.clock.now();
            let t = Instant::now();
            // The reply streams to the person as it is written, the way this model was last
            // seen to stream; a model that never gave a streamed answer is asked without one.
            let res = match variant {
                Some(v) => {
                    let stream = self.reply_stream.clone();
                    let sink = move |d: &str| stream.push(d);
                    let req = req.clone().stream_variant(v);
                    tokio::time::timeout(left, model.chat(&req, Some(&sink)))
                        .await
                        .unwrap_or(Err(LlmError::Timeout(left.as_secs())))
                }
                None => tokio::time::timeout(left, model.chat(&req, None))
                    .await
                    .unwrap_or(Err(LlmError::Timeout(left.as_secs()))),
            };
            if !self.reply_stream.shown().is_empty() {
                self.streamed = true;
            }
            let mut record = UsageRecord::new(
                model.info(),
                purpose,
                started_at,
                t.elapsed().as_millis() as u64,
                &res,
            );
            // A stream that came back without the answer (a provider that streams structured
            // output apart, or not at all) is asked once more without streaming, so the person
            // still gets the model's reply; that way of streaming is not tried again. Not when
            // the endpoint is down: that would only wait twice.
            let unstreamed = variant.is_some()
                && match &res {
                    Ok(r) => r.text.trim().is_empty(),
                    Err(e) => {
                        !e.is_down() && !matches!(e, LlmError::Timeout(_) | LlmError::Resting(_))
                    }
                };
            if let (true, Some(v)) = (unstreamed, variant) {
                self.rt.stream_failed(model.info(), v);
                record.succeeded = false;
                record.error = Some(format!(
                    "streamed as {}: {}",
                    v.as_str(),
                    record.error.as_deref().unwrap_or("no text")
                ));
            }
            self.record_call(record, None).await;
            let res = if unstreamed
                && (self.usage.len() as u32) < self.limits.model_calls
                && !self.time_left().is_zero()
            {
                self.reply_stream.reset("The reply is being written again.");
                let left = self.time_left();
                let started_at = self.clock.now();
                let t = Instant::now();
                let again = tokio::time::timeout(left, model.chat(&req, None))
                    .await
                    .unwrap_or(Err(LlmError::Timeout(left.as_secs())));
                let record = UsageRecord::new(
                    model.info(),
                    purpose,
                    started_at,
                    t.elapsed().as_millis() as u64,
                    &again,
                );
                self.record_call(record, None).await;
                again
            } else {
                res
            };
            if let Ok(r) = res
                && !r.text.trim().is_empty()
            {
                let stopped = r.finish_reason.as_deref() == Some("length");
                return Some((r.text.trim().to_owned(), stopped));
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
        // A conversation with an open change set stays with its agent: what is said next is
        // most likely a correction to it ("drop the low-stock ones").
        let continuing = previous
            .as_ref()
            .filter(|p| self.open_change_set().is_some() && agents.iter().any(|a| &a.id == *p))
            .map(|p| router::Route {
                agent: p.clone(),
                confidence: 1.0,
                reason: "continues the open change set",
            });
        let shortcut = if button_intent {
            None
        } else {
            continuing.or_else(|| {
                router::hinted(
                    &profiles.runtime.router.hints,
                    &agents,
                    &self.text,
                    &attached,
                )
                .or_else(|| router::by_phrase(&agents, &self.text))
            })
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
                output: Some(json!({"agent": r.agent, "confidence": r.confidence,
                                    "tool_set": self.tool_set.as_str()})),
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

    /// The conversation's open change set. The facade has none: its writes run at once.
    fn open_change_set(&self) -> Option<&OpenChangeSet> {
        self.ctx
            .change_set
            .as_ref()
            .filter(|c| c.is_open() && self.tool_set.is_commerce())
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
            self.note_problem("None of the store's tools that I may use fits this request.".into());
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

    fn result_chars(&self) -> usize {
        self.rt.profiles.runtime.context.result_chars
    }

    fn done_summaries(&self) -> Vec<Value> {
        self.history
            .iter()
            .map(|r| {
                let mut v = json!({"tool": r.tool, "args": r.input});
                if let Some(o) = &r.output {
                    v["result"] = compact(o, self.result_chars());
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
        let (mut plan, mut alias) = match first {
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
        // A plan with no steps answers from nothing: a question about the store is answered
        // from its tools. Ask once more, saying so (a greeting may still need no step).
        if plan.steps.is_empty()
            && self.request.ui_event.is_none()
            && self.usage.len() < self.limits.model_calls as usize
            && let Ok(again) = self
                .plan_once(
                    &[&worker, &planner_alias],
                    Some(EMPTY_PLAN.to_owned()),
                    serde_json::to_value(&plan).ok(),
                )
                .await
            && !again.0.steps.is_empty()
        {
            (plan, alias) = again;
        }
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
        // What the failed plan did not get to is still asked for: a new plan that only deals with
        // the failure would quietly drop it.
        let unrun = std::mem::take(&mut self.unrun);
        let asked = if unrun.is_empty() {
            problem
        } else {
            format!(
                "{problem} Not run yet from that plan, still to do unless this changes them: {}.",
                unrun.join(", ")
            )
        };
        // After a failure the planner model takes over (§6).
        match self
            .plan_once(&[&planner_alias, &worker], Some(asked), None)
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
                    self.note_problem(why);
                    return;
                }
                Flow::Revise(why) if self.revisions < self.limits.plan_revisions => {
                    // The new plan usually repeats the writes it already drafted. That changes
                    // those items; it does not ask for the same change a second time.
                    if let Some(d) = self.draft.as_mut() {
                        for item in &mut d.items {
                            item.replaceable = true;
                        }
                    }
                    match self.revise(why).await {
                        Ok(p) => plan = p,
                        Err(Stop(why)) => {
                            self.note_problem(why);
                            return;
                        }
                    }
                }
                Flow::Revise(why) => {
                    self.note_problem(why);
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
                self.note_problem(
                    "I did not remember anything about the person this is for.".into(),
                );
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
            // A read that needs one id the plan did not give, from a step that returned several,
            // runs once per row, as `each` would.
            let each = step
                .each
                .clone()
                .or_else(|| executor::implied_each(step, &tool, &view));
            let result = match &each {
                Some(source) => self.run_each(plan, step, &tool, source, &view).await,
                None => {
                    let bound = executor::bind(step, &tool, &view, &order, &self.pointed);
                    // The result it builds on says there is nothing to do: not called, not
                    // made up by a model either. The reply says so.
                    if let Some(why) = bound.empty {
                        let note = format!("{} was not called: {why}.", tool.tool);
                        self.skipped.push((tool.tool.clone(), note));
                        continue;
                    }
                    let mut input = bound.input;
                    if tool.is_write()
                        && self.surface == ChatSurface::Bo
                        && let Some(base) = self.replaceable_input(&tool.tool)
                    {
                        input = executor::merge(&base, &input);
                    }
                    self.undecided = bound.undecided;
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
                // Memories a guest does not have do not stop the answer: reading them (the profile)
                // goes on without them, and keeping one says it could not.
                Err(Flow::Stop(_)) if tool.remembers() && !tool.is_write() => {}
                Err(Flow::Stop(why)) if tool.remembers() => {
                    self.note_problem(format!("I could not remember that: {why}"));
                }
                Err(flow) => {
                    self.unrun = plan
                        .steps
                        .iter()
                        .skip_while(|s| s.id != step.id)
                        .map(|s| s.tool.clone())
                        .collect();
                    return flow;
                }
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
        repaired.missing.append(&mut self.undecided);
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
            _ if self.tool_set == ToolSet::Facade => {
                self.run_direct_write(step, tool, input, &notes).await
            }
            (_, ChatSurface::Bo) => {
                self.run_change_set_write(plan, step, tool, input, &notes)
                    .await
            }
            (_, ChatSurface::Fo) => self.run_shopper_write(step, tool, input, &notes).await,
        }?;
        self.history.push(record.clone());
        // What this result already shows goes on screen now, not when the turn ends.
        let wrote_change_set = tool.kind != ToolKind::Read
            && self.tool_set != ToolSet::Facade
            && self.surface == ChatSurface::Bo;
        self.progress_blocks(wrote_change_set).await;
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
            // A row that cannot fill the step is left out, not asked about one model call at a
            // time.
            if tool
                .required()
                .iter()
                .any(|f| bound.input.get(*f).is_none_or(Value::is_null))
            {
                continue;
            }
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
        // It fills what is open. What the arguments already hold stays: a bound `$item[1]…` is
        // not the model's to swap for a promotion it saw in a list.
        let merged = executor::merge(&v, &r.value);
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

    /// A tool error: try again with the tool commerce suggests when this agent may use it, or
    /// with the message when the arguments were wrong; otherwise stop and show the reason.
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
            // Arguments the plan got wrong (a field missing, a value out of range) are fixed by
            // planning again with the message; anything else is shown to the person.
            None if e.class == acom_contract::ErrorClass::InvalidInput => Flow::Revise(format!(
                "{tool} failed ({}): {} Plan it again with the arguments it needs.",
                e.code, e.message
            )),
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
        let read = match self.tool_set {
            ToolSet::Commerce => self.tools.read(&tool.tool, &input).await,
            ToolSet::Facade => self.tools.facade_call(&tool.tool, &input).await,
        };
        match read {
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

    /// A facade write: one call that changes the store at once, as a screen's save button does.
    /// There is no preview to show and no change set to hand to people.
    async fn run_direct_write(
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
                format!("Saving with {}", tool.tool),
                Some(&tool.tool),
            )
            .await;
        match self.tools.facade_call(&tool.tool, &input).await {
            Ok(out) => {
                self.end_step(
                    &h,
                    StepStatus::Done,
                    StepEnd {
                        summary: format!("{}: done", tool.tool),
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
                    slots: executor::slots_from_commit(tool, &out),
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
            // Tasks are the back office's; a storefront turn links nothing.
            task_id: (self.surface == ChatSurface::Bo)
                .then(|| self.request.task_id.clone())
                .flatten(),
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
                Err(e) => self.note_problem(format!(
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
    /// The product block this turn shows: the layout rule that fits the shopper and their words
    /// (agents/shopper.yml `layout_rules`), else the plan's choice.
    fn product_layout(&self, reply_plan: &str) -> (String, Option<&LayoutRule>) {
        let facts = self.profile_facts();
        // A first visit: a guest with no history yet. A signed-in shopper has a record.
        let first_visit =
            self.principal.customer_id().is_none() && self.ctx.is_first_turn() && facts.is_empty();
        let said = self.layout_words();
        let rule = shop::layout(&self.profile.layout_rules, &said, &facts, first_visit)
            .filter(|r| self.profile.blocks.contains(&r.ui));
        let ui = rule.map_or(reply_plan, |r| r.ui.as_str()).to_owned();
        (ui, rule)
    }

    /// A comparison shows the products' attributes side by side, which only `compare_products`
    /// reads. When the turn will show one and the plan only searched, compare the top few.
    async fn compare_for_layout(&mut self) {
        let Some(plan) = self.plan.clone() else {
            return;
        };
        let reply_plan = plan.reply_plan.clone().unwrap_or_default();
        if !["compare_table", "look_gallery", "product_list"].contains(&reply_plan.as_str())
            || self.history.iter().any(|r| r.tool == "compare_products")
            || self.product_layout(&reply_plan).0 != "compare_table"
        {
            return;
        }
        let Some(tool) = self
            .tool("compare_products")
            .filter(|t| self.profile.allows(t))
            .cloned()
        else {
            return;
        };
        let ids: Vec<Value> = self
            .product_rows(&["compare_table", "look_gallery", "product_list"])
            .iter()
            .filter_map(|i| {
                i.get("product_id")
                    .or_else(|| i.get("id"))
                    .filter(|v| v.as_str().is_some_and(|s| s.starts_with("prd_")))
                    .cloned()
            })
            .take(4)
            .collect();
        if ids.len() < 2 {
            return;
        }
        let _ = self
            .call(
                &plan,
                "compare",
                &tool,
                json!({"product_ids": ids}),
                vec!["product_ids: the top results, for the comparison the layout shows".into()],
            )
            .await;
    }

    fn product_rows(&self, uis: &[&str]) -> Vec<Value> {
        if let Some(r) = self.last_output(|r| {
            r.kind == ToolKind::Read
                && !records::shaped(&r.tool)
                && shop::items_of(r.output.as_ref().unwrap_or(&Value::Null))
                    .first()
                    .is_some_and(|i| i.get("name").is_some() && is_product_item(i))
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
        let out = self.compose_blocks(false);
        self.publish_blocks(out, true);
    }

    /// After a step: the blocks its results already make, on screen while the turn goes on. A
    /// write into the change set reads the change set again, so its items appear one by one.
    async fn progress_blocks(&mut self, wrote_change_set: bool) {
        if self.emit.is_closed() {
            return;
        }
        if wrote_change_set
            && self.draft.as_ref().is_some_and(|d| d.touched)
            && let Some(view) = self.change_set_view().await
        {
            self.view = Some(view);
        }
        let out = self.compose_blocks(true);
        self.publish_blocks(out, false);
    }

    /// The blocks the turn's results make, in the order they are shown. While the turn still
    /// works (`progress`), only blocks built from what it has read so far: no stand-in table
    /// of the last list read, which the final answer may not keep.
    fn compose_blocks(&self, progress: bool) -> Vec<Block> {
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
                .last_output(|r| r.tool == "search_action_log" || r.tool == "get_action_log")
                .and_then(|r| table::action_log(r.output.as_ref()?)),
            "task_list" => self
                .last_output(|r| r.tool == "list_tasks")
                .and_then(|r| crate::blocks::tasks::task_list(r.output.as_ref()?)),
            "order_detail" => self.order_detail_block(),
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
                .last_output(|r| {
                    matches!(
                        r.tool.as_str(),
                        "get_return_options" | "get_exchange_options" | "get_cancel_options"
                    )
                })
                .and_then(|r| shop::return_options(r.output.as_ref()?, &r.input)),
            "record_list" => self.record_block(),
            // The plan named a product layout but this turn read only records (orders, coupons):
            // those are the answer, not the products shown earlier.
            "compare_table" | "look_gallery" | "product_list"
                if allowed("record_list")
                    && self.product_read().is_none()
                    && self
                        .primary_read()
                        .is_some_and(|r| records::shaped(&r.tool)) =>
            {
                self.record_block()
            }
            "compare_table" | "look_gallery" | "product_list" => {
                let (ui, rule) = self.product_layout(&reply_plan);
                let ui = ui.as_str();
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
        // The plan's block could not be made from what was read, or the plan chose text: when
        // nothing else is shown, show the last list read as it is, so the person sees what the
        // turn found (products as a list on the storefront, anything else as a table).
        let block = if block.is_none() && out.is_empty() && cards.is_empty() && !progress {
            self.found_block(&allowed)
        } else {
            block
        };
        // Products come with what this turn read about them: their reviews and questions.
        let about_products = if allowed("record_list")
            && block.as_ref().is_some_and(|b| {
                matches!(
                    b.ui.as_str(),
                    "compare_table" | "look_gallery" | "product_list"
                )
            }) {
            self.product_content_blocks()
        } else {
            Vec::new()
        };
        // A gift is reviewed in its picker first; the card with the Place order button follows.
        if reply_plan == "gift_picker" {
            out.extend(block);
            out.extend(cards);
        } else {
            out.extend(cards);
            out.extend(block);
        }
        out.extend(about_products);
        out
    }

    /// Sends the turn's blocks: each keeps the id of the block in its place shown earlier this
    /// turn (so it replaces it, in place), and one that did not change is not sent again. At the
    /// end of the turn (`last`) blocks the answer does not keep are taken off the screen, failed
    /// checks are pointed at, and the blocks are kept for the reply and the turn log.
    fn publish_blocks(&mut self, out: Vec<Block>, last: bool) {
        // First where each block sits and the id it keeps there; then blocks that point at
        // another (a gift picker at its checkout card) point at that one's kept id.
        let mut counts: BTreeMap<String, usize> = BTreeMap::new();
        let mut places: Vec<String> = Vec::with_capacity(out.len());
        let mut renamed: Vec<(String, String)> = Vec::new();
        for b in &out {
            let nth = counts.entry(place_of(&b.ui, 0)).or_default();
            let place = place_of(&b.ui, *nth);
            *nth += 1;
            if let Some(s) = self.shown.iter().find(|s| s.place == place) {
                renamed.push((b.block_id.clone(), s.block_id.clone()));
            }
            places.push(place);
        }
        let mut kept: Vec<String> = Vec::new();
        let mut placed: Vec<Block> = Vec::with_capacity(out.len());
        for (mut b, place) in out.into_iter().zip(places) {
            if let Some((_, id)) = renamed.iter().find(|(from, _)| *from == b.block_id) {
                b.block_id.clone_from(id);
            }
            rename_block_ids(&mut b.data, &renamed);
            for a in &mut b.actions {
                if let Action::Open { target, .. } = a {
                    let mut t = Value::String(std::mem::take(target));
                    rename_block_ids(&mut t, &renamed);
                    *target = t.as_str().unwrap_or_default().to_owned();
                }
            }
            let sent = serde_json::to_value(&b).unwrap_or(Value::Null);
            match self.shown.iter_mut().find(|s| s.place == place) {
                Some(s) if s.sent == sent => {}
                Some(s) => {
                    s.sent = sent;
                    self.emit.send(Event::Block(Box::new(b.clone())));
                }
                None => {
                    self.shown.push(ShownBlock {
                        place: place.clone(),
                        block_id: b.block_id.clone(),
                        sent,
                    });
                    self.emit.send(Event::Block(Box::new(b.clone())));
                }
            }
            kept.push(place);
            placed.push(b);
        }
        if !last {
            return;
        }
        for gone in self.shown.iter().filter(|s| !kept.contains(&s.place)) {
            self.emit.send(Event::BlockRemoved {
                block_id: gone.block_id.clone(),
            });
        }
        self.shown.retain(|s| kept.contains(&s.place));
        for b in placed {
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
            .map(|r| json!({"tool": r.tool, "result": compact(r.output.as_ref().unwrap_or(&Value::Null), self.result_chars())}))
            .collect();
        // Writes that ran at once (the facade's saves) and what they answered.
        let saved: Vec<Value> = self
            .direct_writes()
            .map(|r| json!({"tool": r.tool, "result": compact(r.output.as_ref().unwrap_or(&Value::Null), self.result_chars())}))
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
        json!({"request": self.text, "change_set": cs, "reads": reads, "saved": saved, "blocks_shown": blocks,
               "problems": self.problems, "not_called": self.skipped.iter().map(|(_, note)| note).collect::<Vec<_>>(),
               "interpretation": self.plan.as_ref().map(|p| p.interpretation.iter().map(|u| u.text.clone()).collect::<Vec<_>>())})
    }

    /// Writes this turn ran at once, without a preview (the facade's saves).
    fn direct_writes(&self) -> impl Iterator<Item = &StepRecord> {
        self.history
            .iter()
            .filter(|r| r.kind != ToolKind::Read && r.preview.is_none() && r.output.is_some())
    }

    /// The last list a read returned, as a block this agent may show.
    fn found_block(&self, allowed: &dyn Fn(&str) -> bool) -> Option<Block> {
        if let Some(read) = self.product_read()
            && allowed("product_list")
        {
            return shop::products("product_list", read.output.as_ref()?, None);
        }
        if allowed("order_detail")
            && let Some(b) = self.order_detail_block()
        {
            return Some(b);
        }
        // Only who the shopper is was read ("what do you remember about me?"): their profile.
        if allowed("profile_card")
            && let Some(r) = self.primary_read().filter(|r| r.tool == "get_my_profile")
            && let Some(b) = shopper::profile_card(r.output.as_ref()?)
        {
            return Some(b);
        }
        if allowed("record_list") {
            return self.record_block();
        }
        let read = self.last_output(|r| {
            r.kind == ToolKind::Read
                && r.output
                    .as_ref()
                    .is_some_and(|o| !shop::items_of(o).is_empty())
        })?;
        if allowed("product_table") {
            table::product_table(read.output.as_ref()?)
        } else {
            None
        }
    }

    /// The newest read this turn whose items are products (search results, a wishlist), not
    /// records that only point at products (reviews, alerts).
    fn product_read(&self) -> Option<&StepRecord> {
        self.history.iter().rev().find(|r| {
            r.kind == ToolKind::Read
                && !records::shaped(&r.tool)
                && r.output.as_ref().is_some_and(|o| {
                    let items = shop::items_of(o);
                    !items.is_empty() && items.iter().all(is_product_item)
                })
        })
    }

    /// The order this turn read, with its lines (`get_order`; not its payment or delivery). Of
    /// several (each of the shopper's orders read to find one), the one whose parcel the turn
    /// tracked, else the one holding what the request names, else the last read.
    fn order_detail_block(&self) -> Option<Block> {
        let orders: Vec<&Value> = self
            .history
            .iter()
            .rev()
            .filter_map(|r| r.output.as_ref())
            .filter(|o| o.get("order_number").is_some() && o.get("ordered_item").is_some())
            .collect();
        // `track_shipment` names its parcel `delivery_id`, `get_delivery` as its `id`.
        let tracked = self
            .last_output(|r| matches!(r.tool.as_str(), "track_shipment" | "get_delivery"))
            .and_then(|r| {
                let o = r.output.as_ref()?;
                o.get("delivery_id").or_else(|| o.get("id"))?.as_str()
            });
        let order = tracked
            .and_then(|t| {
                orders
                    .iter()
                    .find(|o| o.pointer("/delivery/id").and_then(Value::as_str) == Some(t))
            })
            // Else the one with what the request names ("my windbreaker order").
            .or_else(|| {
                let said: Vec<String> = self
                    .text
                    .to_lowercase()
                    .split(|c: char| !c.is_alphanumeric())
                    .filter(|w| {
                        w.len() >= 4 && !["order", "orders", "track", "where", "show"].contains(w)
                    })
                    .map(str::to_owned)
                    .collect();
                orders.iter().find(|o| {
                    o.get("ordered_item")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|l| l.get("name").and_then(Value::as_str))
                        .any(|name| {
                            let name = name.to_lowercase();
                            said.iter().any(|w| name.contains(w.as_str()))
                        })
                })
            })
            .or_else(|| orders.first())?;
        table::order_detail(order)
    }

    /// The read a records answer shows: the newest one with a shape of its own, else the newest
    /// that is not only about who is asking.
    fn primary_read(&self) -> Option<&StepRecord> {
        let reads: Vec<&StepRecord> = self
            .history
            .iter()
            .rev()
            .filter(|r| r.kind == ToolKind::Read && r.output.as_ref().is_some_and(|o| !o.is_null()))
            .collect();
        // What the agent always has at hand (the shopper's orders and profile) is often read only
        // to set up the answer, so another read with a shape of its own comes first.
        let at_hand = |r: &&&StepRecord| self.profile.tool_scope.always.contains(&r.tool);
        reads
            .iter()
            .find(|r| records::shaped(&r.tool) && !at_hand(r))
            .or_else(|| reads.iter().find(|r| records::shaped(&r.tool)))
            .or_else(|| {
                reads
                    .iter()
                    .find(|r| !records::SUPPORTING.contains(&r.tool.as_str()))
            })
            .or_else(|| reads.first())
            .copied()
    }

    /// `record_list` from the primary read; reviews come with their summary when both were read.
    fn record_block(&self) -> Option<Block> {
        let read = self.primary_read()?;
        if matches!(read.tool.as_str(), "list_reviews" | "get_review_summary") {
            return self.reviews_block();
        }
        records::record_list(&read.tool, read.output.as_ref()?)
    }

    fn reviews_block(&self) -> Option<Block> {
        let of = |tool: &str| {
            self.last_output(|r| r.tool == tool)
                .and_then(|r| r.output.as_ref())
        };
        records::reviews_with_summary(of("list_reviews"), of("get_review_summary"))
    }

    /// Reviews and questions this turn read, each as its own `record_list`.
    fn product_content_blocks(&self) -> Vec<Block> {
        let mut out: Vec<Block> = self.reviews_block().into_iter().collect();
        out.extend(
            self.last_output(|r| r.tool == "list_questions")
                .and_then(|r| records::record_list(&r.tool, r.output.as_ref()?)),
        );
        out
    }

    /// A problem the reply names, once however often the turn ran into it (a plan revised
    /// runs its steps again).
    fn note_problem(&mut self, why: String) {
        if !self.problems.contains(&why) {
            self.problems.push(why);
        }
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
                "task_list" => parts.push("Here is what is waiting and what is in progress.".into()),
                "change_set" => {}
                _ => parts.push("Here is what I found.".into()),
            }
        }
        for r in self.direct_writes() {
            parts.push(format!("{} is saved.", r.tool));
        }
        parts.dedup();
        parts.extend(self.problems.iter().cloned());
        if parts.is_empty() {
            let read = self.history.iter().any(|r| r.kind == ToolKind::Read);
            parts.push(if read {
                "I looked this up, but nothing matched.".into()
            } else {
                "I could not find anything to show for that.".into()
            });
        }
        parts.join(" ")
    }

    /// The reply's text: the `text` of the structured answer, or the answer itself when a model
    /// wrote plain text, without any copy of its instructions. A structured answer stopped
    /// before its closing brace (the token limit, or the schema's `maxLength` closing the string)
    /// still gives its text, marked cut, never the raw JSON.
    async fn reply_text(
        &mut self,
        worker: &str,
        req: ChatRequest,
        prompt: &str,
    ) -> Option<ReplyDraft> {
        // What it must not copy: its instructions, and what a check told it when asking again
        // ("That repeats the request…" came back once as the answer).
        let guide = std::iter::once(prompt)
            .chain(
                req.messages
                    .iter()
                    .skip(2)
                    .filter(|m| m.role == acom_llm::Role::User)
                    .map(|m| m.content.as_str()),
            )
            .collect::<Vec<_>>()
            .join("\n");
        let (raw, stopped) = self.model_text("reply", &[worker], req).await?;
        let parsed = acom_llm::parse::extract_json(&raw)
            .and_then(|v| v.get("text").and_then(Value::as_str).map(str::to_owned));
        let (text, cut) = match parsed {
            Some(t) => {
                let full = t.chars().count() >= REPLY_MAX_CHARS;
                (t, stopped || full)
            }
            None => match checker::text_of_cut_reply(&raw) {
                Some(t) => (t, true),
                None => (raw, stopped),
            },
        };
        let text = without_echo(text.trim().to_owned(), &guide)?;
        Some(ReplyDraft { text, cut })
    }

    /// Marks the last model call as turned down by a reply check.
    fn reject_last_call(&mut self, why: &str) {
        if let Some(last) = self.usage.last_mut() {
            last.succeeded = false;
            last.error = Some(why.to_owned());
        }
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
        // Structured, like the plan: a reasoning model asked for free text can spend the whole
        // budget thinking and return nothing; a schema makes it write the answer.
        let req = ChatRequest::new(
            "reply",
            vec![
                Message::system(prompt.clone()),
                Message::user(format!("Results of this turn: {facts}")),
            ],
        )
        .json("reply", reply_schema())
        .max_tokens(500);
        let worker = self.worker().to_owned();
        let Some(mut draft) = self.reply_text(&worker, req.clone(), &prompt).await else {
            return fallback;
        };
        // A reply the length limit stopped ends mid-sentence: ask once for a shorter one, else
        // keep its finished sentences.
        if draft.cut {
            self.reject_last_call("the reply was cut at the length limit");
            self.log_rejected_reply(
                "Checking the reply fits the length limit",
                "The reply was cut at the length limit".into(),
                json!({"reply": draft.text}),
            )
            .await;
            let retried = if self.usage.len() < self.limits.model_calls as usize {
                let mut again = req.clone();
                again.messages.push(Message::assistant(draft.text.clone()));
                again.messages.push(Message::user(CUT_REPLY));
                self.reply_text(&worker, again, &prompt).await
            } else {
                None
            };
            draft = match retried {
                Some(d) if !d.cut => d,
                other => {
                    let shorter = other.and_then(ReplyDraft::finished);
                    match shorter.or_else(|| checker::whole_sentences(&draft.text)) {
                        Some(text) => ReplyDraft { text, cut: false },
                        None => return fallback,
                    }
                }
            };
        }
        let mut text = draft.text;
        // A reply that repeats the request is the model thinking aloud: ask once for the answer
        // itself, else fall back to the plain reply.
        if checker::repeats_request(&text, &self.text) {
            self.reject_last_call("the reply repeated the request");
            self.log_rejected_reply(
                "Checking the reply answers instead of repeating the request",
                "The reply repeated the request".into(),
                json!({"reply": text}),
            )
            .await;
            if self.usage.len() >= self.limits.model_calls as usize {
                return fallback;
            }
            let mut again = req.clone();
            again.messages.push(Message::assistant(text.clone()));
            again.messages.push(Message::user(REPEATED_REQUEST));
            match self
                .reply_text(&worker, again, &prompt)
                .await
                .and_then(ReplyDraft::finished)
            {
                Some(t) if !checker::repeats_request(&t, &self.text) => text = t,
                _ => return fallback,
            }
        }
        // Tool and field names mean nothing to the person: ask once for plain words, and keep
        // whichever reply names fewer of them.
        let mut names: BTreeSet<String> = self
            .history
            .iter()
            .map(|r| r.tool.clone())
            .chain(self.skipped.iter().map(|(tool, _)| tool.clone()))
            .collect();
        for r in &self.history {
            if let Some(out) = &r.output {
                checker::field_names(out, &mut names);
            }
        }
        let named = checker::system_words(&text, &self.text, &names);
        if !named.is_empty() && self.usage.len() < self.limits.model_calls as usize {
            self.reject_last_call("the reply named tools or fields");
            self.log_rejected_reply(
                "Checking the reply speaks in the store's words",
                format!("Tool or field names in the reply: {}", named.join(", ")),
                json!({"reply": text, "names": named}),
            )
            .await;
            let mut again = req.clone();
            again.messages.push(Message::assistant(text.clone()));
            again.messages.push(Message::user(format!(
                "{SYSTEM_WORDS}: {}. Write it again for the person in plain words, two or three \
                 sentences: what it means for the store, not which tool or field said it.",
                named.join(", ")
            )));
            if let Some(t) = self
                .reply_text(&worker, again, &prompt)
                .await
                .and_then(ReplyDraft::finished)
                && checker::system_words(&t, &self.text, &names).len() < named.len()
            {
                text = t;
            }
        }
        // Owned copies: the retry below calls the model, which needs `self` mutably.
        let mut owned: Vec<Value> = self
            .history
            .iter()
            .filter_map(|r| r.output.clone())
            .collect();
        owned.extend(
            self.history
                .iter()
                .filter_map(|r| r.preview.as_ref())
                .filter_map(|p| serde_json::to_value(p).ok()),
        );
        owned.push(facts);
        let sources: Vec<&Value> = owned.iter().collect();
        let asked = self.text.clone();
        let mut unsupported = checker::unsupported_numbers(&text, &sources, &[&asked]);
        // One more try, naming the numbers no result holds (a sum the model did itself, a
        // number it guessed); a second miss falls back to the plain reply.
        if !unsupported.is_empty() && self.usage.len() < self.limits.model_calls as usize {
            if let Some(last) = self.usage.last_mut() {
                last.succeeded = false;
                last.error = Some(format!(
                    "numbers not in the results: {}",
                    unsupported.join(", ")
                ));
            }
            self.log_unsupported_numbers(&text, &unsupported).await;
            let mut again = req;
            again.messages.push(Message::assistant(text.clone()));
            again.messages.push(Message::user(format!(
                "These numbers are not in the results: {}. Write the reply again using only numbers that appear in the results, or none.",
                unsupported.join(", ")
            )));
            if let Some(t) = self
                .reply_text(&worker, again, &prompt)
                .await
                .and_then(ReplyDraft::finished)
            {
                unsupported = checker::unsupported_numbers(&t, &sources, &[&asked]);
                text = t;
            }
        }
        if unsupported.is_empty() {
            let today = self
                .clock
                .now()
                .with_timezone(&self.settings.time.zone)
                .date_naive();
            checker::fix_weekdays(&text, today)
        } else {
            tracing::info!(numbers = ?unsupported, "reply used numbers not in the results; using the plain reply");
            self.log_unsupported_numbers(&text, &unsupported).await;
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

    /// A reply a check turned down, kept in the turn's log with why (the model call itself is
    /// already recorded as answered).
    async fn log_rejected_reply(&mut self, check: &str, why: String, output: Value) {
        let h = self.begin_step(StepKind::Check, check.into(), None).await;
        self.end_step(
            &h,
            StepStatus::Failed,
            StepEnd {
                summary: why,
                output: Some(output),
                ..StepEnd::default()
            },
        )
        .await;
    }

    async fn log_unsupported_numbers(&mut self, text: &str, unsupported: &[String]) {
        self.log_rejected_reply(
            "Checking the reply's numbers against the results",
            format!("Numbers not in the results: {}", unsupported.join(", ")),
            json!({"reply": text, "unsupported": unsupported}),
        )
        .await;
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
            // A streamed reply is on screen already; `message.done` carries the one that stands
            // (a check may have changed it). A reply nobody saw written comes in sentences.
            if !self.streamed {
                for piece in sentences(text) {
                    self.emit.send(Event::MessageDelta { text: piece });
                }
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
                usage: self.stored_usage(&usage),
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

    /// The usage the turn row keeps: the totals, and which tool set the turn used, for the
    /// evaluation to read back (`GET …/conversations/{id}?log=true`).
    fn stored_usage(&self, usage: &TurnUsage) -> Value {
        let mut v = serde_json::to_value(usage).unwrap_or_else(|_| json!({}));
        if let Value::Object(o) = &mut v {
            o.insert("tool_set".into(), json!(self.tool_set.as_str()));
        }
        v
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

/// A tool result for a prompt (`agents/runtime.yml context.result_chars`): whole when its JSON
/// fits `budget` characters. Otherwise the biggest part is halved until it fits: a list keeps
/// its first half and says how many items it left out, a string keeps its first half and ends
/// in `…`. Cutting the biggest part first keeps detail where the result is small: a 20-product
/// search loses products before the first products lose their sizes.
fn compact(v: &Value, budget: usize) -> Value {
    let mut out = v.clone();
    while json_chars(&out) > budget && halve_biggest(&mut out) {}
    out
}

/// A cut list ends in this marker; the number is how many items it left out.
fn left_out(v: &Value) -> Option<usize> {
    v.as_str()?
        .strip_prefix("… ")?
        .strip_suffix(" more")?
        .parse()
        .ok()
}

fn json_chars(v: &Value) -> usize {
    v.to_string().chars().count()
}

/// Halves the biggest list (of two items or more) or string (of 80 characters or more) inside
/// `v`; false when nothing is left to halve.
fn halve_biggest(v: &mut Value) -> bool {
    fn biggest(v: &Value, path: &mut Vec<usize>, best: &mut Option<(usize, Vec<usize>)>) {
        let cuttable = match v {
            Value::Array(a) => a.iter().filter(|x| left_out(x).is_none()).count() >= 2,
            Value::String(s) => s.chars().count() >= 80,
            _ => false,
        };
        if cuttable {
            let size = json_chars(v);
            if best.as_ref().is_none_or(|(b, _)| size > *b) {
                *best = Some((size, path.clone()));
            }
        }
        let children: Vec<&Value> = match v {
            Value::Array(a) => a.iter().collect(),
            Value::Object(o) => o.values().collect(),
            _ => Vec::new(),
        };
        for (i, c) in children.into_iter().enumerate() {
            path.push(i);
            biggest(c, path, best);
            path.pop();
        }
    }
    let mut best = None;
    biggest(v, &mut Vec::new(), &mut best);
    let Some((_, path)) = best else {
        return false;
    };
    let mut node = v;
    for i in path {
        node = match node {
            Value::Array(a) => &mut a[i],
            Value::Object(o) => match o.values_mut().nth(i) {
                Some(x) => x,
                None => return false,
            },
            _ => return false,
        };
    }
    match node {
        Value::Array(a) => {
            let before = a.last().and_then(left_out);
            if before.is_some() {
                a.pop();
            }
            let keep = a.len().div_ceil(2);
            let dropped = a.len() - keep;
            a.truncate(keep);
            a.push(json!(format!("… {} more", before.unwrap_or(0) + dropped)));
        }
        Value::String(s) => {
            let text = s.strip_suffix('…').unwrap_or(s);
            let keep = text.chars().count() / 2;
            *s = format!("{}…", text.chars().take(keep).collect::<String>());
        }
        _ => return false,
    }
    true
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
