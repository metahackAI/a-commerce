//! `create_event` (docs/api/promotion.md#event): an event made as a `draft`. The draw seed is
//! made when it is created and only its hash (`seed_commitment`) is shown until a draw runs
//! (`event.draw.fairness.seed_commit_before_close`).

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Change, ChangeOp, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    COMBINATION_NOT_ALLOWED, DisplayPeriod, Eligibility, EntryLimit, EventDraft, EventSchedule,
    INVALID_SCHEDULE, ODDS_EXCEED_ONE, REWARD_TYPE_DISABLED, RewardTier, TYPE_DISABLED,
    check_draft, default_limit, event, event_time, new_seed, sha256,
};
use crate::repo::{self, db};
use crate::{TIER_NOT_FOUND, audience};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateEventInput {
    /// Name people read.
    #[schemars(length(min = 1, max = 100), example = &"Fall check-in week")]
    pub name: String,
    /// First moment entries are taken (store time without an offset).
    #[schemars(example = &"2026-11-09T00:00")]
    pub start_date: String,
    /// Last moment entries are taken (inclusive, minute precision).
    #[schemars(example = &"2026-11-15T23:59")]
    pub end_date: String,
    /// When shoppers see the event. Default: the entry period.
    #[schemars(example = json!({"valid_from": "2026-11-06T00:00", "valid_through": "2026-11-22T23:59"}))]
    pub display_period: Option<DisplayPeriod>,
    /// How shoppers enter: an id from `list_event_types` (kind entry_mechanic).
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"check_in")]
    pub entry_mechanic: String,
    /// How winners are chosen: an id from `list_event_types` (kind selection_method).
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"random_draw")]
    pub selection_method: String,
    /// The fields an entry sends, `{field: type}` (survey questions, a photo and caption). Needed
    /// for `custom` mechanics.
    #[schemars(example = json!({"image": "ImageObject", "caption": "Text"}))]
    pub entry_payload_schema: Option<Value>,
    /// Rewards by rank. `instant_win` needs `odds`; `check_in` may set `round`.
    #[schemars(length(min = 1, max = 20), example = json!([{"rank": 1, "reward_type": "product", "reward_ref": "prd_wool_overcoat", "reward_value": {"value": 289.00, "currency": "USD"}, "winner_count": 1, "waitlist_count": 2}, {"rank": 2, "reward_type": "loyalty_points", "reward_value": {"value": 5.00, "currency": "USD"}, "winner_count": 100}]))]
    pub reward_tiers: Vec<RewardTier>,
    /// Who may enter. Default: everyone.
    #[schemars(example = json!({"valid_for_member_tier": "everyone"}))]
    pub eligibility: Option<Eligibility>,
    /// Entries per member. Default: `event.entry_limit.per_member`.
    #[schemars(example = json!({"count": 1, "period": "day"}))]
    pub entry_limit: Option<EntryLimit>,
    /// When the draw, the announcement and the rewards happen.
    #[schemars(example = json!({"draw_at": "2026-11-16T10:00"}))]
    pub schedule: Option<EventSchedule>,
    /// Store-defined jurisdiction ids whose compliance rules apply. Default: chosen by the
    /// winner's address (`event.compliance.jurisdiction_from`).
    #[schemars(inner(regex(pattern = r"^[a-z][a-z0-9_]*$")), example = json!([]))]
    pub compliance_jurisdictions: Option<Vec<String>>,
}

/// What a run of `create_event` writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRun {
    pub event_id: String,
    pub draft: EventDraft,
}

pub struct CreateEvent;

impl Operation for CreateEvent {
    const META: OpMeta = OpMeta {
        address: "promotion.event.create",
        tool: "create_event",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/promotion/events",
        summary: "Create an event as a draft: how shoppers enter (list_event_types), how winners are chosen, rewards by rank (list_reward_types), who may enter, the entry limit and the schedule. Checks the store's registries, the schedule order and its compliance rules.",
        consumes: &[
            Slot::one("entry_mechanic"),
            Slot::one("selection_method"),
            Slot::one("reward_type"),
            Slot::one("tier_id"),
            Slot::one("segment_id"),
        ],
        produces: &[Slot::one("event_id")],
        checks: &[
            "event.entry_mechanics",
            "event.selection_methods.allowed_combinations",
            "event.reward_types",
            "event.schedule",
            "event.compliance.rules",
        ],
        errors: &[
            TYPE_DISABLED,
            COMBINATION_NOT_ALLOWED,
            REWARD_TYPE_DISABLED,
            ODDS_EXCEED_ONE,
            TIER_NOT_FOUND,
            INVALID_SCHEDULE,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreateEventInput;
    type Output = Value;
}

/// The draft of an event from the create inputs.
pub(crate) fn draft_of(
    cx: &Cx,
    input: CreateEventInput,
    next: &'static str,
) -> Result<EventDraft, ToolError> {
    let t = |field: &str, s: &str| event_time(&cx.time, field, s, next);
    let schedule = input.schedule.unwrap_or_default();
    let (dfrom, dthrough) = match &input.display_period {
        Some(p) => (
            Some(t("display_period.valid_from", &p.valid_from)?),
            Some(t("display_period.valid_through", &p.valid_through)?),
        ),
        None => (None, None),
    };
    let mut eligibility = input.eligibility.unwrap_or_default();
    if eligibility.valid_for_member_tier.as_deref() == Some("everyone") {
        eligibility.valid_for_member_tier = None;
    }
    let mut tiers = input.reward_tiers;
    tiers.sort_by_key(|r| r.rank);
    Ok(EventDraft {
        name: input.name.trim().to_owned(),
        start_date: t("start_date", &input.start_date)?,
        end_date: t("end_date", &input.end_date)?,
        display_valid_from: dfrom,
        display_valid_through: dthrough,
        entry_mechanic: input.entry_mechanic,
        selection_method: input.selection_method,
        entry_payload_schema: input.entry_payload_schema,
        reward_tiers: tiers,
        eligibility,
        entry_limit: match input.entry_limit {
            Some(l) => l,
            None => default_limit(&cx.policy)?,
        },
        draw_at: schedule
            .draw_at
            .as_deref()
            .map(|s| t("schedule.draw_at", s))
            .transpose()?,
        announce_at: schedule
            .announce_at
            .as_deref()
            .map(|s| t("schedule.announce_at", s))
            .transpose()?,
        dispatch_at: schedule
            .dispatch_at
            .as_deref()
            .map(|s| t("schedule.dispatch_at", s))
            .transpose()?,
        compliance_jurisdictions: input.compliance_jurisdictions.unwrap_or_default(),
    })
}

/// Writes the event's fields (all of them) to its row.
pub(crate) async fn write_fields(
    tx: &mut TxCx,
    id: &str,
    d: &EventDraft,
) -> Result<u64, ToolError> {
    Ok(sqlx::query(
        "UPDATE promotion.event SET name = $3, start_date = $4, end_date = $5,
           display_valid_from = $6, display_valid_through = $7, entry_mechanic = $8,
           selection_method = $9, entry_payload_schema = $10, reward_tiers = $11,
           eligibility = $12, valid_for_member_tier = $13, entry_limit = $14, draw_at = $15,
           announce_at = $16, dispatch_at = $17, compliance_jurisdictions = $18,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(&d.name)
    .bind(d.start_date)
    .bind(d.end_date)
    .bind(d.display_valid_from)
    .bind(d.display_valid_through)
    .bind(&d.entry_mechanic)
    .bind(&d.selection_method)
    .bind(&d.entry_payload_schema)
    .bind(serde_json::to_value(&d.reward_tiers).unwrap_or_default())
    .bind(serde_json::to_value(&d.eligibility).unwrap_or_default())
    .bind(&d.eligibility.valid_for_member_tier)
    .bind(serde_json::to_value(d.entry_limit).unwrap_or_default())
    .bind(d.draw_at)
    .bind(d.announce_at)
    .bind(d.dispatch_at)
    .bind(&d.compliance_jurisdictions)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?
    .rows_affected())
}

impl WriteOp<Cx> for CreateEvent {
    type Plan = EventRun;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreateEventInput,
    ) -> Result<Planned<EventRun>, ToolError> {
        let draft = draft_of(cx, input, "create_event")?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let checks = check_draft(cx, &mut tx, &draft, "create_event").await?;
        tx.commit().await.map_err(ToolError::from)?;
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "event".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"name": draft.name, "status": "draft",
                           "entry_mechanic": draft.entry_mechanic,
                           "selection_method": draft.selection_method,
                           "start_date": cx.time.format(draft.start_date),
                           "end_date": cx.time.format(draft.end_date),
                           "valid_for_member_tier": audience(draft.eligibility.valid_for_member_tier.as_deref()),
                           "ranks": draft.reward_tiers.len()}),
                note: None,
            }],
            plan: EventRun {
                event_id: repo::new_id("evt"),
                draft,
            },
            checks,
            basis: Vec::new(),
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval.reason.unwrap_or_default(),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &EventRun) -> Result<Committed, ToolError> {
        if event(tx, &run.event_id).await?.is_none() {
            let seed = new_seed();
            let change_set_id = tx.change_set_id.clone();
            sqlx::query(
                "INSERT INTO promotion.event (tenant_id, id, name, event_status, start_date,
                   end_date, entry_mechanic, selection_method, reward_tiers, entry_limit,
                   seed_commitment, draw_seed, change_set_id)
                 VALUES ($1, $2, $3, 'draft', $4, $5, $6, $7, '[]', '{}', $8, $9, $10)
                 ON CONFLICT (tenant_id, id) DO NOTHING",
            )
            .bind(tx.tenant.as_str())
            .bind(&run.event_id)
            .bind(&run.draft.name)
            .bind(run.draft.start_date)
            .bind(run.draft.end_date)
            .bind(&run.draft.entry_mechanic)
            .bind(&run.draft.selection_method)
            .bind(sha256(&seed))
            .bind(&seed)
            .bind(change_set_id)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
            write_fields(tx, &run.event_id, &run.draft).await?;
        }
        Ok(Committed {
            id: run.event_id.clone(),
            produced: serde_json::Map::from_iter([("event_id".to_owned(), json!(run.event_id))]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &EventRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let e = event(&mut tx, &run.event_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = e.is_some_and(|e| EventDraft::from_row(&e) == run.draft);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} differs from the plan", run.event_id)]
            },
        })
    }

    async fn undo(&self, tx: &mut TxCx, run: &EventRun, _: &Committed) -> Result<(), ToolError> {
        // Without entries the event goes; with entries it is cancelled and kept as a record.
        let entered: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM promotion.event_entry WHERE tenant_id = $1 AND event_id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&run.event_id)
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        let sql = if entered == 0 {
            "DELETE FROM promotion.event WHERE tenant_id = $1 AND id = $2"
        } else {
            "UPDATE promotion.event SET event_status = 'cancelled', updated_at = now(),
               version = version + 1 WHERE tenant_id = $1 AND id = $2"
        };
        sqlx::query(sql)
            .bind(tx.tenant.as_str())
            .bind(&run.event_id)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn takes_the_spec_example() {
        let s = closed_schema_for::<CreateEventInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        let i: CreateEventInput = serde_json::from_value(json!({
            "name": "Fall check-in week", "start_date": "2026-11-09T00:00", "end_date": "2026-11-15T23:59",
            "entry_mechanic": "check_in", "selection_method": "random_draw",
            "reward_tiers": [{"rank": 1, "reward_type": "product", "reward_ref": "prd_wool_overcoat",
                              "reward_value": {"value": 289.00, "currency": "USD"}, "winner_count": 1, "waitlist_count": 2},
                             {"rank": 2, "reward_type": "loyalty_points", "reward_value": {"value": 5.00, "currency": "USD"}, "winner_count": 100}],
            "eligibility": {"valid_for_member_tier": "everyone"},
            "entry_limit": {"count": 1, "period": "day"},
            "schedule": {"draw_at": "2026-11-16T10:00"}, "compliance_jurisdictions": []}))
        .unwrap();
        assert_eq!(i.reward_tiers.len(), 2);
        assert_eq!(CreateEvent::META.kind, Kind::Write);
    }
}
