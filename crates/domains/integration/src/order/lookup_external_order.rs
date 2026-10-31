//! `lookup_external_order` (`integration.order.lookup_external_order`): an outside channel's
//! orders and whether they can ship. The channel is not connected in this store, so the answer
//! is the copy it last sent, and says so; nothing is asked of the channel and nothing changes.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::channel::registered;
use crate::model::{copy_json, holds_orders, lookup_note};
use crate::repo::{self, CopyFilter, CopyKind};
use crate::{CHANNEL_NOT_REGISTERED, EXTERNAL_LOOKUP_FAILED, check_id, list, parse_time};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LookupExternalOrderInput {
    /// The marketplace or outside warehouse (`list_channels`).
    #[schemars(regex(pattern = r"^chn_[a-z0-9_]+$"), example = &"chn_marketplace_a")]
    pub channel_id: String,
    /// One order, by the channel's own order number.
    #[schemars(length(min = 1, max = 120), example = &"MK-55102")]
    pub external_order_ref: Option<String>,
    /// Orders placed at or after this time (store time when no offset).
    #[schemars(example = &"2026-11-05T00:00")]
    pub order_date_from: Option<String>,
    /// Orders placed at or before this time (store time when no offset).
    #[schemars(example = &"2026-11-06T23:59")]
    pub order_date_through: Option<String>,
    /// Only orders in this status on the channel, in the channel's own word.
    #[serde(rename = "acom:external_status", alias = "external_status")]
    #[schemars(length(min = 1, max = 80), example = &"paid_waiting_shipment")]
    pub external_status: Option<String>,
    /// Orders per page, 1–100 (default 20).
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` of the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// Look up an outside channel's orders.
#[derive(Debug)]
pub struct LookupExternalOrder;

impl Operation for LookupExternalOrder {
    const META: OpMeta = OpMeta {
        address: "integration.order.lookup_external_order",
        tool: "lookup_external_order",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Post,
        path: "/v1/integration/orders/lookup",
        summary: "Read an outside channel's orders (a marketplace's or an outside warehouse's), whether each can ship, and the store order it maps to. Reads the copy the channel last sent; changes nothing.",
        consumes: &[Slot::one("channel_id")],
        produces: &[Slot::many("external_order_ref")],
        checks: &[],
        errors: &[CHANNEL_NOT_REGISTERED, EXTERNAL_LOOKUP_FAILED],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = LookupExternalOrderInput;
    type Output = Value;
}

impl ReadOp<Cx> for LookupExternalOrder {
    async fn read(&self, cx: &Cx, input: LookupExternalOrderInput) -> Result<Value, ToolError> {
        check_id("channel_id", "chn", &input.channel_id)?;
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let filter = CopyFilter {
            reference: input.external_order_ref.map(|r| r.trim().to_owned()),
            from: input
                .order_date_from
                .as_deref()
                .map(|t| parse_time(cx, "order_date_from", t))
                .transpose()?,
            through: input
                .order_date_through
                .as_deref()
                .map(|t| parse_time(cx, "order_date_through", t))
                .transpose()?,
            external_status: input.external_status.map(|s| s.trim().to_owned()),
            claim_type: None,
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let channel = registered(&mut tx, &input.channel_id, CHANNEL_NOT_REGISTERED).await?;
        holds_orders(&channel, "list_channels")?;
        let (rows, total) = repo::copies(
            &mut tx,
            CopyKind::Order,
            &channel.id,
            &filter,
            page.limit,
            page.offset,
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let items = rows
            .iter()
            .map(|r| copy_json(&channel, r, CopyKind::Order, &cx.time))
            .collect();
        let mut out = list(items, total, &page);
        out["acom:source"] = lookup_note(&channel, "orders", &cx.time);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};
    use serde_json::json;

    #[test]
    fn the_input_is_closed() {
        let s = closed_schema_for::<LookupExternalOrderInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["channel_id"]));
        assert!(
            serde_json::from_value::<LookupExternalOrderInput>(
                json!({"channel_id": "chn_marketplace_a", "status": "paid"})
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn a_bad_channel_id_is_refused_before_any_query() {
        let e = LookupExternalOrder
            .read(
                &crate::testing::offline("adm_sj"),
                serde_json::from_value(json!({"channel_id": "marketplace_a"})).unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
    }

    #[tokio::test]
    #[ignore = "integration: needs the demo database (DATABASE_URL)"]
    async fn reads_the_marketplace_copy_and_says_it_is_not_connected() {
        let cx = crate::testing::db("adm_sj").await;
        let out = LookupExternalOrder
            .read(
                &cx,
                serde_json::from_value(json!({"channel_id": "chn_marketplace_a",
                                              "external_order_ref": "MK-55102"}))
                .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(out["total"], 1, "{out:#}");
        let o = &out["items"][0];
        assert_eq!(o["acom:external_status"], "paid_waiting_shipment");
        assert_eq!(o["acom:order_status"], "processing");
        assert_eq!(o["acom:shippable"], true);
        assert_eq!(out["acom:source"]["connected"], false);
        let e = LookupExternalOrder
            .read(
                &cx,
                serde_json::from_value(json!({"channel_id": "chn_nowhere"})).unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("channel_not_registered", Some("register_channel"))
        );
        let e = LookupExternalOrder
            .read(
                &cx,
                serde_json::from_value(json!({"channel_id": "chn_carrier_ups"})).unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
    }
}
