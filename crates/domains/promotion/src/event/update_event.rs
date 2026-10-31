//! `update_event` (docs/api/promotion.md#event): change an event's fields, or move it to
//! `scheduled` (from draft, or back from paused), `paused` or `cancelled`. Once someone entered,
//! nothing may take something away from them (`changes_after_entry`): the mechanic and method
//! stay, the period may not shrink, eligibility may only widen, rewards may only grow.

use std::collections::BTreeSet;

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::enums::EventStatus;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::create_event::write_fields;
use super::{
    CHANGES_AFTER_ENTRY, DisplayPeriod, Eligibility, EntryLimit, EventDraft, EventSchedule,
    RewardTier, check_draft, entry_counts, event, event_time, known_event, set_event_status,
};

/// The status to move the event to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EventMove {
    /// Ready: it opens and closes with its entry period (from draft, or back from paused).
    Scheduled,
    /// Stop taking entries for now (from scheduled or running).
    Paused,
    /// Called off for good (before any draw).
    Cancelled,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateEventInput {
    /// The event.
    #[schemars(regex(pattern = r"^evt_[a-z0-9_]+$"), example = &"evt_0311")]
    pub event_id: String,
    /// scheduled: ready (from draft, or resume from paused); paused: stop entries for now;
    /// cancelled: call it off.
    #[schemars(example = &"scheduled")]
    pub status: Option<EventMove>,
    /// New name.
    #[schemars(length(min = 1, max = 100), example = &"Fall check-in week")]
    pub name: Option<String>,
    /// New first moment entries are taken.
    #[schemars(example = &"2026-11-09T00:00")]
    pub start_date: Option<String>,
    /// New last moment entries are taken.
    #[schemars(example = &"2026-11-16T23:59")]
    pub end_date: Option<String>,
    /// New display period.
    #[schemars(example = json!({"valid_from": "2026-11-06T00:00", "valid_through": "2026-11-22T23:59"}))]
    pub display_period: Option<DisplayPeriod>,
    /// New entry mechanic (only before anyone entered).
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"click_apply")]
    pub entry_mechanic: Option<String>,
    /// New selection method (only before anyone entered).
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"random_draw")]
    pub selection_method: Option<String>,
    /// New fields an entry sends, `{field: type}`.
    #[schemars(example = json!({"answer": "Text"}))]
    pub entry_payload_schema: Option<Value>,
    /// New reward ranks, replacing the current ones.
    #[schemars(length(min = 1, max = 20), example = json!([{"rank": 1, "reward_type": "product", "reward_ref": "prd_wool_overcoat", "reward_value": {"value": 289.00, "currency": "USD"}, "winner_count": 2}]))]
    pub reward_tiers: Option<Vec<RewardTier>>,
    /// New eligibility, replacing the current one.
    #[schemars(example = json!({"valid_for_member_tier": "everyone"}))]
    pub eligibility: Option<Eligibility>,
    /// New entry limit.
    #[schemars(example = json!({"count": 2, "period": "day"}))]
    pub entry_limit: Option<EntryLimit>,
    /// New draw, announcement and dispatch times (each one given replaces the current).
    #[schemars(example = json!({"draw_at": "2026-11-17T10:00"}))]
    pub schedule: Option<EventSchedule>,
    /// New compliance jurisdictions.
    #[schemars(inner(regex(pattern = r"^[a-z][a-z0-9_]*$")), example = json!(["us_ny"]))]
    pub compliance_jurisdictions: Option<Vec<String>>,
}

/// What a run of `update_event` writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventUpdateRun {
    pub event_id: String,
    pub before: EventDraft,
    pub after: EventDraft,
    pub stored_from: String,
    pub stored_to: String,
}

pub struct UpdateEvent;

impl Operation for UpdateEvent {
    const META: OpMeta = OpMeta {
        address: "promotion.event.update",
        tool: "update_event",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Patch,
        path: "/v1/promotion/events/{event_id}",
        summary: "Change an event or move it to scheduled, paused or cancelled. After the first entry nothing may be taken away from entrants: eligibility may only widen and rewards only grow.",
        consumes: &[Slot::one("event_id")],
        produces: &[Slot::one("event_id")],
        checks: &["event.schedule", "event.eligibility"],
        errors: &[
            ErrorCode::INVALID_STATE,
            CHANGES_AFTER_ENTRY,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UpdateEventInput;
    type Output = Value;
}

/// What a change takes away from people who already entered, if anything.
pub(crate) fn takes_away(before: &EventDraft, after: &EventDraft) -> Vec<String> {
    let mut out = Vec::new();
    if before.entry_mechanic != after.entry_mechanic
        || before.selection_method != after.selection_method
    {
        out.push("changes how entries are made or decided".to_owned());
    }
    if after.start_date > before.start_date || after.end_date < before.end_date {
        out.push("shortens the entry period".to_owned());
    }
    let (b, a) = (&before.eligibility, &after.eligibility);
    if a.valid_for_member_tier.is_some() && a.valid_for_member_tier != b.valid_for_member_tier {
        out.push("limits it to a tier".to_owned());
    }
    if a.segment_id.is_some() && a.segment_id != b.segment_id {
        out.push("limits it to a segment".to_owned());
    }
    let narrows = |old: Option<Vec<String>>, new: Option<Vec<String>>| match (old, new) {
        (_, None) => false,
        (None, Some(_)) => true,
        (Some(o), Some(n)) => !o.iter().all(|x| n.contains(x)),
    };
    let words = |l: &Option<Vec<acom_core::enums::MemberType>>| {
        l.as_ref()
            .map(|l| l.iter().map(|x| x.as_str().to_owned()).collect())
    };
    let chans = |l: &Option<Vec<acom_core::enums::SalesChannel>>| {
        l.as_ref()
            .map(|l| l.iter().map(|x| x.as_str().to_owned()).collect())
    };
    if narrows(words(&b.member_types), words(&a.member_types)) {
        out.push("limits the member types".to_owned());
    }
    if narrows(chans(&b.channels), chans(&a.channels)) {
        out.push("limits the channels".to_owned());
    }
    let min = |e: &Eligibility| {
        e.purchase_condition
            .as_ref()
            .and_then(|p| p.min_subtotal)
            .map(|m| m.value)
    };
    if a.purchase_condition.is_some() && (b.purchase_condition.is_none() || min(a) > min(b)) {
        out.push("asks more of the purchase".to_owned());
    }
    for old in &before.reward_tiers {
        match after.reward_tiers.iter().find(|t| t.rank == old.rank) {
            None => out.push(format!("removes rank {}", old.rank)),
            Some(new) => {
                if new.reward_type != old.reward_type
                    || new.winner_count < old.winner_count
                    || new.reward_value.map(|v| v.value) < old.reward_value.map(|v| v.value)
                {
                    out.push(format!("reduces the rank {} reward", old.rank));
                }
            }
        }
    }
    let unlimited = |l: &EntryLimit| l.period == super::LimitPeriod::Unlimited;
    if !unlimited(&before.entry_limit)
        && (after.entry_limit.period != before.entry_limit.period && !unlimited(&after.entry_limit)
            || after.entry_limit.count < before.entry_limit.count)
    {
        out.push("lowers the entry limit".to_owned());
    }
    out
}

impl WriteOp<Cx> for UpdateEvent {
    type Plan = EventUpdateRun;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UpdateEventInput,
    ) -> Result<Planned<EventUpdateRun>, ToolError> {
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let e = known_event(&mut tx, &input.event_id, "search_events").await?;
        let status = e.status(now);
        let before = EventDraft::from_row(&e);
        let mut after = before.clone();
        let t = |field: &str, s: &str| event_time(&cx.time, field, s, "update_event");
        if let Some(n) = &input.name {
            after.name = n.trim().to_owned();
        }
        if let Some(s) = &input.start_date {
            after.start_date = t("start_date", s)?;
        }
        if let Some(s) = &input.end_date {
            after.end_date = t("end_date", s)?;
        }
        if let Some(p) = &input.display_period {
            after.display_valid_from = Some(t("display_period.valid_from", &p.valid_from)?);
            after.display_valid_through =
                Some(t("display_period.valid_through", &p.valid_through)?);
        }
        if let Some(m) = &input.entry_mechanic {
            after.entry_mechanic = m.clone();
        }
        if let Some(m) = &input.selection_method {
            after.selection_method = m.clone();
        }
        if input.entry_payload_schema.is_some() {
            after.entry_payload_schema = input.entry_payload_schema.clone();
        }
        if let Some(r) = &input.reward_tiers {
            after.reward_tiers = r.clone();
            after.reward_tiers.sort_by_key(|r| r.rank);
        }
        if let Some(el) = &input.eligibility {
            after.eligibility = el.clone();
            if after.eligibility.valid_for_member_tier.as_deref() == Some("everyone") {
                after.eligibility.valid_for_member_tier = None;
            }
        }
        if let Some(l) = input.entry_limit {
            after.entry_limit = l;
        }
        if let Some(s) = &input.schedule {
            if let Some(x) = &s.draw_at {
                after.draw_at = Some(t("schedule.draw_at", x)?);
            }
            if let Some(x) = &s.announce_at {
                after.announce_at = Some(t("schedule.announce_at", x)?);
            }
            if let Some(x) = &s.dispatch_at {
                after.dispatch_at = Some(t("schedule.dispatch_at", x)?);
            }
        }
        if let Some(j) = &input.compliance_jurisdictions {
            after.compliance_jurisdictions = j.clone();
        }
        let fields_change = after != before;
        if !fields_change && input.status.is_none() {
            return Err(ToolError::invalid_input(
                "Say what to change: a status or a field of the event.",
            )
            .detail(json!({"field": "status", "reason": "missing"})));
        }
        let refuse = |why: String, allowed: &[&str]| {
            ToolError::new(ErrorCode::INVALID_STATE, why)
                .detail(json!({"entity": "event", "from": status, "allowed_from": allowed}))
                .next("get_event")
        };
        let stored_to = match input.status {
            None => e.event_status.clone(),
            Some(m) => {
                let (to, allowed): (&str, &[EventStatus]) = match m {
                    EventMove::Scheduled => {
                        ("scheduled", &[EventStatus::Draft, EventStatus::Paused])
                    }
                    EventMove::Paused => {
                        ("paused", &[EventStatus::Scheduled, EventStatus::Running])
                    }
                    EventMove::Cancelled => (
                        "cancelled",
                        &[
                            EventStatus::Draft,
                            EventStatus::Scheduled,
                            EventStatus::Running,
                            EventStatus::Paused,
                            EventStatus::Closed,
                        ],
                    ),
                };
                let already =
                    status.as_str() == to || (to == "scheduled" && status == EventStatus::Running);
                if !already && !allowed.contains(&status) {
                    return Err(refuse(
                        format!("{} is {status}, so it cannot become {to}.", e.name),
                        &allowed.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
                    ));
                }
                if already {
                    e.event_status.clone()
                } else {
                    to.to_owned()
                }
            }
        };
        let mut checks = Vec::new();
        if fields_change {
            if matches!(
                status,
                EventStatus::Closed
                    | EventStatus::Drawing
                    | EventStatus::Announced
                    | EventStatus::Fulfilling
                    | EventStatus::Completed
                    | EventStatus::Cancelled
            ) {
                return Err(refuse(
                    format!(
                        "{} is {status}; its entries are closed, so it cannot change.",
                        e.name
                    ),
                    &["draft", "scheduled", "running", "paused"],
                ));
            }
            let entered: i64 = entry_counts(&mut tx, &e.id).await?.values().sum();
            let lost = takes_away(&before, &after);
            if entered > 0 && !lost.is_empty() {
                return Err(ToolError::new(
                    CHANGES_AFTER_ENTRY,
                    format!(
                        "{entered} people entered {}; the change {}.",
                        e.name,
                        crate::model::and_list(&lost)
                    ),
                )
                .detail(json!({"rule": "event.eligibility", "items": lost}))
                .next("get_event"));
            }
            checks = check_draft(cx, &mut tx, &after, "update_event").await?;
            checks.retain(|c| c.rule == "event.schedule" || c.rule == "event.compliance.rules");
            checks.push(Check {
                rule: "event.eligibility".into(),
                passed: true,
                detail: Some(if entered == 0 {
                    "Nobody entered yet".into()
                } else {
                    format!("Nothing is taken from the {entered} who entered")
                }),
                fix_tool: None,
            });
        }
        tx.commit().await.map_err(ToolError::from)?;
        let mut changes = Vec::new();
        if stored_to != e.event_status {
            changes.push(Change {
                op: ChangeOp::Update,
                entity: "event".into(),
                id: e.id.clone(),
                from: json!(status),
                to: json!(stored_to),
                note: None,
            });
        }
        if fields_change {
            let b = serde_json::to_value(&before).unwrap_or_default();
            let a = serde_json::to_value(&after).unwrap_or_default();
            let keys: BTreeSet<&String> =
                a.as_object().into_iter().flat_map(|o| o.keys()).collect();
            let mut from = serde_json::Map::new();
            let mut to = serde_json::Map::new();
            for k in keys {
                if a.get(k) != b.get(k) {
                    from.insert(k.clone(), b.get(k).cloned().unwrap_or(Value::Null));
                    to.insert(k.clone(), a.get(k).cloned().unwrap_or(Value::Null));
                }
            }
            changes.push(Change {
                op: ChangeOp::Update,
                entity: "event".into(),
                id: e.id.clone(),
                from: Value::Object(from),
                to: Value::Object(to),
                note: None,
            });
        }
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        Ok(Planned {
            plan: EventUpdateRun {
                event_id: e.id.clone(),
                before,
                after,
                stored_from: e.event_status.clone(),
                stored_to,
            },
            changes,
            checks,
            basis: vec![Basis {
                entity: "promotion.event".into(),
                id: e.id,
                version: i64::from(e.version),
            }],
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval.reason.unwrap_or_default(),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &EventUpdateRun) -> Result<Committed, ToolError> {
        if run.after != run.before {
            write_fields(tx, &run.event_id, &run.after).await?;
        }
        if run.stored_to != run.stored_from
            && !set_event_status(tx, &run.event_id, &run.stored_from, &run.stored_to).await?
        {
            let now = event(tx, &run.event_id).await?.map(|e| e.event_status);
            if now.as_deref() != Some(run.stored_to.as_str()) {
                return Err(ToolError::new(
                    ErrorCode::INVALID_STATE,
                    "The event changed since the preview; preview again.",
                )
                .detail(json!({"entity": "event", "id": run.event_id}))
                .next("get_event"));
            }
        }
        Ok(Committed {
            id: run.event_id.clone(),
            produced: serde_json::Map::from_iter([("event_id".to_owned(), json!(run.event_id))]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &EventUpdateRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let e = event(&mut tx, &run.event_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = e.is_some_and(|e| {
            EventDraft::from_row(&e) == run.after && e.event_status == run.stored_to
        });
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} differs from the plan", run.event_id)]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        run: &EventUpdateRun,
        _: &Committed,
    ) -> Result<(), ToolError> {
        if run.after != run.before {
            write_fields(tx, &run.event_id, &run.before).await?;
        }
        if run.stored_to != run.stored_from {
            set_event_status(tx, &run.event_id, &run.stored_to, &run.stored_from).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{LimitPeriod, PurchaseCondition};
    use acom_contract::schema::{closed_schema_for, violations};
    use chrono::{DateTime, Utc};

    fn draft() -> EventDraft {
        let t = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
        EventDraft {
            name: "Fall check-in week".into(),
            start_date: t("2026-11-09T00:00:00-05:00"),
            end_date: t("2026-11-15T23:59:00-05:00"),
            display_valid_from: None,
            display_valid_through: None,
            entry_mechanic: "check_in".into(),
            selection_method: "random_draw".into(),
            entry_payload_schema: None,
            reward_tiers: vec![RewardTier {
                rank: 1,
                reward_type: "product".into(),
                reward_ref: Some("prd_wool_overcoat".into()),
                reward_value: None,
                winner_count: 1,
                waitlist_count: Some(2),
                odds: None,
                round: None,
            }],
            eligibility: Eligibility::default(),
            entry_limit: EntryLimit {
                count: 1,
                period: LimitPeriod::Day,
            },
            draw_at: None,
            announce_at: None,
            dispatch_at: None,
            compliance_jurisdictions: Vec::new(),
        }
    }

    #[test]
    fn after_entries_only_widening_is_allowed() {
        let b = draft();
        let mut wider = b.clone();
        wider.reward_tiers[0].winner_count = 2;
        wider.end_date += chrono::Duration::days(1);
        assert!(takes_away(&b, &wider).is_empty());
        let mut narrower = b.clone();
        narrower.eligibility.valid_for_member_tier = Some("vip".into());
        narrower.eligibility.purchase_condition = Some(PurchaseCondition::default());
        narrower.reward_tiers.clear();
        assert_eq!(
            takes_away(&b, &narrower),
            vec![
                "limits it to a tier",
                "asks more of the purchase",
                "removes rank 1"
            ]
        );
        let s = closed_schema_for::<UpdateEventInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
    }
}
