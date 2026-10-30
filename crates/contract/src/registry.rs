//! Type-erased registry of operations, and the tool manifest built from it.

use std::collections::BTreeMap;
use std::sync::Arc;

use futures::future::BoxFuture;
use serde::Serialize;
use serde_json::Value;

use crate::error::{ErrorCode, ToolError};
use crate::job::{DynJob, Job, JobAdapter, JobMeta, kind_is_valid};
use crate::meta::{Kind, OpMeta};
use crate::op::{
    Committed, Ctx, Operation, PersonOp, Planned, ReadOp, UndoSupport, Verification, WriteOp,
};
use crate::schema::closed_schema_for;

/// A preview with its plan as JSON, ready to be stored.
#[derive(Debug, Clone, PartialEq)]
pub struct DynPlanned {
    pub plan: Value,
    pub inner: Planned<()>,
}

/// One operation with JSON in and out.
pub trait DynOp<C: Ctx>: Send + Sync {
    fn meta(&self) -> &'static OpMeta;
    fn input_schema(&self) -> &Value;
    fn read<'a>(&'a self, cx: &'a C, input: Value) -> BoxFuture<'a, Result<Value, ToolError>>;
    fn preview<'a>(
        &'a self,
        cx: &'a C,
        input: Value,
    ) -> BoxFuture<'a, Result<DynPlanned, ToolError>>;
    fn commit<'a>(
        &'a self,
        tx: &'a mut C::Tx,
        plan: &'a Value,
    ) -> BoxFuture<'a, Result<Committed, ToolError>>;
    fn verify<'a>(
        &'a self,
        cx: &'a C,
        plan: &'a Value,
        done: &'a Committed,
    ) -> BoxFuture<'a, Result<Verification, ToolError>>;
    fn undo<'a>(
        &'a self,
        tx: &'a mut C::Tx,
        plan: &'a Value,
        done: &'a Committed,
    ) -> BoxFuture<'a, Result<(), ToolError>>;
    /// Runs a person-only endpoint. Other kinds refuse.
    fn person<'a>(
        &'a self,
        tx: &'a mut C::Tx,
        input: Value,
    ) -> BoxFuture<'a, Result<Value, ToolError>> {
        let _ = (tx, input);
        Box::pin(async move { Err(ToolError::invalid_input("not a person-only endpoint")) })
    }
    /// Whether `undo` can take this write back.
    fn undo_support(&self) -> UndoSupport {
        UndoSupport::NotSupported
    }
}

fn parse<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, ToolError> {
    serde_json::from_value(v).map_err(|e| ToolError::invalid_input(e.to_string()))
}

fn to_json<T: Serialize>(v: &T) -> Result<Value, ToolError> {
    serde_json::to_value(v).map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))
}

fn not_a(kind: &str, meta: &OpMeta) -> ToolError {
    ToolError::new(
        ErrorCode::INVALID_INPUT,
        format!("`{}` is not a {kind} operation", meta.tool),
    )
}

struct ReadAdapter<O> {
    op: O,
    schema: Value,
}

impl<C: Ctx, O: ReadOp<C>> DynOp<C> for ReadAdapter<O> {
    fn meta(&self) -> &'static OpMeta {
        &O::META
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn read<'a>(&'a self, cx: &'a C, input: Value) -> BoxFuture<'a, Result<Value, ToolError>> {
        Box::pin(async move {
            let out = self.op.read(cx, parse(input)?).await?;
            to_json(&out)
        })
    }
    fn preview<'a>(&'a self, _: &'a C, _: Value) -> BoxFuture<'a, Result<DynPlanned, ToolError>> {
        Box::pin(async move { Err(not_a("write", &O::META)) })
    }
    fn commit<'a>(
        &'a self,
        _: &'a mut C::Tx,
        _: &'a Value,
    ) -> BoxFuture<'a, Result<Committed, ToolError>> {
        Box::pin(async move { Err(not_a("write", &O::META)) })
    }
    fn verify<'a>(
        &'a self,
        _: &'a C,
        _: &'a Value,
        _: &'a Committed,
    ) -> BoxFuture<'a, Result<Verification, ToolError>> {
        Box::pin(async move { Err(not_a("write", &O::META)) })
    }
    fn undo<'a>(
        &'a self,
        _: &'a mut C::Tx,
        _: &'a Value,
        _: &'a Committed,
    ) -> BoxFuture<'a, Result<(), ToolError>> {
        Box::pin(async move { Err(not_a("write", &O::META)) })
    }
}

struct WriteAdapter<O> {
    op: O,
    schema: Value,
}

impl<C: Ctx, O: WriteOp<C>> DynOp<C> for WriteAdapter<O> {
    fn meta(&self) -> &'static OpMeta {
        &O::META
    }
    fn undo_support(&self) -> UndoSupport {
        O::UNDO
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn read<'a>(&'a self, _: &'a C, _: Value) -> BoxFuture<'a, Result<Value, ToolError>> {
        Box::pin(async move { Err(not_a("read", &O::META)) })
    }
    fn preview<'a>(
        &'a self,
        cx: &'a C,
        input: Value,
    ) -> BoxFuture<'a, Result<DynPlanned, ToolError>> {
        Box::pin(async move {
            let p = self.op.preview(cx, parse(input)?).await?;
            Ok(DynPlanned {
                plan: to_json(&p.plan)?,
                inner: Planned {
                    plan: (),
                    changes: p.changes,
                    checks: p.checks,
                    basis: p.basis,
                    approval: p.approval,
                },
            })
        })
    }
    fn commit<'a>(
        &'a self,
        tx: &'a mut C::Tx,
        plan: &'a Value,
    ) -> BoxFuture<'a, Result<Committed, ToolError>> {
        Box::pin(async move {
            let plan: O::Plan = parse(plan.clone())?;
            self.op.commit(tx, &plan).await
        })
    }
    fn verify<'a>(
        &'a self,
        cx: &'a C,
        plan: &'a Value,
        done: &'a Committed,
    ) -> BoxFuture<'a, Result<Verification, ToolError>> {
        Box::pin(async move {
            let plan: O::Plan = parse(plan.clone())?;
            self.op.verify(cx, &plan, done).await
        })
    }
    fn undo<'a>(
        &'a self,
        tx: &'a mut C::Tx,
        plan: &'a Value,
        done: &'a Committed,
    ) -> BoxFuture<'a, Result<(), ToolError>> {
        Box::pin(async move {
            let plan: O::Plan = parse(plan.clone())?;
            self.op.undo(tx, &plan, done).await
        })
    }
}

struct PersonAdapter<O> {
    op: O,
    schema: Value,
}

impl<C: Ctx, O: PersonOp<C>> DynOp<C> for PersonAdapter<O> {
    fn meta(&self) -> &'static OpMeta {
        &O::META
    }
    fn input_schema(&self) -> &Value {
        &self.schema
    }
    fn read<'a>(&'a self, _: &'a C, _: Value) -> BoxFuture<'a, Result<Value, ToolError>> {
        Box::pin(async move { Err(not_a("read", &O::META)) })
    }
    fn preview<'a>(&'a self, _: &'a C, _: Value) -> BoxFuture<'a, Result<DynPlanned, ToolError>> {
        Box::pin(async move { Err(not_a("write", &O::META)) })
    }
    fn commit<'a>(
        &'a self,
        _: &'a mut C::Tx,
        _: &'a Value,
    ) -> BoxFuture<'a, Result<Committed, ToolError>> {
        Box::pin(async move { Err(not_a("write", &O::META)) })
    }
    fn verify<'a>(
        &'a self,
        _: &'a C,
        _: &'a Value,
        _: &'a Committed,
    ) -> BoxFuture<'a, Result<Verification, ToolError>> {
        Box::pin(async move { Err(not_a("write", &O::META)) })
    }
    fn undo<'a>(
        &'a self,
        _: &'a mut C::Tx,
        _: &'a Value,
        _: &'a Committed,
    ) -> BoxFuture<'a, Result<(), ToolError>> {
        Box::pin(async move { Err(not_a("write", &O::META)) })
    }
    fn person<'a>(
        &'a self,
        tx: &'a mut C::Tx,
        input: Value,
    ) -> BoxFuture<'a, Result<Value, ToolError>> {
        Box::pin(async move {
            let out = self.op.run(tx, parse(input)?).await?;
            to_json(&out)
        })
    }
}

/// Collects operations and job handlers from every domain crate.
pub struct RegistryBuilder<C: Ctx> {
    ops: BTreeMap<&'static str, Arc<dyn DynOp<C>>>,
    jobs: BTreeMap<&'static str, Arc<dyn DynJob<C>>>,
    problems: Vec<String>,
}

impl<C: Ctx> Default for RegistryBuilder<C> {
    fn default() -> Self {
        Self {
            ops: BTreeMap::new(),
            jobs: BTreeMap::new(),
            problems: Vec::new(),
        }
    }
}

impl<C: Ctx> std::fmt::Debug for RegistryBuilder<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegistryBuilder")
            .field("ops", &self.ops.keys().collect::<Vec<_>>())
            .field("jobs", &self.jobs.keys().collect::<Vec<_>>())
            .finish()
    }
}

impl<C: Ctx> RegistryBuilder<C> {
    pub fn read<O: ReadOp<C>>(&mut self, op: O) -> &mut Self {
        self.check_kind::<O>(false);
        let schema = closed_schema_for::<O::Input>();
        self.insert(O::META.tool, Arc::new(ReadAdapter { op, schema }))
    }

    pub fn write<O: WriteOp<C>>(&mut self, op: O) -> &mut Self {
        self.check_kind::<O>(true);
        let schema = closed_schema_for::<O::Input>();
        self.insert(O::META.tool, Arc::new(WriteAdapter { op, schema }))
    }

    pub fn person<O: PersonOp<C>>(&mut self, op: O) -> &mut Self {
        if O::META.kind != Kind::Person || !O::META.person_scope.starts_with("human:") {
            self.problems.push(format!(
                "{}: person-only endpoints need Kind::Person and a human:* scope",
                O::META.tool
            ));
        }
        let schema = closed_schema_for::<O::Input>();
        self.insert(O::META.tool, Arc::new(PersonAdapter { op, schema }))
    }

    /// A handler for one kind of scheduled work (`platform.job.kind`).
    pub fn job<J: Job<C>>(&mut self, job: J) -> &mut Self {
        let kind = J::META.kind;
        if !kind_is_valid(kind) {
            self.problems.push(format!(
                "job {kind}: kinds are `domain.verb` in lowercase words"
            ));
        }
        if let Some(w) = J::META.watcher
            && kind != format!("watcher.{w}")
        {
            self.problems
                .push(format!("job {kind}: a watcher's kind is `watcher.{w}`"));
        }
        if self.jobs.insert(kind, Arc::new(JobAdapter(job))).is_some() {
            self.problems.push(format!("job {kind}: registered twice"));
        }
        self
    }

    fn check_kind<O: Operation>(&mut self, write: bool) {
        let ok = match O::META.kind {
            Kind::Read => !write,
            Kind::Write | Kind::WriteApproval => write,
            Kind::Person => false,
        };
        if !ok {
            self.problems
                .push(format!("{}: registered as the wrong kind", O::META.tool));
        }
    }

    fn insert(&mut self, tool: &'static str, op: Arc<dyn DynOp<C>>) -> &mut Self {
        if self.ops.insert(tool, op).is_some() {
            self.problems.push(format!("{tool}: registered twice"));
        }
        self
    }

    /// Fails when two operations share a tool name, a kind is wrong, or a schema is not closed.
    pub fn build(self) -> Result<Registry<C>, Vec<String>> {
        let mut problems = self.problems;
        for (tool, op) in &self.ops {
            for v in crate::schema::violations(op.input_schema()) {
                problems.push(format!("{tool}: {v}"));
            }
        }
        if problems.is_empty() {
            Ok(Registry {
                ops: self.ops,
                jobs: self.jobs,
            })
        } else {
            Err(problems)
        }
    }
}

pub struct Registry<C: Ctx> {
    ops: BTreeMap<&'static str, Arc<dyn DynOp<C>>>,
    jobs: BTreeMap<&'static str, Arc<dyn DynJob<C>>>,
}

impl<C: Ctx> std::fmt::Debug for Registry<C> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("ops", &self.ops.len())
            .field("jobs", &self.jobs.len())
            .finish()
    }
}

/// One entry of `GET /v1/tools`: everything an agent needs to find and call a tool.
#[derive(Debug, Clone, Serialize)]
pub struct ManifestEntry {
    #[serde(flatten)]
    pub meta: OpMeta,
    pub input_schema: Value,
}

impl<C: Ctx> Registry<C> {
    pub fn get(&self, tool: &str) -> Option<&Arc<dyn DynOp<C>>> {
        self.ops.get(tool)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Arc<dyn DynOp<C>>> {
        self.ops.values()
    }

    pub fn len(&self) -> usize {
        self.ops.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }

    /// The handler of a job kind.
    pub fn job(&self, kind: &str) -> Option<&Arc<dyn DynJob<C>>> {
        self.jobs.get(kind)
    }

    /// Every registered job kind, in name order.
    pub fn jobs(&self) -> impl Iterator<Item = &'static JobMeta> + '_ {
        self.jobs.values().map(|j| j.meta())
    }

    pub fn manifest(&self, visible: impl Fn(&OpMeta) -> bool) -> Vec<ManifestEntry> {
        self.ops
            .values()
            .filter(|op| visible(op.meta()))
            .map(|op| ManifestEntry {
                meta: *op.meta(),
                input_schema: op.input_schema().clone(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::ReadResult;
    use crate::meta::{ActionType, Audience, Group, Method, Slot};
    use crate::op::{ApprovalNeed, UndoSupport};
    use schemars::JsonSchema;
    use serde::Deserialize;

    struct TestCx;
    impl Ctx for TestCx {
        type Tx = Vec<String>;
    }

    #[derive(Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct SearchIn {
        /// Words to look for.
        query: String,
    }

    struct Search;
    impl Operation for Search {
        const META: OpMeta = OpMeta {
            address: "search.product.search",
            tool: "search_products",
            group: Group::Offer,
            kind: Kind::Read,
            method: Method::Get,
            path: "/v1/search/products",
            summary: "Find products by words and filters.",
            consumes: &[],
            produces: &[Slot::many("product_id")],
            checks: &[],
            errors: &[],
            audience: Audience::Both,
            action_type: ActionType::SEARCH,
            person_scope: "",
        };
        type Input = SearchIn;
        type Output = ReadResult<String>;
    }
    impl ReadOp<TestCx> for Search {
        async fn read(&self, _: &TestCx, input: SearchIn) -> Result<ReadResult<String>, ToolError> {
            Ok(ReadResult::all(vec![format!("prd_{}", input.query)]))
        }
    }

    #[derive(Deserialize, JsonSchema)]
    #[serde(deny_unknown_fields)]
    struct NoteIn {
        /// Text to remember.
        text: String,
    }

    struct Remember;
    impl Operation for Remember {
        const META: OpMeta = OpMeta {
            address: "customer.preference.update_my_preferences",
            tool: "update_my_preferences",
            group: Group::Engage,
            kind: Kind::Write,
            method: Method::Post,
            path: "/v1/customer/me/preferences",
            summary: "Remember what the shopper said they like.",
            consumes: &[],
            produces: &[Slot::one("memory_id")],
            checks: &[],
            errors: &[],
            audience: Audience::Shopper,
            action_type: ActionType::UPDATE,
            person_scope: "",
        };
        type Input = NoteIn;
        type Output = ();
    }
    impl WriteOp<TestCx> for Remember {
        type Plan = String;
        const UNDO: UndoSupport = UndoSupport::Supported;
        async fn preview(&self, _: &TestCx, input: NoteIn) -> Result<Planned<String>, ToolError> {
            Ok(Planned {
                plan: input.text,
                changes: vec![],
                checks: vec![],
                basis: vec![],
                approval: ApprovalNeed::NotNeeded,
            })
        }
        async fn commit(
            &self,
            tx: &mut Vec<String>,
            plan: &String,
        ) -> Result<Committed, ToolError> {
            tx.push(plan.clone());
            Ok(Committed {
                id: "mem_1".into(),
                produced: Default::default(),
            })
        }
        async fn verify(
            &self,
            _: &TestCx,
            _: &String,
            _: &Committed,
        ) -> Result<Verification, ToolError> {
            Ok(Verification {
                matched: true,
                mismatches: vec![],
            })
        }
        async fn undo(
            &self,
            tx: &mut Vec<String>,
            _: &String,
            _: &Committed,
        ) -> Result<(), ToolError> {
            tx.pop();
            Ok(())
        }
    }

    struct Tick;
    impl crate::job::Job<TestCx> for Tick {
        const META: JobMeta = JobMeta {
            kind: "watcher.ticks",
            summary: "Counts runs.",
            watcher: Some("ticks"),
        };
        async fn run(
            &self,
            tx: &mut Vec<String>,
            payload: &Value,
        ) -> Result<crate::job::JobOutcome, ToolError> {
            tx.push(payload.to_string());
            Ok(crate::job::JobOutcome::Watched(Vec::new()))
        }
    }

    struct Misnamed;
    impl crate::job::Job<TestCx> for Misnamed {
        const META: JobMeta = JobMeta {
            kind: "watcher.other",
            summary: "Kind and watcher key disagree.",
            watcher: Some("ticks"),
        };
        async fn run(
            &self,
            _: &mut Vec<String>,
            _: &Value,
        ) -> Result<crate::job::JobOutcome, ToolError> {
            Ok(crate::job::JobOutcome::done(Value::Null))
        }
    }

    fn registry() -> Registry<TestCx> {
        let mut b = RegistryBuilder::default();
        b.read(Search).write(Remember).job(Tick);
        b.build().unwrap()
    }

    #[tokio::test]
    async fn runs_job_handlers_by_kind() {
        let r = registry();
        assert_eq!(
            r.jobs().map(|m| m.kind).collect::<Vec<_>>(),
            ["watcher.ticks"]
        );
        let mut tx = Vec::new();
        let out = r
            .job("watcher.ticks")
            .unwrap()
            .run(&mut tx, &serde_json::json!({"n": 1}))
            .await
            .unwrap();
        assert_eq!(out, crate::job::JobOutcome::Watched(Vec::new()));
        assert_eq!(tx, vec![r#"{"n":1}"#]);
        assert!(r.job("promotion.start").is_none());
    }

    #[test]
    fn refuses_duplicate_and_misnamed_jobs() {
        let mut b: RegistryBuilder<TestCx> = RegistryBuilder::default();
        b.job(Tick).job(Tick).job(Misnamed);
        let problems = b.build().unwrap_err();
        assert!(problems.iter().any(|p| p.contains("registered twice")));
        assert!(problems.iter().any(|p| p.contains("watcher.other")));
    }

    #[tokio::test]
    async fn runs_reads_and_writes_through_json() {
        let r = registry();
        let out = r
            .get("search_products")
            .unwrap()
            .read(&TestCx, serde_json::json!({"query": "parka"}))
            .await
            .unwrap();
        assert_eq!(out["items"][0], "prd_parka");

        let w = r.get("update_my_preferences").unwrap();
        let planned = w
            .preview(&TestCx, serde_json::json!({"text": "packs small"}))
            .await
            .unwrap();
        let mut tx = Vec::new();
        let done = w.commit(&mut tx, &planned.plan).await.unwrap();
        assert_eq!(tx, vec!["packs small"]);
        w.undo(&mut tx, &planned.plan, &done).await.unwrap();
        assert!(tx.is_empty());
    }

    #[tokio::test]
    async fn rejects_unknown_fields_and_wrong_calls() {
        let r = registry();
        let e = r
            .get("search_products")
            .unwrap()
            .read(&TestCx, serde_json::json!({"q": "x"}))
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
        let e = r
            .get("search_products")
            .unwrap()
            .preview(&TestCx, serde_json::json!({}))
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
    }

    #[test]
    fn manifest_carries_closed_schemas() {
        let m = registry().manifest(|meta| meta.audience != Audience::Internal);
        assert_eq!(m.len(), 2);
        let search = m.iter().find(|e| e.meta.tool == "search_products").unwrap();
        assert_eq!(search.input_schema["additionalProperties"], false);
    }
}
