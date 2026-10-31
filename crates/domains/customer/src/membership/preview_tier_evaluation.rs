//! `preview_tier_evaluation`: who would move up, down, or into a grace period if the program
//! were evaluated now (or `as_of`). Changes nothing.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    EvaluationScope, as_of, entry_benefits, evaluate_members, grouped, program_not_found,
    scope_ids, tier_defs,
};
use crate::access::{only_fields, staff_only};
use crate::progress::Direction;
use crate::wording::mask_name;
use crate::{AS_OF_IN_FUTURE, PROGRAM_NOT_FOUND, repo};

/// Which moves to list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MoveDirection {
    /// Moves to a higher tier.
    Up,
    /// Moves to a lower tier.
    Down,
    /// Kept for now with a warning (grace period).
    Grace,
    /// Every move.
    Any,
}

/// How the answer is shaped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GroupBy {
    /// One row per member.
    Member,
    /// One row per path (from, to) with the number of members.
    Move,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewTierEvaluationInput {
    /// The program (`list_tiers`).
    #[schemars(regex(pattern = r"^prg_[a-z0-9_]+$"), example = &"prg_rewards")]
    pub program_id: String,
    /// The moment to measure at, store time. Now when left out; not in the future.
    #[schemars(example = &"2026-11-06T11:00")]
    pub as_of: Option<String>,
    /// all (default), segment (`segment_id`) or customers (`customer_ids`).
    #[schemars(example = &"all")]
    pub scope: Option<EvaluationScope>,
    /// The segment, with `scope: segment`.
    #[schemars(regex(pattern = r"^seg_[a-z0-9_]+$"), example = &"seg_lapsed_gold")]
    pub segment_id: Option<String>,
    /// The members, with `scope: customers`.
    #[schemars(length(min = 1, max = 100), example = ["cus_alex"])]
    pub customer_ids: Option<Vec<String>>,
    /// up, down, grace, or any (default).
    #[schemars(example = &"any")]
    pub direction: Option<MoveDirection>,
    /// member (default) or move.
    #[schemars(example = &"member")]
    pub group_by: Option<GroupBy>,
    /// Fields to return of each row. All when left out.
    #[schemars(example = ["customer_id", "to_tier_id"])]
    pub fields: Option<Vec<String>>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct PreviewTierEvaluation;

impl Operation for PreviewTierEvaluation {
    const META: OpMeta = OpMeta {
        address: "customer.membership.preview_tier_evaluation",
        tool: "preview_tier_evaluation",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/tier-programs/{program_id}/evaluation-preview",
        summary: "Who would move up, down or into a grace period if the tier program were evaluated now, and why. Changes nothing.",
        consumes: &[Slot::one("program_id")],
        produces: &[Slot::many("customer_id")],
        checks: &[],
        errors: &[PROGRAM_NOT_FOUND, AS_OF_IN_FUTURE],
        audience: Audience::Operator,
        action_type: ActionType::ASSESS,
        person_scope: "",
    };
    type Input = PreviewTierEvaluationInput;
    type Output = Value;
}

fn wanted(d: Option<MoveDirection>, m: Direction) -> bool {
    match d.unwrap_or(MoveDirection::Any) {
        MoveDirection::Any => matches!(m, Direction::Up | Direction::Down | Direction::Grace),
        MoveDirection::Up => m == Direction::Up,
        MoveDirection::Down => m == Direction::Down,
        MoveDirection::Grace => m == Direction::Grace,
    }
}

impl ReadOp<Cx> for PreviewTierEvaluation {
    async fn read(&self, cx: &Cx, input: PreviewTierEvaluationInput) -> Result<Value, ToolError> {
        staff_only(cx)?;
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let at = as_of(cx, input.as_of.as_deref(), "preview_tier_evaluation")?;
        let ids = scope_ids(
            input.scope.unwrap_or(EvaluationScope::All),
            input.segment_id.as_deref(),
            input.customer_ids.as_deref(),
            "preview_tier_evaluation",
        )?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let program = repo::program(&mut tx, &input.program_id)
            .await?
            .ok_or_else(|| program_not_found(&input.program_id))?;
        let tiers = tier_defs(&repo::tiers(&mut tx, Some(&program.id)).await?);
        let outcome = evaluate_members(
            &mut tx,
            &program,
            &tiers,
            &program.evaluation,
            &program.movement,
            ids.as_deref(),
            at,
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let moves: Vec<_> = outcome
            .moves
            .into_iter()
            .filter(|(_, m)| wanted(input.direction, m.direction))
            .collect();
        let fields = input.fields.unwrap_or_default();
        let (total, rows): (usize, Vec<Value>) = match input.group_by.unwrap_or(GroupBy::Member) {
            GroupBy::Move => {
                let g = grouped(&moves);
                let rows = g
                    .iter()
                    .map(|(f, t, d, n)| {
                        json!({"from_tier_id": f, "to_tier_id": t, "direction": d.word(),
                               "members": n})
                    })
                    .collect::<Vec<_>>();
                (rows.len(), rows)
            }
            GroupBy::Member => {
                let rows = moves
                    .iter()
                    .map(|(p, m)| {
                        json!({"customer_id": p.customer_id, "name": mask_name(&p.name),
                        "from_tier_id": m.from_tier_id, "to_tier_id": m.to_tier_id,
                        "direction": m.direction.word(), "reason": m.reason,
                        "benefits_on_entry": if m.direction == Direction::Up {
                            entry_benefits(&tiers, &m.to_tier_id)
                        } else {
                            vec![]
                        }})
                    })
                    .collect::<Vec<_>>();
                (rows.len(), rows)
            }
        };
        let total = i64::try_from(total).unwrap_or(i64::MAX);
        let items: Vec<Value> = rows
            .into_iter()
            .skip(usize::try_from(page.offset).unwrap_or(usize::MAX))
            .take(usize::try_from(page.limit).unwrap_or(0))
            .map(|r| only_fields(r, &fields))
            .collect();
        let mut out = json!({"items": items, "total": total,
                             "as_of": cx.time.format(at), "measured": outcome.measured});
        if !outcome.measured {
            out["note"] =
                json!("Orders are not connected here, so no result was measured and nobody moves.");
        }
        if let Some(c) = page.next_cursor(items.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::schema::{closed_schema_for, violations};

    use super::*;

    #[test]
    fn input_is_closed_with_word_options() {
        let s = closed_schema_for::<PreviewTierEvaluationInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(
            s["properties"]["group_by"]["enum"],
            json!(["member", "move"])
        );
        assert!(wanted(None, Direction::Grace));
        assert!(!wanted(None, Direction::Same));
    }
}
