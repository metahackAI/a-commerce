//! The shopper's side: the cart list and its save, and the order sheet init.

use acom_contract::{OpMeta, ToolError};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{amt, bad, need, opt, row_failed, saved, screen};
use crate::codes::{PayMthdCd, RowStat};
use crate::{Backend, Builder, Screen};

pub fn register(b: &mut Builder) {
    b.add(GetCartList).add(SaveCartList).add(GetOrdShtInitData);
}

/// Lines as the screens show them.
fn line_rows(lines: &Value) -> Vec<Value> {
    lines
        .as_array()
        .into_iter()
        .flatten()
        .map(|l| {
            json!({"cartSeq": l["cart_line_id"], "prdNo": l["product_id"], "optNo": l["variant_id"],
                   "prdNm": l["name"], "optVal1": l["color"], "optVal2": l["size"],
                   "ordQty": l["quantity"], "salePrc": amt(&l["price"])})
        })
        .collect()
}

fn discounts(d: &Value) -> Vec<Value> {
    d.as_array()
        .into_iter()
        .flatten()
        .map(|x| json!({"dcNm": x["label"], "dcAmt": amt(&x["amount"])}))
        .collect()
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CartIn {}

#[derive(Debug)]
pub struct GetCartList;

impl Screen for GetCartList {
    const META: OpMeta = screen!(
        "sales.cart.getCartList",
        "getCartList",
        Transact,
        Read,
        Shopper,
        SEARCH,
        "Cart list."
    );
    type Input = CartIn;

    async fn call(&self, be: &Backend<'_>, _: CartIn) -> Result<Value, ToolError> {
        let c = be.read("get_cart", json!({})).await?;
        Ok(json!({
            "cartNo": c["id"], "list": line_rows(&c["lines"]),
            "totPrdAmt": amt(&c["subtotal"]), "dcList": discounts(&c["discounts"]),
            "dlvCst": amt(&c["shipping"]), "totPayAmt": amt(&c["total"]),
            "dlvAddrNo": c["delivery"]["address_id"], "arrvDt": c["delivery"]["arrives"],
            "payMthdCd": PayMthdCd::of(&c["payment"]["payment_method"]),
        }))
    }
}

// One row of the cart grid as the screen saves it.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CartRow {
    /// Row status.
    pub row_stat: RowStat,
    /// Cart seq.
    pub cart_seq: Option<String>,
    /// Product no.
    pub prd_no: Option<String>,
    /// Option no.
    pub opt_no: Option<String>,
    /// Option value 1.
    pub opt_val1: Option<String>,
    /// Option value 2.
    pub opt_val2: Option<String>,
    /// Order qty.
    pub ord_qty: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CartSaveIn {
    /// Grid rows.
    pub rows: Vec<CartRow>,
}

fn cart_call(r: &CartRow) -> Result<(&'static str, Value), ToolError> {
    let mut q = Map::new();
    match r.row_stat {
        RowStat::I => {
            if let Some(v) = &r.opt_no {
                q.insert("mode".into(), json!("variant"));
                q.insert("variant_id".into(), json!(v));
            } else {
                q.insert("mode".into(), json!("product_options"));
                q.insert("product_id".into(), json!(need(&r.prd_no, "prdNo")?));
                q.insert("color".into(), json!(need(&r.opt_val1, "optVal1")?));
                q.insert("size".into(), json!(need(&r.opt_val2, "optVal2")?));
            }
            opt(&mut q, "order_quantity", r.ord_qty);
            Ok(("add_to_cart", Value::Object(q)))
        }
        RowStat::U => {
            q.insert("cart_line_id".into(), json!(need(&r.cart_seq, "cartSeq")?));
            opt(&mut q, "order_quantity", r.ord_qty);
            opt(&mut q, "color", r.opt_val1.as_ref());
            opt(&mut q, "size", r.opt_val2.as_ref());
            Ok(("update_cart_item", Value::Object(q)))
        }
        RowStat::D => {
            q.insert("cart_line_id".into(), json!(need(&r.cart_seq, "cartSeq")?));
            Ok(("remove_cart_item", Value::Object(q)))
        }
    }
}

#[derive(Debug)]
pub struct SaveCartList;

impl Screen for SaveCartList {
    const META: OpMeta = screen!(
        "sales.cart.saveCartList",
        "saveCartList",
        Transact,
        Write,
        Shopper,
        ADD,
        "Cart list save."
    );
    type Input = CartSaveIn;

    async fn call(&self, be: &Backend<'_>, i: CartSaveIn) -> Result<Value, ToolError> {
        if i.rows.is_empty() {
            return Err(bad("No rows."));
        }
        let mut out = Vec::new();
        for (n, r) in i.rows.iter().enumerate() {
            let done = async {
                let (tool, input) = cart_call(r)?;
                be.run(tool, input).await
            }
            .await
            .map_err(|e| row_failed(n + 1, out.len(), &e))?;
            out.push(json!({"rowNo": n + 1, "rowStat": r.row_stat.as_code(),
                            "cartSeq": done.produced.get("cart_line_id").unwrap_or(&json!(done.id))}));
        }
        Ok(saved(out))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OrdShtIn {
    /// Delivery address no.
    pub dlv_addr_no: Option<String>,
    /// Pay method code.
    pub pay_mthd_cd: Option<PayMthdCd>,
}

#[derive(Debug)]
pub struct GetOrdShtInitData;

impl Screen for GetOrdShtInitData {
    const META: OpMeta = screen!(
        "sales.ord.getOrdShtInitData",
        "getOrdShtInitData",
        Transact,
        Read,
        Shopper,
        ORDER,
        "Order sheet init data."
    );
    type Input = OrdShtIn;

    /// Loads the cart and prepares the order from it, as an order sheet page does. The sheet
    /// number is the prepared order: only the shopper's own button can place it.
    async fn call(&self, be: &Backend<'_>, i: OrdShtIn) -> Result<Value, ToolError> {
        let cart = be.read("get_cart", json!({})).await?;
        let address = i
            .dlv_addr_no
            .clone()
            .or_else(|| cart["delivery"]["address_id"].as_str().map(str::to_owned))
            .ok_or_else(|| bad("`dlvAddrNo` is required."))?;
        let mut q = Map::new();
        q.insert("delivery_mode".into(), json!("saved"));
        q.insert("delivery_address_id".into(), json!(address));
        opt(&mut q, "payment_method", i.pay_mthd_cd.map(PayMthdCd::word));
        q.insert("client_total".into(), cart["total"].clone());
        let p = be.prepare("prepare_order", Value::Object(q)).await?;
        let order = p.changes.first().map(|c| c.to.clone()).unwrap_or_default();
        Ok(json!({
            "ordShtNo": p.preview_id, "expDtm": p.expires_at,
            "list": line_rows(&order["lines"]),
            "totPrdAmt": amt(&order["subtotal"]), "dcList": discounts(&order["discounts"]),
            "dlvCst": amt(&order["shipping"]), "totPayAmt": amt(&order["total"]),
            "dlvAddrNm": order["delivery"]["address_label"], "arrvDt": order["delivery"]["arrives"],
            "payMthdCd": PayMthdCd::of(&order["payment"]["payment_method"]),
        }))
    }
}
