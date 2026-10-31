//! What the order and claim resources share: a channel's copy as reads show it, and how a
//! channel's status word becomes ours (`status_mapping.unmapped`) and whether the move is one
//! the core domain allows (`status_mapping.transitions_from_core`).

use acom_contract::{Check, ToolError};
use acom_core::StoreTime;
use acom_core::enums::ChannelType;
use serde_json::{Value, json};

use crate::repo::{ChannelRow, CopyKind, ExternalRow};
use crate::rules::MappingRules;
use crate::{EXTERNAL_LOOKUP_FAILED, STATUS_UNMAPPED, not_connected};

/// The part of a channel's mapping a copy uses.
pub(crate) fn part(kind: CopyKind) -> &'static str {
    match kind {
        CopyKind::Order => "order",
        CopyKind::Claim => "claim",
    }
}

/// A channel that holds orders and claims (a marketplace or an outside warehouse), reachable.
pub(crate) fn holds_orders(c: &ChannelRow, next: &'static str) -> Result<(), ToolError> {
    let kind: ChannelType = c
        .channel_type
        .parse()
        .map_err(|e: acom_core::enums::UnknownWord| ToolError::invalid_input(e.to_string()))?;
    if !matches!(kind, ChannelType::Marketplace | ChannelType::Fulfillment) {
        return Err(ToolError::invalid_input(format!(
            "{} is a {}; orders and claims come from marketplaces and outside warehouses.",
            c.id, c.channel_type
        ))
        .detail(json!({"field": "channel_id", "reason": "enum",
                       "allowed": ["marketplace", "fulfillment"]}))
        .next("list_channels"));
    }
    if c.connection_status == "disconnected" {
        return Err(ToolError::new(
            EXTERNAL_LOOKUP_FAILED,
            format!(
                "{} is disconnected; its orders and claims cannot be read until it is connected again.",
                c.id
            ),
        )
        .detail(json!({"upstream": c.id, "retryable": true}))
        .next(next));
    }
    Ok(())
}

/// A channel's order or claim as the lookups show it.
pub(crate) fn copy_json(
    c: &ChannelRow,
    r: &ExternalRow,
    kind: CopyKind,
    time: &StoreTime,
) -> Value {
    let mapped = c.maps(part(kind), &r.external_status);
    match kind {
        CopyKind::Order => json!({
            "external_order_ref": r.reference,
            "channel_id": r.channel_id,
            "order_date": time.format(r.at),
            "ordered_item": r.ordered_item,
            "acom:external_status": r.external_status,
            "acom:order_status": mapped,
            "acom:shippable": mapped.as_deref() == Some("processing"),
            "order_id": r.mapped_id,
            "acom:last_synced_at": time.format(r.last_synced_at),
        }),
        CopyKind::Claim => json!({
            "external_claim_ref": r.reference,
            "external_order_ref": r.order_reference,
            "channel_id": r.channel_id,
            "claim_type": r.claim_type,
            "requested_at": time.format(r.at),
            "ordered_item": r.ordered_item,
            "acom:external_status": r.external_status,
            "acom:claim_status": mapped,
            "claim_id": r.mapped_id,
            "acom:last_synced_at": time.format(r.last_synced_at),
        }),
    }
}

/// What a lookup answer says about where its rows come from.
pub(crate) fn lookup_note(c: &ChannelRow, what: &str, time: &StoreTime) -> Value {
    let when = c
        .last_synced_at
        .map_or_else(|| "never".to_owned(), |t| time.format(t));
    not_connected(
        &c.id,
        &format!(
            "these are the {what} it last sent (last sync {when}); nothing was asked of it now"
        ),
    )
}

/// A channel's word in our words: `Ok(Some(word))`, or `Ok(None)` when the mapping does not name
/// it and the store lets such words through unchanged; `status_unmapped` when it refuses them.
pub(crate) fn map_status(
    rules: &MappingRules,
    c: &ChannelRow,
    kind: CopyKind,
    external: &str,
) -> Result<(Option<String>, Check), ToolError> {
    let p = part(kind);
    match c.maps(p, external) {
        Some(word) => Ok((
            Some(word.clone()),
            Check {
                rule: "status_mapping.unmapped".into(),
                passed: true,
                detail: Some(format!("{}'s `{external}` is {word}", c.name)),
                fix_tool: None,
            },
        )),
        None if rules.reject_unmapped => Err(ToolError::new(
            STATUS_UNMAPPED,
            format!(
                "{} has no {p} status mapped for `{external}`; add it to the channel's mapping.",
                c.name
            ),
        )
        .detail(
            json!({"rule": "status_mapping.unmapped", "channel_id": c.id, "part": p,
                       "external_status": external}),
        )
        .next("register_channel")),
        None => Ok((
            None,
            Check {
                rule: "status_mapping.unmapped".into(),
                passed: true,
                detail: Some(format!(
                    "`{external}` is not mapped; the store records it and moves nothing"
                )),
                fix_tool: None,
            },
        )),
    }
}

/// The `status_mapping.transitions_from_core` check for a move the core domain allows, or for
/// nothing to move. The core domain decides the move whatever the policy says (its service
/// refuses any other); the policy value only shows in the sentence.
pub(crate) fn transition_check(
    rules: &MappingRules,
    owner: &str,
    from: &str,
    to: Option<&str>,
) -> Check {
    let mut detail = match to {
        Some(to) => format!(
            "{} → {} is a move {owner} allows",
            from.replace('_', " "),
            to.replace('_', " ")
        ),
        None => format!("Already {}; nothing moves", from.replace('_', " ")),
    };
    if !rules.transitions_from_core {
        detail.push_str(&format!(" ({owner} decides every move)"));
    }
    Check {
        rule: "status_mapping.transitions_from_core".into(),
        passed: true,
        detail: Some(detail),
        fix_tool: None,
    }
}
