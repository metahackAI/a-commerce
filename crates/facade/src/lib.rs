//! A deliberately screen-shaped tool set for the evaluation (`docs/design.md` §7).
//!
//! Written by us as generic examples of the patterns in `docs/design.md` §2, over the same
//! back end: every screen calls the real operations through the commerce [`Registry`], so both
//! tool sets read and change the same data. Not part of the product. What the screens copy from
//! a typical screen-shaped back end:
//!
//! | pattern (design.md §2) | here |
//! |---|---|
//! | screen-shaped endpoints | one refund is a grid (`getClmGridList`), a popup init (`getRfndPopInitData`) and a save (`saveRfndProc`) |
//! | abbreviated names | `getPrmtTgtMbrList`, `saveCustMsgSend`, fields like `prmtTpCd`, `rspbCd` |
//! | codes instead of meaning | `prmtTpCd: "T03"`, `clmStatCd: "R10"`; the table is in `codes.rs`, never in a schema |
//! | reads as POST, separate count | every grid is a POST without a total, and has a `…Cnt` twin |
//! | bulk saves | `savePrmtList(rows[])` with `rowStat` I/U/D |
//! | version duplicates | `getPrdDtlInfo` and `getPrdDtlInfoV2` |
//! | rules only in people's heads | no checks named, no policy words in any description, writes run without rule checks |
//!
//! Writes run directly, as a legacy back end does: no preview handed back, no change set, no
//! person's approval, and the real operation's rule checks are not applied. Row-level security
//! and scopes still apply: the screens run with the caller's context, and every real operation
//! they touch passes the same guard as the normal tool endpoint. The facade answers only in
//! demo mode (`docs/api/README.md` §7); a demo store is loaded again between evaluation runs.

mod codes;
mod screens;

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use acom_changeset::{LogEntry, log_action};
use acom_contract::{
    Committed, DynOp, ErrorClass, ErrorCode, Kind, ManifestEntry, OpMeta, Preview, Registry,
    ToolError,
};
use acom_core::enums::ActionStatus;
use acom_store::Cx;
use futures::future::BoxFuture;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

/// A request the screen does not take.
pub const ERR_PARAM: ErrorCode = ErrorCode::new("ERR_PARAM", ErrorClass::InvalidInput);
/// A row the screen cannot process.
pub const ERR_PROC: ErrorCode = ErrorCode::new("ERR_PROC", ErrorClass::InvalidState);

/// One screen call: typed input, JSON out.
pub trait Screen: Send + Sync + 'static {
    /// Names and kind. The address names the real domain, so the same scopes apply.
    const META: OpMeta;
    /// Must produce a closed schema.
    type Input: DeserializeOwned + JsonSchema + Send;

    fn call(
        &self,
        be: &Backend<'_>,
        input: Self::Input,
    ) -> impl Future<Output = Result<Value, ToolError>> + Send;
}

/// A screen with JSON in and out.
pub trait DynScreen: Send + Sync {
    fn meta(&self) -> &'static OpMeta;
    fn input_schema(&self) -> &Value;
    fn call<'a>(
        &'a self,
        be: &'a Backend<'a>,
        input: Value,
    ) -> BoxFuture<'a, Result<Value, ToolError>>;
}

struct Adapter<S> {
    screen: S,
    schema: Value,
}

impl<S: Screen> DynScreen for Adapter<S> {
    fn meta(&self) -> &'static OpMeta {
        &S::META
    }

    fn input_schema(&self) -> &Value {
        &self.schema
    }

    fn call<'a>(
        &'a self,
        be: &'a Backend<'a>,
        input: Value,
    ) -> BoxFuture<'a, Result<Value, ToolError>> {
        Box::pin(async move {
            let input: S::Input = serde_json::from_value(input)
                .map_err(|e| ToolError::new(ERR_PARAM, format!("Invalid parameter: {e}")))?;
            self.screen.call(be, input).await
        })
    }
}

/// Checks a caller may use one real operation (audience and scopes), as the tool endpoint does.
pub type Guard<'g> = dyn Fn(&OpMeta) -> Result<(), ToolError> + Send + Sync + 'g;

/// The back end one screen call runs against: the real registry, the caller's context, and the
/// guard every real operation passes.
pub struct Backend<'a> {
    registry: &'a Registry<Cx>,
    cx: &'a Cx,
    guard: &'a Guard<'a>,
    screen: &'static str,
}

impl std::fmt::Debug for Backend<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Backend")
            .field("screen", &self.screen)
            .finish()
    }
}

/// A real operation's error as the screen passes it on: its code and sentence, without the
/// A-Commerce tool it suggests next (this tool set has none of them).
fn legacy(e: ToolError) -> ToolError {
    ToolError {
        next_tool: None,
        detail: None,
        ..e
    }
}

impl<'a> Backend<'a> {
    pub fn new(
        registry: &'a Registry<Cx>,
        cx: &'a Cx,
        guard: &'a Guard<'a>,
        screen: &'static str,
    ) -> Self {
        Self {
            registry,
            cx,
            guard,
            screen,
        }
    }

    pub fn cx(&self) -> &Cx {
        self.cx
    }

    fn op(&self, tool: &str) -> Result<&'a Arc<dyn DynOp<Cx>>, ToolError> {
        let op = self.registry.get(tool).ok_or_else(|| {
            ToolError::new(
                ErrorCode::INTERNAL,
                format!("The screen's back end has no `{tool}`."),
            )
        })?;
        (self.guard)(op.meta()).map_err(legacy)?;
        Ok(op)
    }

    /// A real read.
    pub async fn read(&self, tool: &str, input: Value) -> Result<Value, ToolError> {
        let op = self.op(tool)?;
        if op.meta().kind != Kind::Read {
            return Err(ToolError::new(
                ErrorCode::INTERNAL,
                format!("`{tool}` is not a read."),
            ));
        }
        op.read(self.cx, input).await.map_err(legacy)
    }

    /// A real write, run at once: the operation plans it and commits the plan in one
    /// transaction, whatever its rule checks say and without a person's approval. Returns what
    /// the write produced (`id` and its slots). Never a person-only action, and never a write
    /// that only prepares (an order is placed only by the shopper's own button).
    pub async fn run(&self, tool: &str, input: Value) -> Result<Committed, ToolError> {
        let op = self.op(tool)?;
        let meta = op.meta();
        if !matches!(meta.kind, Kind::Write | Kind::WriteApproval)
            || acom_changeset::preview::person_endpoint(self.cx, meta.tool).is_some()
        {
            return Err(ToolError::new(
                ErrorCode::INTERNAL,
                format!("`{tool}` cannot be run from a screen."),
            ));
        }
        let planned = op.preview(self.cx, input).await.map_err(legacy)?;
        let skipped = planned.inner.checks.iter().filter(|c| !c.passed).count();
        let mut tx = self.cx.begin().await.map_err(ToolError::from)?;
        let done = op.commit(&mut tx, &planned.plan).await.map_err(legacy)?;
        let entity = planned.inner.changes.first().map(|c| c.entity.clone());
        log_action(
            &mut tx,
            LogEntry {
                object: entity
                    .as_deref()
                    .filter(|_| !done.id.is_empty())
                    .map(|e| (e, done.id.as_str())),
                result: Some(json!({"id": done.id, "produced": done.produced,
                    "via": self.screen, "checks_not_applied": skipped})),
                ..LogEntry::new(meta.action_type.0, ActionStatus::Completed, meta.tool)
            },
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(done)
    }

    /// A write that only prepares (`prepare_order`): the preview is stored as on the normal
    /// path, so only the shopper's own button can place it.
    pub async fn prepare(&self, tool: &str, input: Value) -> Result<Preview, ToolError> {
        self.op(tool)?;
        acom_changeset::preview(self.registry, self.cx, tool, input, None)
            .await
            .map_err(legacy)
    }
}

/// Every screen, by name.
pub struct Facade {
    screens: BTreeMap<&'static str, Arc<dyn DynScreen>>,
}

impl std::fmt::Debug for Facade {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Facade")
            .field("screens", &self.screens.keys().collect::<Vec<_>>())
            .finish()
    }
}

/// Collects the screens and refuses ones that would not work as tools.
#[derive(Default)]
pub struct Builder {
    screens: BTreeMap<&'static str, Arc<dyn DynScreen>>,
    problems: Vec<String>,
}

impl std::fmt::Debug for Builder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Builder")
            .field("screens", &self.screens.len())
            .finish()
    }
}

/// `getXxx` or `saveXxx`, letters and digits only: never a `verb_noun` tool name.
fn is_screen_name(name: &str) -> bool {
    (name.starts_with("get") || name.starts_with("save"))
        && name.chars().all(|c| c.is_ascii_alphanumeric())
        && name.chars().any(|c| c.is_ascii_uppercase())
}

impl Builder {
    pub fn add<S: Screen>(&mut self, screen: S) -> &mut Self {
        let m = &S::META;
        let schema = acom_contract::schema::closed_schema_for::<S::Input>();
        for v in acom_contract::schema::violations(&schema) {
            self.problems.push(format!("{}: {v}", m.tool));
        }
        if !is_screen_name(m.tool) {
            self.problems
                .push(format!("{}: a screen is named getXxx or saveXxx", m.tool));
        }
        if !m.address.ends_with(&format!(".{}", m.tool)) {
            self.problems
                .push(format!("{}: the address ends with the screen name", m.tool));
        }
        if !matches!(m.kind, Kind::Read | Kind::Write) || !m.person_scope.is_empty() {
            self.problems
                .push(format!("{}: a screen is a read or a write", m.tool));
        }
        if !m.checks.is_empty() || !m.consumes.is_empty() || !m.produces.is_empty() {
            self.problems.push(format!(
                "{}: screens declare no checks and no links",
                m.tool
            ));
        }
        let adapter = Adapter { screen, schema };
        if self.screens.insert(m.tool, Arc::new(adapter)).is_some() {
            self.problems.push(format!("{}: registered twice", m.tool));
        }
        self
    }

    pub fn build(self) -> Result<Facade, Vec<String>> {
        if self.problems.is_empty() {
            Ok(Facade {
                screens: self.screens,
            })
        } else {
            Err(self.problems)
        }
    }
}

impl Facade {
    /// Every screen. Fails when a schema is not closed or a screen is named like a tool.
    pub fn build() -> Result<Self, Vec<String>> {
        let mut b = Builder::default();
        screens::register(&mut b);
        b.build()
    }

    pub fn get(&self, tool: &str) -> Option<&Arc<dyn DynScreen>> {
        self.screens.get(tool)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn DynScreen>> {
        self.screens.values()
    }

    pub fn len(&self) -> usize {
        self.screens.len()
    }

    pub fn is_empty(&self) -> bool {
        self.screens.is_empty()
    }

    /// The facade's manifest, in the shape of `GET /v1/tools`.
    pub fn manifest(&self, visible: impl Fn(&OpMeta) -> bool) -> Vec<ManifestEntry> {
        self.screens
            .values()
            .filter(|s| visible(s.meta()))
            .map(|s| ManifestEntry {
                meta: *s.meta(),
                input_schema: s.input_schema().clone(),
            })
            .collect()
    }

    /// Runs one screen with the caller's context. `guard` is asked before every real
    /// operation the screen touches.
    pub async fn call(
        &self,
        tool: &str,
        registry: &Registry<Cx>,
        cx: &Cx,
        guard: &Guard<'_>,
        input: Value,
    ) -> Result<Value, ToolError> {
        let screen = self
            .get(tool)
            .ok_or_else(|| ToolError::new(ErrorCode::NOT_FOUND, format!("No screen `{tool}`.")))?;
        let be = Backend::new(registry, cx, guard, screen.meta().tool);
        screen.call(&be, input).await
    }
}

#[cfg(test)]
mod tests;
