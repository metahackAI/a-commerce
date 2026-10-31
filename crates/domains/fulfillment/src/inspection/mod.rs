//! The `inspection` resource (docs/api/fulfillment.md `## inspection`): the condition check of
//! items that came back. Its result drives the claim's status, cost and stock restore.

pub(crate) mod assess_return_condition;

pub use assess_return_condition::AssessReturnCondition;
