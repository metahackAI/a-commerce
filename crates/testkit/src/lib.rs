//! Test support: load the demo store (`seed/demo/`), set the demo clock to a scene, and
//! replay a scenario's tool calls over HTTP. Scenarios: `docs/scenarios.md`.

pub mod generate;
pub mod load;
pub mod named;

/// Seed rows in load order: the table (`schema.table`) and the row, a JSON object whose keys
/// are column names. A missing key takes the column default.
pub type SeedRows = Vec<(String, serde_json::Value)>;
