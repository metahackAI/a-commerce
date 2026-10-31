//! The contract every A-Commerce operation follows.
//!
//! One operation is defined once (metadata + typed input/output + handlers). From that
//! definition the registry produces the REST routes, the MCP tools, the agent's tool
//! manifest, and the OpenAPI document. Specs: `docs/api/README.md`, `docs/backend/commerce.md`.

pub mod envelope;
pub mod error;
pub mod fields;
pub mod job;
pub mod meta;
pub mod op;
pub mod registry;
pub mod schema;

pub use envelope::{Change, ChangeOp, Check, CommitResult, Preview, ReadResult, Undo};
pub use error::{ErrorClass, ErrorCode, ToolError};
pub use job::{
    DomainEvent, DynJob, EntityRef, FixCall, FixDraft, Job, JobMeta, JobOutcome, TaskDraft,
};
pub use meta::{ActionType, Audience, Group, Kind, Method, OpMeta, Slot};
pub use op::{
    ApprovalNeed, Basis, Committed, Ctx, Operation, PersonOp, Planned, ReadOp, UndoSupport,
    Verification, WriteOp,
};
pub use registry::{DynOp, DynPlanned, ManifestEntry, Registry, RegistryBuilder};
