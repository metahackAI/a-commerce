//! `preview_segment` (docs/api/promotion.md#segment): how many customers a stored segment or a
//! set of conditions holds now, a sample, and the count per tier. A read, though it is a POST.

use std::collections::BTreeSet;

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{ConditionRules, by_tier, members, members_of, validate};
use crate::{TIER_NOT_FOUND, check_id};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewSegmentInput {
    /// A stored segment (`list_segments`). Give this or `condition_rules`.
    #[schemars(regex(pattern = r"^seg_[a-z0-9_]+$"), example = &"seg_lapsed_knit_buyers")]
    pub segment_id: Option<String>,
    /// Conditions to try, the same shape as `define_segment`. Give this or `segment_id`.
    #[schemars(example = json!({"member_tiers": ["gold"], "last_order_within_days": 30}))]
    pub condition_rules: Option<ConditionRules>,
    /// How many customer ids to show, 1–20. Default 5.
    #[schemars(range(min = 1, max = 20), example = 5)]
    pub sample_size: Option<u32>,
}

pub struct PreviewSegment;

impl Operation for PreviewSegment {
    const META: OpMeta = OpMeta {
        address: "promotion.segment.preview",
        tool: "preview_segment",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Post,
        path: "/v1/promotion/segments/preview",
        summary: "How many customers a saved segment or a set of conditions holds right now, with a sample of ids and the count per tier. Changes nothing.",
        consumes: &[Slot::one("segment_id")],
        produces: &[Slot::many("customer_id")],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND, TIER_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::ASSESS,
        person_scope: "",
    };
    type Input = PreviewSegmentInput;
    type Output = Value;
}

impl ReadOp<Cx> for PreviewSegment {
    async fn read(&self, cx: &Cx, input: PreviewSegmentInput) -> Result<Value, ToolError> {
        let sample = usize::try_from(input.sample_size.unwrap_or(5).clamp(1, 20)).unwrap_or(5);
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let found = match (&input.segment_id, &input.condition_rules) {
            (Some(id), None) => {
                check_id("segment_id", "seg", id, false)?;
                members_of(&mut tx, id, now, "list_segments").await?.1
            }
            (None, Some(rules)) => {
                validate(&mut tx, rules, "preview_segment").await?;
                members(&mut tx, rules, now, &mut BTreeSet::new()).await?
            }
            _ => {
                return Err(ToolError::invalid_input(
                    "Give either `segment_id` or `condition_rules`.",
                )
                .detail(json!({"field": "segment_id", "reason": "missing"})));
            }
        };
        tx.commit().await.map_err(ToolError::from)?;
        let ids: Vec<&String> = found.iter().take(sample).map(|m| &m.customer_id).collect();
        Ok(json!({
            "estimated_size": found.len(),
            "sample_customer_ids": ids,
            "by_tier": by_tier(&found),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn takes_a_segment_or_conditions() {
        let s = closed_schema_for::<PreviewSegmentInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        let _: PreviewSegmentInput =
            serde_json::from_value(json!({"segment_id": "seg_x", "sample_size": 5})).unwrap();
        assert_eq!(PreviewSegment::META.kind, Kind::Read);
    }
}
