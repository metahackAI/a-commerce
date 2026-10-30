//! `get_change_history` (docs/api/catalog.md#approval).

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::StoreTime;
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::product::visible_group;
use super::product_not_found;
use crate::model::change_type;
use crate::repo::{self, HistoryRow};
use crate::{PRODUCT_NOT_FOUND, list};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetChangeHistoryInput {
    /// The product.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_breton_tee")]
    pub product_id: String,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// What ran on a product, newest first.
#[derive(Debug)]
pub struct GetChangeHistory;

impl Operation for GetChangeHistory {
    const META: OpMeta = OpMeta {
        address: "catalog.approval.get_history",
        tool: "get_change_history",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/products/{product_id}/history",
        summary: "The append-only change history of a product and its variants and offers: who changed what, from what to what, and whether it was a content or structural change.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::many("action_id")],
        checks: &[],
        errors: &[PRODUCT_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetChangeHistoryInput;
    type Output = Value;
}

/// One history entry: the action, who did it, and one change it made.
fn entry(r: &HistoryRow, time: &StoreTime) -> Value {
    let agent = match r.actor_kind.as_str() {
        "system" => json!({"@type": "SoftwareApplication", "id": r.agent_id,
                           "name": r.agent_name.as_deref().unwrap_or("system")}),
        "agent_on_behalf" | "external_agent" => json!({"@type": "SoftwareApplication",
            "name": r.agent_name, "acom:on_behalf_of": r.agent_id}),
        _ => json!({"@type": "Person", "id": r.agent_id, "name": r.agent_name}),
    };
    let to = r.change.get("to").cloned().unwrap_or(Value::Null);
    json!({
        "action_id": r.id,
        "agent": agent,
        "tool": r.instrument,
        "op": r.change.get("op"),
        "entity": r.change.get("entity"),
        "id": r.change.get("id"),
        "change_type": change_type(&r.instrument, &to),
        "from": r.change.get("from").cloned().unwrap_or(Value::Null),
        "to": to,
        "end_time": time.format(r.end_time),
    })
}

impl ReadOp<Cx> for GetChangeHistory {
    async fn read(&self, cx: &Cx, input: GetChangeHistoryInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let Some(group) = visible_group(cx, &mut tx, &input.product_id).await? else {
            return Err(product_not_found(
                "product",
                &input.product_id,
                "search_products",
            ));
        };
        // The product and every record under it: variants and their offers.
        let mut records = vec![group.id.clone()];
        for v in repo::variants(&mut tx, std::slice::from_ref(&group.id)).await? {
            records.extend(v.offer_id.clone());
            records.push(v.id);
        }
        let (rows, total) = repo::history(&mut tx, &records, page.limit, page.offset).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items = rows.iter().map(|r| entry(r, &cx.time)).collect();
        Ok(list(items, total, &page))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::testing;

    #[test]
    fn a_price_fix_reads_as_a_structural_change_by_the_system() {
        let time = StoreTime::new(chrono_tz::America::New_York);
        let row = HistoryRow {
            id: "act_1040_01".into(),
            actor_kind: "system".into(),
            agent_id: Some("sys_executor".into()),
            agent_name: None,
            instrument: "set_price".into(),
            end_time: time
                .parse("2026-11-06T08:31:11")
                .unwrap_or_else(|e| panic!("{e}")),
            change: json!({"op": "update", "entity": "offer",
                           "id": "off_breton_tee_xs_navy_stripe",
                           "from": {"price": 19.00}, "to": {"price": 23.00}}),
        };
        let e = entry(&row, &time);
        assert_eq!(e["change_type"], "structural");
        assert_eq!(e["agent"]["@type"], "SoftwareApplication");
        assert_eq!(e["end_time"], "2026-11-06T08:31:11-05:00");
        assert_eq!(e["from"]["price"].to_string(), "19.0");
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn breton_tee_history_has_the_floor_fix() {
        let cx = testing::operator_db().await;
        let out = GetChangeHistory
            .read(
                &cx,
                GetChangeHistoryInput {
                    product_id: "prd_breton_tee".into(),
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(out["total"].as_i64().unwrap_or(0) >= 1);
        assert_eq!(out["items"][0]["tool"], "set_price");
    }
}
