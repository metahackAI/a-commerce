//! `list_tiers`.

use crate::access::only_fields;
use crate::{PROGRAM_NOT_FOUND, repo};
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// A field of a tier row.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TierField {
    /// The tier id.
    Id,
    /// The name people see.
    Name,
    /// Higher is better.
    Rank,
    /// The program the tier belongs to.
    ProgramId,
    /// Members in the tier now.
    MemberCount,
}

impl TierField {
    fn word(self) -> &'static str {
        match self {
            Self::Id => "id",
            Self::Name => "name",
            Self::Rank => "rank",
            Self::ProgramId => "program_id",
            Self::MemberCount => "member_count",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListTiersInput {
    /// Only the tiers of this program. All programs when left out.
    #[schemars(regex(pattern = r"^prg_[a-z0-9_]+$"), example = &"prg_rewards")]
    pub program_id: Option<String>,
    /// Return only these fields of each tier. All when left out.
    #[schemars(example = ["id", "member_count"])]
    pub fields: Option<Vec<TierField>>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListTiers;

impl Operation for ListTiers {
    const META: OpMeta = OpMeta {
        address: "customer.membership.list_tiers",
        tool: "list_tiers",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/customer/tiers",
        summary: "The store's member tiers with how many members each has, highest first. Turns words like \"VIP members\" into a tier id for valid_for_member_tier.",
        consumes: &[],
        produces: &[Slot::many("tier_id")],
        checks: &[],
        errors: &[PROGRAM_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListTiersInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListTiers {
    async fn read(&self, cx: &Cx, input: ListTiersInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        if let Some(p) = &input.program_id
            && repo::program(&mut tx, p).await?.is_none()
        {
            return Err(
                ToolError::new(PROGRAM_NOT_FOUND, format!("There is no program {p}."))
                    .detail(json!({"entity": "program", "id": p}))
                    .next("list_tiers"),
            );
        }
        let tiers = repo::tiers(&mut tx, input.program_id.as_deref()).await?;
        let counts = repo::member_counts(&mut tx, input.program_id.as_deref()).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let fields: Vec<String> = input
            .fields
            .unwrap_or_default()
            .iter()
            .map(|f| f.word().to_owned())
            .collect();
        let total = i64::try_from(tiers.len()).unwrap_or(i64::MAX);
        let items: Vec<Value> = tiers
            .iter()
            .skip(usize::try_from(page.offset).unwrap_or(usize::MAX))
            .take(usize::try_from(page.limit).unwrap_or(0))
            .map(|t| {
                let n = counts.iter().find(|(id, _)| *id == t.id).map_or(0, |c| c.1);
                only_fields(
                    json!({"id": t.id, "name": t.name, "rank": t.rank,
                           "program_id": t.member_program_id, "member_count": n}),
                    &fields,
                )
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
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
    fn input_is_closed_and_described() {
        let s = closed_schema_for::<ListTiersInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
    }
}
