//! `list_restock_subscriptions` (docs/api/catalog.md#restock_alert): the signed-in shopper's own
//! restock alerts, newest first, with whether the variant is back and the alert went out.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::AlertRow;
use crate::{INVALID_FILTER, list};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListRestockSubscriptionsInput {
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// The shopper's restock alerts.
#[derive(Debug)]
pub struct ListRestockSubscriptions;

impl Operation for ListRestockSubscriptions {
    const META: OpMeta = OpMeta {
        address: "catalog.restock_alert.list",
        tool: "list_restock_subscriptions",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/restock-alerts",
        summary: "The signed-in shopper's restock alerts: which product, size and color they wait for, how they will hear, whether it is back in stock, and when the alert went out or lapses.",
        consumes: &[],
        produces: &[Slot::many("restock_alert_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Shopper,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListRestockSubscriptionsInput;
    type Output = Value;
}

fn alert_json(a: &AlertRow, time: &acom_core::StoreTime) -> Value {
    json!({
        "id": a.id, "product_id": a.product_group_id, "product_name": a.product_name,
        "variant_id": a.product_id, "size": a.size, "color": a.color,
        "channel": a.message_channel, "availability": a.availability,
        "start_time": time.format(a.start_time), "expires": time.format(a.expires),
        "notified_at": a.notified_at.map(|t| time.format(t)),
    })
}

impl ReadOp<Cx> for ListRestockSubscriptions {
    async fn read(
        &self,
        cx: &Cx,
        input: ListRestockSubscriptionsInput,
    ) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let customer = crate::signed_in_shopper(cx, "see your restock alerts")?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (rows, total) = super::alerts_of(&mut tx, &customer, page.limit, page.offset).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(list(
            rows.iter().map(|a| alert_json(a, &cx.time)).collect(),
            total,
            &page,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[tokio::test]
    async fn guests_have_no_alerts_to_list() {
        let e = ListRestockSubscriptions
            .read(
                &testing::guest(),
                ListRestockSubscriptionsInput {
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
        let schema = acom_contract::schema::closed_schema_for::<ListRestockSubscriptionsInput>();
        assert!(acom_contract::schema::violations(&schema).is_empty());
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn jamie_waits_for_the_chore_coat_m() {
        let cx = testing::shopper_db().await;
        let out = ListRestockSubscriptions
            .read(
                &cx,
                ListRestockSubscriptionsInput {
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out["total"], json!(1));
        let a = &out["items"][0];
        assert_eq!(a["id"], "rsa_204");
        assert_eq!(a["variant_id"], "var_chore_coat_m_olive");
        assert_eq!(a["availability"], "out_of_stock");
        assert_eq!(a["start_time"], "2026-11-05T09:12:00-05:00");
        assert!(a["notified_at"].is_null());
    }
}
