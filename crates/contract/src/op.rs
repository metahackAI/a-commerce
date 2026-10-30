//! Typed operation traits. Domain crates implement these; the registry erases the types.
//!
//! The context type is a parameter so this crate stays free of I/O: `acom-store` defines the
//! real context (tenant, principal, clock, policy snapshot, database pool).

use std::future::Future;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use crate::envelope::{Change, Check};
use crate::error::ToolError;
use crate::meta::OpMeta;

/// What an operation runs against.
pub trait Ctx: Send + Sync + 'static {
    /// An open transaction scoped to one store.
    type Tx: Send;
}

pub trait Operation: Send + Sync + 'static {
    const META: OpMeta;
    /// Must produce a closed schema (see `schema::close`).
    type Input: DeserializeOwned + JsonSchema + Send;
    type Output: Serialize + JsonSchema + Send;
}

pub trait ReadOp<C: Ctx>: Operation {
    fn read(
        &self,
        cx: &C,
        input: Self::Input,
    ) -> impl Future<Output = Result<Self::Output, ToolError>> + Send;
}

/// Rows a preview read, with the version it saw. A run re-checks them before it commits.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Basis {
    /// `schema.table`, for example `catalog.offer`.
    pub entity: String,
    pub id: String,
    pub version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "needs", rename_all = "snake_case")]
pub enum ApprovalNeed {
    NotNeeded,
    Needed { reason: String },
}

/// What a preview computes. `plan` is stored and later run as is.
#[derive(Debug, Clone, PartialEq)]
pub struct Planned<P> {
    pub plan: P,
    pub changes: Vec<Change>,
    pub checks: Vec<Check>,
    pub basis: Vec<Basis>,
    pub approval: ApprovalNeed,
}

/// What a run produced: the main id and the slots later steps can consume.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Committed {
    pub id: String,
    /// Slot name → value, for example `promotion_id` → `"promo_1042"`.
    pub produced: serde_json::Map<String, Value>,
}

/// Read-back result for one write.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Verification {
    pub matched: bool,
    /// One line per mismatch, empty when everything matched.
    pub mismatches: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UndoSupport {
    Supported,
    /// Messages sent and money paid out cannot be taken back.
    NotSupported,
}

pub trait WriteOp<C: Ctx>: Operation {
    type Plan: Serialize + DeserializeOwned + Send + Sync;
    const UNDO: UndoSupport;

    /// Must not change state.
    fn preview(
        &self,
        cx: &C,
        input: Self::Input,
    ) -> impl Future<Output = Result<Planned<Self::Plan>, ToolError>> + Send;

    /// Applies the plan inside the transaction. Must be idempotent for one preview.
    fn commit(
        &self,
        tx: &mut C::Tx,
        plan: &Self::Plan,
    ) -> impl Future<Output = Result<Committed, ToolError>> + Send;

    fn verify(
        &self,
        cx: &C,
        plan: &Self::Plan,
        done: &Committed,
    ) -> impl Future<Output = Result<Verification, ToolError>> + Send;

    fn undo(
        &self,
        tx: &mut C::Tx,
        plan: &Self::Plan,
        done: &Committed,
    ) -> impl Future<Output = Result<(), ToolError>> + Send;
}

/// A person-only endpoint. Runs directly in one transaction, without a preview; the caller must
/// hold the `human:*` scope named in its metadata. Never offered to a model.
pub trait PersonOp<C: Ctx>: Operation {
    fn run(
        &self,
        tx: &mut C::Tx,
        input: Self::Input,
    ) -> impl Future<Output = Result<Self::Output, ToolError>> + Send;
}
