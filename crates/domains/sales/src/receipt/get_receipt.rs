//! `get_receipt` (`sales.receipt.get`): the receipt issued for an order.

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_core::enums::ReceiptType;
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{no_receipt, receipt_json};
use crate::ORDER_NOT_FOUND;
use crate::order::visible_order;
use crate::repo;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetReceiptInput {
    /// The order (`get_my_orders`, `search_orders`).
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20381")]
    pub order_id: String,
    /// Only a receipt of this type. The latest one of any type when left out.
    #[schemars(example = &"proof")]
    pub receipt_type: Option<ReceiptType>,
}

pub(crate) struct GetReceipt;

impl Operation for GetReceipt {
    const META: OpMeta = OpMeta {
        address: "sales.receipt.get",
        tool: "get_receipt",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/sales/orders/{order_id}/receipt",
        summary: "The receipt issued for an order: its type, when it was issued and the amount it states, with the order's other receipts. Shoppers see their own orders, operators every order.",
        consumes: &[Slot::one("order_id")],
        produces: &[Slot::one("receipt_id")],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND, ORDER_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetReceiptInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetReceipt {
    async fn read(&self, cx: &Cx, input: GetReceiptInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let o = visible_order(cx, &mut tx, &input.order_id).await?;
        let all = repo::receipts(&mut tx, &o.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let wanted = input.receipt_type.map(ReceiptType::as_str);
        let r = all
            .iter()
            .find(|r| wanted.is_none_or(|t| r.receipt_type == t))
            .ok_or_else(|| no_receipt(&o.id, wanted))?;
        let mut out = receipt_json(r, &o.order_number, &cx.time)?;
        out["other_receipts"] = json!(
            all.iter()
                .filter(|other| other.id != r.id)
                .map(|other| json!({"id": other.id, "receipt_type": other.receipt_type}))
                .collect::<Vec<_>>()
        );
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed_with_types_as_words() {
        let s = closed_schema_for::<GetReceiptInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(
            s["properties"]["receipt_type"]["enum"],
            json!(["deduction", "proof", "tax_invoice"])
        );
    }

    /// Integration (`DATABASE_URL`, demo seed): Jamie's proof receipt for #20381; Alex has no
    /// receipt for #20402 and is pointed at `issue_receipt`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn reads_jamies_receipt() {
        let jamie = crate::testdb::cx("cus_jamie").await;
        let r = GetReceipt
            .read(
                &jamie,
                GetReceiptInput {
                    order_id: "ord_20381".into(),
                    receipt_type: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(r["id"], "rct_20381");
        assert_eq!(r["receipt_type"], "proof");
        assert_eq!(r["amount"]["value"].to_string(), "64.00");
        let alex = crate::testdb::cx("cus_alex").await;
        let e = GetReceipt
            .read(
                &alex,
                GetReceiptInput {
                    order_id: "ord_20402".into(),
                    receipt_type: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("not_found", Some("issue_receipt"))
        );
    }
}
