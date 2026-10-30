//! The contract every A-Commerce operation follows.
//!
//! One operation is defined once (metadata + typed input/output + handlers). From that
//! definition the registry produces the REST routes, the MCP tools, the agent's tool
//! manifest, and the OpenAPI document. Specs: `docs/api/README.md`, `docs/backend/commerce.md`.

pub mod envelope;
pub mod error;
pub mod meta;
pub mod op;
pub mod registry;
pub mod schema;

pub use envelope::{Change, ChangeOp, Check, CommitResult, Preview, ReadResult, Undo};
pub use error::{ErrorClass, ErrorCode, ToolError};
pub use meta::{ActionType, Audience, Group, Kind, Method, OpMeta, Slot};
pub use op::{
    ApprovalNeed, Basis, Committed, Ctx, Operation, Planned, ReadOp, UndoSupport, Verification,
    WriteOp,
};
pub use registry::{DynOp, ManifestEntry, Registry, RegistryBuilder};
