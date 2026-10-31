//! `define_segment` (docs/api/promotion.md#segment). Segments are versioned: defining one with
//! the name of a segment the store has gives that segment new conditions and its next version
//! (same id); a new name makes a new segment.

use std::collections::BTreeSet;

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{ConditionRules, members, segment, segment_named, slug, validate};
use crate::TIER_NOT_FOUND;
use crate::repo::{self, db};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DefineSegmentInput {
    /// Name people read; an existing segment's name gives it new conditions and a new version.
    #[schemars(length(min = 1, max = 100), example = &"Lapsed knit buyers")]
    pub name: String,
    /// Who is in it; at least one condition, and every given condition must hold.
    #[schemars(example = json!({"purchased_category_ids": ["cat_knitwear"], "exclude_segment_ids": ["seg_recent_buyers"]}))]
    pub condition_rules: ConditionRules,
}

/// What a run of `define_segment` writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SegmentRun {
    pub segment_id: String,
    pub name: String,
    pub rules: ConditionRules,
    pub estimated_size: i32,
    pub estimated_at: DateTime<Utc>,
    /// The conditions before, when an existing segment gets a new version.
    pub before: Option<(ConditionRules, i32, DateTime<Utc>)>,
}

pub struct DefineSegment;

impl Operation for DefineSegment {
    const META: OpMeta = OpMeta {
        address: "promotion.segment.define",
        tool: "define_segment",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/promotion/segments",
        summary: "Define a group of customers by conditions (tiers, categories bought, last order, spend, other segments to leave out), for coupons, messages and events. A name the store already has gets new conditions as its next version.",
        consumes: &[],
        produces: &[Slot::one("segment_id")],
        checks: &[],
        errors: &[TIER_NOT_FOUND, ErrorCode::NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = DefineSegmentInput;
    type Output = Value;
}

impl WriteOp<Cx> for DefineSegment {
    type Plan = SegmentRun;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: DefineSegmentInput,
    ) -> Result<Planned<SegmentRun>, ToolError> {
        let name = input.name.trim().to_owned();
        if name.is_empty() {
            return Err(ToolError::invalid_input("`name` must not be empty.")
                .detail(json!({"field": "name", "reason": "missing"})));
        }
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        validate(&mut tx, &input.condition_rules, "define_segment").await?;
        let existing = segment_named(&mut tx, &name).await?;
        let mut seen: BTreeSet<String> = existing.iter().map(|s| s.id.clone()).collect();
        let found = members(&mut tx, &input.condition_rules, now, &mut seen).await?;
        let id = match &existing {
            Some(s) => s.id.clone(),
            None => {
                let readable = format!("seg_{}", slug(&name));
                if readable.len() > 4 && segment(&mut tx, &readable).await?.is_none() {
                    readable
                } else {
                    repo::new_id("seg")
                }
            }
        };
        tx.commit().await.map_err(ToolError::from)?;
        let size = i32::try_from(found.len()).unwrap_or(i32::MAX);
        let rules_json = serde_json::to_value(&input.condition_rules).unwrap_or_default();
        let (changes, basis) = match &existing {
            Some(s) => (
                vec![Change {
                    op: ChangeOp::Update,
                    entity: "segment".into(),
                    id: s.id.clone(),
                    from: json!({"version": s.version, "condition_rules": s.condition_rules, "estimated_size": s.estimated_size}),
                    to: json!({"version": s.version + 1, "condition_rules": rules_json, "estimated_size": size}),
                    note: Some(format!("A new version of {}", s.name)),
                }],
                vec![Basis {
                    entity: "promotion.segment".into(),
                    id: s.id.clone(),
                    version: i64::from(s.version),
                }],
            ),
            None => (
                vec![Change {
                    op: ChangeOp::Create,
                    entity: "segment".into(),
                    id: String::new(),
                    from: Value::Null,
                    to: json!({"name": name, "condition_rules": rules_json, "estimated_size": size}),
                    note: None,
                }],
                Vec::new(),
            ),
        };
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        Ok(Planned {
            plan: SegmentRun {
                segment_id: id,
                name,
                rules: input.condition_rules,
                estimated_size: size,
                estimated_at: now,
                before: existing.map(|s| (s.rules(), s.estimated_size, s.estimated_at)),
            },
            changes,
            checks: Vec::new(),
            basis,
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval.reason.unwrap_or_default(),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &SegmentRun) -> Result<Committed, ToolError> {
        let rules = serde_json::to_value(&run.rules).unwrap_or_default();
        sqlx::query(
            "INSERT INTO promotion.segment (tenant_id, id, name, condition_rules, estimated_size,
               estimated_at)
             VALUES ($1, $2, $3, $4, $5, $6)
             ON CONFLICT (tenant_id, id) DO UPDATE SET condition_rules = EXCLUDED.condition_rules,
               estimated_size = EXCLUDED.estimated_size, estimated_at = EXCLUDED.estimated_at,
               updated_at = now(), version = promotion.segment.version + 1
             WHERE promotion.segment.condition_rules IS DISTINCT FROM EXCLUDED.condition_rules",
        )
        .bind(tx.tenant.as_str())
        .bind(&run.segment_id)
        .bind(&run.name)
        .bind(&rules)
        .bind(run.estimated_size)
        .bind(run.estimated_at)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        Ok(Committed {
            id: run.segment_id.clone(),
            produced: serde_json::Map::from_iter([(
                "segment_id".to_owned(),
                json!(run.segment_id),
            )]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &SegmentRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let s = segment(&mut tx, &run.segment_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = s.is_some_and(|s| s.rules() == run.rules);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} differs from the plan", run.segment_id)]
            },
        })
    }

    async fn undo(&self, tx: &mut TxCx, run: &SegmentRun, _: &Committed) -> Result<(), ToolError> {
        match &run.before {
            None => {
                sqlx::query("DELETE FROM promotion.segment WHERE tenant_id = $1 AND id = $2")
                    .bind(tx.tenant.as_str())
                    .bind(&run.segment_id)
                    .execute(&mut *tx.tx)
                    .await
                    .map_err(db)?;
            }
            Some((rules, size, at)) => {
                sqlx::query(
                    "UPDATE promotion.segment SET condition_rules = $3, estimated_size = $4,
                       estimated_at = $5, updated_at = now(), version = version + 1
                     WHERE tenant_id = $1 AND id = $2",
                )
                .bind(tx.tenant.as_str())
                .bind(&run.segment_id)
                .bind(serde_json::to_value(rules).unwrap_or_default())
                .bind(size)
                .bind(at)
                .execute(&mut *tx.tx)
                .await
                .map_err(db)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn takes_the_spec_inputs() {
        let s = closed_schema_for::<DefineSegmentInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        let i: DefineSegmentInput = serde_json::from_value(json!({"name": "Lapsed knit buyers",
            "condition_rules": {"purchased_category_ids": ["cat_knitwear"], "last_order_within_days": 365}})).unwrap();
        assert_eq!(i.condition_rules.last_order_within_days, Some(365));
        assert_eq!(DefineSegment::META.kind, Kind::Write);
    }
}
