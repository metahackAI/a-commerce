//! The `messaging` domain: customer messages, templates, campaigns, notifications.
//!
//! Operations, inputs, outputs, checks, and errors are specified in `docs/api/messaging.md`.
//! Layout: the folders follow the API hierarchy `messaging.{resource}.{operation}`, one folder
//! per resource (`message`, `template`, `notification`) and one file per operation, named after
//! its tool (`message/send_customer_message.rs`). Domain-wide: `model` (pure rules: quiet hours,
//! reachability, subjects, template variables), `repo` (SQL). Other domains are reached only through their `service` modules, never
//! through their tables.

pub mod model;
pub mod service;

mod message;
mod notification;
mod repo;
mod template;
#[cfg(test)]
mod tests;

use acom_contract::{RegistryBuilder, ToolError};
use acom_store::Cx;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};

/// Adds every operation of this domain to the registry.
pub fn register(b: &mut RegistryBuilder<Cx>) {
    b.write(message::SendCustomerMessage)
        .write(template::CreateMessageTemplate)
        .read(template::ListMessageTemplates)
        .read(notification::ListNotifications);
}

/// A time filter in store time.
pub(crate) fn parse_time(
    cx: &Cx,
    field: &str,
    s: Option<&str>,
) -> Result<Option<DateTime<Utc>>, ToolError> {
    s.map(|s| {
        cx.time.parse(s).map_err(|e| {
            ToolError::new(template::INVALID_FILTER, e.to_string())
                .detail(json!({"field": field, "reason": "format"}))
        })
    })
    .transpose()
}

/// Keeps only the named top-level fields (all when `fields` is empty); `id` always stays.
pub(crate) fn select(v: Value, fields: &[String]) -> Value {
    if fields.is_empty() {
        return v;
    }
    match v {
        Value::Object(m) => Value::Object(
            m.into_iter()
                .filter(|(k, _)| k == "id" || fields.iter().any(|f| f == k))
                .collect(),
        ),
        other => other,
    }
}
