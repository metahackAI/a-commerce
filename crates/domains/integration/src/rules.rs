//! The integration rules, read from `policies/integration.yml` (with the store's overrides).
//! Business numbers live there; this file only reads them.

use acom_contract::{ErrorCode, ToolError};
use acom_policy::PolicySet;
use serde_json::Value;

fn missing(path: &str) -> ToolError {
    ToolError::new(
        ErrorCode::INTERNAL,
        format!("The store's policy has no `{path}`."),
    )
}

fn at<'a>(p: &'a PolicySet, path: &str) -> Result<&'a Value, ToolError> {
    p.lookup(path).ok_or_else(|| missing(path))
}

fn word<'a>(p: &'a PolicySet, path: &str) -> Result<&'a str, ToolError> {
    at(p, path)?.as_str().ok_or_else(|| missing(path))
}

fn flag(p: &PolicySet, path: &str) -> Result<bool, ToolError> {
    at(p, path)?.as_bool().ok_or_else(|| missing(path))
}

fn count(p: &PolicySet, path: &str) -> Result<i64, ToolError> {
    at(p, path)?
        .as_i64()
        .filter(|n| *n >= 0)
        .ok_or_else(|| missing(path))
}

fn words(p: &PolicySet, path: &str) -> Result<Vec<String>, ToolError> {
    at(p, path)?
        .as_array()
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .ok_or_else(|| missing(path))
}

/// `import`: how `import_products` takes an outside feed.
#[derive(Debug, Clone, PartialEq)]
pub struct ImportRules {
    /// `import.max_items_per_batch`.
    pub max_items: usize,
    /// `import.partial_success: all_or_nothing` refuses the batch when a row is rejected;
    /// `report_per_item` takes the rows that pass and reports the others.
    pub all_or_nothing: bool,
    /// `import.required_fields` (`name`, `price`, `category`, `size`).
    pub required_fields: Vec<String>,
    /// `import.category_mapping: required`: a row whose category maps to no standard category is
    /// rejected.
    pub category_required: bool,
}

impl ImportRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        Ok(Self {
            max_items: usize::try_from(count(p, "integration.import.max_items_per_batch")?)
                .unwrap_or(usize::MAX),
            all_or_nothing: word(p, "integration.import.partial_success")? == "all_or_nothing",
            required_fields: words(p, "integration.import.required_fields")?,
            category_required: word(p, "integration.import.category_mapping")? == "required",
        })
    }

    pub fn requires(&self, field: &str) -> bool {
        self.required_fields.iter().any(|f| f == field)
    }
}

/// `status_mapping`: how a channel's status words become ours.
#[derive(Debug, Clone, PartialEq)]
pub struct MappingRules {
    /// `status_mapping.unmapped: reject`: a word the mapping does not name is refused
    /// (`status_unmapped`); otherwise it is recorded and nothing moves.
    pub reject_unmapped: bool,
    /// `status_mapping.must_cover`: the parts a channel's mapping must have (`order`, `claim`).
    pub must_cover: Vec<String>,
    /// `status_mapping.transitions_from_core`: a mapped status still has to be a move the core
    /// domain allows.
    pub transitions_from_core: bool,
}

impl MappingRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        Ok(Self {
            reject_unmapped: word(p, "integration.status_mapping.unmapped")? == "reject",
            must_cover: words(p, "integration.status_mapping.must_cover")?,
            transitions_from_core: flag(p, "integration.status_mapping.transitions_from_core")?,
        })
    }
}

/// `channel`: registering an outside system.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelRules {
    /// `channel.credential_vaulting: required`: credentials come as references to a secret
    /// store and are kept only as hashes.
    pub vaulting_required: bool,
    /// `channel.connection_test_required`.
    pub connection_test_required: bool,
}

impl ChannelRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        Ok(Self {
            vaulting_required: word(p, "integration.channel.credential_vaulting")? == "required",
            connection_test_required: flag(p, "integration.channel.connection_test_required")?,
        })
    }
}

/// `events`: receiving what carriers and payment providers send.
#[derive(Debug, Clone, PartialEq)]
pub struct EventRules {
    /// `events.signature_required`.
    pub signature_required: bool,
    /// `events.replay_window.minutes`: an event that happened longer ago than this when it
    /// arrives is kept, not applied.
    pub replay_window_minutes: i64,
    /// `events.pending.retry_every_minutes`: how often an event waiting for its parcel, payment
    /// or refund is tried again.
    pub retry_every_minutes: i64,
    /// `events.pending.give_up_after_minutes`: when a waiting event stops being tried.
    pub give_up_after_minutes: i64,
}

impl EventRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        Ok(Self {
            signature_required: flag(p, "integration.events.signature_required")?,
            replay_window_minutes: count(p, "integration.events.replay_window.minutes")?,
            retry_every_minutes: count(p, "integration.events.pending.retry_every_minutes")?.max(1),
            give_up_after_minutes: count(p, "integration.events.pending.give_up_after_minutes")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_rules_read() {
        let p = crate::testing::policies();
        let i = ImportRules::from_policy(&p).unwrap();
        assert_eq!(i.max_items, 500);
        assert!(!i.all_or_nothing);
        assert!(i.requires("size") && i.category_required);
        let m = MappingRules::from_policy(&p).unwrap();
        assert!(m.reject_unmapped && m.transitions_from_core);
        assert_eq!(m.must_cover, ["order", "claim"]);
        let c = ChannelRules::from_policy(&p).unwrap();
        assert!(c.vaulting_required && c.connection_test_required);
        let e = EventRules::from_policy(&p).unwrap();
        assert!(e.signature_required);
        assert_eq!(e.replay_window_minutes, 15);
        assert!(e.retry_every_minutes >= 1 && e.give_up_after_minutes > e.retry_every_minutes);
    }
}
