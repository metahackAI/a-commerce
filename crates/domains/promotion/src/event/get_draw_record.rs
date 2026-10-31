//! `get_draw_record` (docs/api/promotion.md#event): the audit record of one draw run. Operators
//! see all of it (`event.draw.fairness.record`); a shopper sees what is public: the method, the
//! seed hash and when the winners were announced. The approver is read from the change set that
//! ran the draw.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_store::Cx;
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::known_event;
use crate::repo::db;
use crate::{check_id, not_found};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetDrawRecordInput {
    /// The event.
    #[schemars(regex(pattern = r"^evt_[a-z0-9_]+$"), example = &"evt_0311")]
    pub event_id: String,
    /// The draw record (`get_event` lists them).
    #[schemars(regex(pattern = r"^drw_[a-z0-9_]+$"), example = &"drw_0311_01")]
    pub draw_record_id: String,
}

pub struct GetDrawRecord;

impl Operation for GetDrawRecord {
    const META: OpMeta = OpMeta {
        address: "promotion.event.get_draw_record",
        tool: "get_draw_record",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/events/{event_id}/draws/{draw_record_id}",
        summary: "The audit record of one draw: method, seed hash and seed, entrant snapshot, who was left out, the winners and waitlist, who ran and who approved it. Shoppers see only the method, the seed hash and when winners were announced.",
        consumes: &[Slot::one("draw_record_id")],
        produces: &[Slot::many("entry_id")],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetDrawRecordInput;
    type Output = Value;
}

#[derive(Debug, sqlx::FromRow)]
struct RecordRow {
    id: String,
    event_id: String,
    draw_mode: String,
    rank: Option<i32>,
    method: String,
    seed_commitment: String,
    seed: String,
    entrant_snapshot_digest: String,
    entrant_count: i32,
    excluded: Value,
    result: Value,
    reason: Option<String>,
    agent_id: String,
    change_set_id: Option<String>,
    start_time: DateTime<Utc>,
}

impl ReadOp<Cx> for GetDrawRecord {
    async fn read(&self, cx: &Cx, input: GetDrawRecordInput) -> Result<Value, ToolError> {
        check_id("draw_record_id", "drw", &input.draw_record_id, false)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let e = known_event(&mut tx, &input.event_id, "search_events").await?;
        let r = sqlx::query_as::<_, RecordRow>(
            "SELECT id, event_id, draw_mode, rank, method, seed_commitment, seed,
                    entrant_snapshot_digest, entrant_count, excluded, result, reason, agent_id,
                    change_set_id, start_time
             FROM promotion.draw_record WHERE tenant_id = $1 AND event_id = $2 AND id = $3",
        )
        .bind(tx.tenant.as_str())
        .bind(&e.id)
        .bind(&input.draw_record_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?
        .ok_or_else(|| not_found("draw_record", &input.draw_record_id, "get_event"))?;
        tx.commit().await.map_err(ToolError::from)?;
        let announced = e.announce_at.unwrap_or(r.start_time);
        if !acom_catalog::is_operator(&cx.principal) {
            return Ok(
                json!({"id": r.id, "event_id": r.event_id, "method": r.method,
                             "seed_commitment": r.seed_commitment,
                             "announced_at": cx.time.format(announced)}),
            );
        }
        let approved_by = match &r.change_set_id {
            Some(cs) => acom_changeset::read::get(cx, cs, &["approved_by".to_owned()])
                .await
                .ok()
                .and_then(|v| v.get("approved_by").cloned())
                .unwrap_or(Value::Null),
            None => Value::Null,
        };
        Ok(json!({
            "id": r.id, "event_id": r.event_id, "mode": r.draw_mode, "rank": r.rank,
            "method": r.method, "seed_commitment": r.seed_commitment, "seed": r.seed,
            "entrant_snapshot_digest": r.entrant_snapshot_digest, "entrant_count": r.entrant_count,
            "excluded": r.excluded, "results": r.result, "reason": r.reason,
            "agent": r.agent_id, "approved_by": approved_by, "change_set_id": r.change_set_id,
            "start_time": cx.time.format(r.start_time),
            "announced_at": cx.time.format(announced),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn takes_the_spec_inputs() {
        let s = closed_schema_for::<GetDrawRecordInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(s["required"], json!(["event_id", "draw_record_id"]));
        assert_eq!(GetDrawRecord::META.audience, Audience::Both);
    }
}
