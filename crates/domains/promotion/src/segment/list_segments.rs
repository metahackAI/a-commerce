//! `list_segments` (docs/api/promotion.md#segment).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::segments;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListSegmentsInput {
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListSegments;

impl Operation for ListSegments {
    const META: OpMeta = OpMeta {
        address: "promotion.segment.list",
        tool: "list_segments",
        group: Group::Engage,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/promotion/segments",
        summary: "The saved customer segments with their version, conditions and size when last defined.",
        consumes: &[],
        produces: &[Slot::many("segment_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListSegmentsInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListSegments {
    async fn read(&self, cx: &Cx, input: ListSegmentsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (total, rows) = segments(&mut tx, page.limit, page.offset).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|s| {
                json!({"id": s.id, "name": s.name, "version": s.version,
                       "estimated_size": s.estimated_size,
                       "estimated_at": cx.time.format(s.estimated_at),
                       "condition_rules": s.condition_rules})
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn pages_with_a_closed_schema() {
        let s = closed_schema_for::<ListSegmentsInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(ListSegments::META.method, Method::Get);
    }
}
