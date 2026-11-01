//! Orders. The shopper's side: the cart list and its save, and the order sheet init. The
//! operator's side: the order grid and its count, and the order detail popup.

use acom_contract::{OpMeta, ToolError};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::clm::clm_tp;
use super::{Paging, amt, bad, count, count_only, items, need, opt, row_failed, saved, screen};
use crate::codes::{ClmStatCd, DlvStatCd, OrdStatCd, PayMthdCd, RowStat};
use crate::{Backend, Builder, Screen};

pub fn register(b: &mut Builder) {
    b.add(GetCartList)
        .add(SaveCartList)
        .add(GetOrdShtInitData)
        .add(GetOrdGridList)
        .add(GetOrdGridListCnt)
        .add(GetOrdDtlPopInitData);
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

// The operator's order search form; the count screen takes the same form.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OrdSrchIn {
    /// Order status code.
    pub ord_stat_cd: Option<OrdStatCd>,
    /// Order date from.
    pub ord_dtm_from: Option<String>,
    /// Order date to.
    pub ord_dtm_to: Option<String>,
    /// Member no.
    pub mbr_no: Option<String>,
    /// Search keyword.
    pub srch_kwd: Option<String>,
    /// Page no.
    pub page_no: Option<u32>,
    /// Page size.
    pub page_size: Option<u32>,
}

impl OrdSrchIn {
    fn query(&self) -> Map<String, Value> {
        let mut q = Map::new();
        opt(
            &mut q,
            "order_status",
            self.ord_stat_cd.map(|c| vec![c.word()]),
        );
        opt(&mut q, "order_date_from", self.ord_dtm_from.as_ref());
        opt(&mut q, "order_date_to", self.ord_dtm_to.as_ref());
        opt(&mut q, "customer_id", self.mbr_no.as_ref());
        // One search box, as order grids have: a number is an order number, anything else the
        // orderer.
        if let Some(k) = self
            .srch_kwd
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
        {
            let number = k.trim_start_matches('#');
            if !number.is_empty() && number.bytes().all(|b| b.is_ascii_digit()) {
                q.insert("order_number".into(), json!(number));
            } else {
                q.insert("customer".into(), json!(k));
            }
        }
        q
    }
}

#[derive(Debug)]
pub struct GetOrdGridList;

impl Screen for GetOrdGridList {
    const META: OpMeta = screen!(
        "sales.ord.getOrdGridList",
        "getOrdGridList",
        Transact,
        Read,
        Operator,
        SEARCH,
        "Order grid list."
    );
    type Input = OrdSrchIn;

    async fn call(&self, be: &Backend<'_>, i: OrdSrchIn) -> Result<Value, ToolError> {
        let paging = Paging::new(i.page_no, i.page_size)?;
        let mut q = i.query();
        paging.apply(&mut q);
        let out = be.read("search_orders", Value::Object(q)).await?;
        let list = items(&out)
            .iter()
            .map(|o| {
                json!({"ordNo": o["id"], "ordDtm": o["order_date"], "mbrNo": o["customer_id"],
                       "ordAmt": amt(&o["total"]), "ordStatCd": OrdStatCd::of(&o["order_status"])})
            })
            .collect();
        Ok(paging.grid(list))
    }
}

#[derive(Debug)]
pub struct GetOrdGridListCnt;

impl Screen for GetOrdGridListCnt {
    const META: OpMeta = screen!(
        "sales.ord.getOrdGridListCnt",
        "getOrdGridListCnt",
        Transact,
        Read,
        Operator,
        SEARCH,
        "Order grid list count."
    );
    type Input = OrdSrchIn;

    async fn call(&self, be: &Backend<'_>, i: OrdSrchIn) -> Result<Value, ToolError> {
        let mut q = i.query();
        count_only(&mut q);
        let out = be.read("search_orders", Value::Object(q)).await?;
        Ok(count(&out))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct OrdNoIn {
    /// Order no.
    pub ord_no: String,
}

/// When the parcel left: its first carrier scan.
fn dlv_start(events: &Value) -> Value {
    events
        .as_array()
        .into_iter()
        .flatten()
        .filter(|e| {
            matches!(
                e["delivery_state"].as_str(),
                Some("picked_up" | "in_transit")
            )
        })
        .filter_map(|e| e["at"].as_str())
        .min()
        .map_or(Value::Null, Value::from)
}

#[derive(Debug)]
pub struct GetOrdDtlPopInitData;

impl Screen for GetOrdDtlPopInitData {
    const META: OpMeta = screen!(
        "sales.ord.getOrdDtlPopInitData",
        "getOrdDtlPopInitData",
        Transact,
        Read,
        Operator,
        SEARCH,
        "Order detail popup init data."
    );
    type Input = OrdNoIn;

    /// The order, its lines, the parcel and the claims on it, as one popup loads them.
    async fn call(&self, be: &Backend<'_>, i: OrdNoIn) -> Result<Value, ToolError> {
        let o = be.read("get_order", json!({"order_id": i.ord_no})).await?;
        let d = &o["delivery"];
        let parcel = match d["id"].as_str() {
            Some(id) => be.read("get_delivery", json!({"delivery_id": id})).await?,
            None => Value::Null,
        };
        let addr = &d["delivery_address"];
        let city: Vec<&str> = ["address_locality", "address_region", "address_country"]
            .iter()
            .filter_map(|k| addr[*k].as_str())
            .collect();
        let lines: Vec<Value> = o["ordered_item"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|l| {
                json!({"ordSeq": l["id"], "prdNo": l["product_id"], "optNo": l["variant_id"],
                       "prdNm": l["name"], "optVal1": l["color"], "optVal2": l["size"],
                       "ordQty": l["order_quantity"], "salePrc": amt(&l["price"]),
                       "dcAmt": amt(&l["discount"]),
                       "ordDtlStatCd": OrdStatCd::of(&l["order_item_status"])})
            })
            .collect();
        let claims: Vec<Value> = o["claims"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| {
                json!({"clmNo": c["id"], "clmTpCd": clm_tp(&c["claim_type"]),
                       "clmStatCd": ClmStatCd::of(&c["claim_status"]),
                       "clmReqDtm": c["requested_at"]})
            })
            .collect();
        Ok(json!({
            "ordNo": o["id"], "ordDtm": o["order_date"], "mbrNo": o["customer_id"],
            "ordStatCd": OrdStatCd::of(&o["order_status"]),
            "totPrdAmt": amt(&o["totals"]["subtotal"]), "dlvCst": amt(&o["totals"]["shipping"]),
            "totPayAmt": amt(&o["totals"]["total"]),
            "payMthdCd": PayMthdCd::of(&o["payment"]["payment_method"]),
            "rcvrNm": addr["name"], "zipNo": addr["postal_code"],
            "baseAddr": addr["street_address"], "dtlAddr": city.join(", "),
            "dlvStatCd": DlvStatCd::of(&d["delivery_state"]),
            "dlvCoNm": parcel["carrier"], "invcNo": parcel["tracking_number"],
            "dlvStrtDtm": dlv_start(&parcel["events"]), "dlvCmplDtm": d["delivered_at"],
            "prdList": lines, "clmList": claims,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn search(kwd: &str) -> Map<String, Value> {
        OrdSrchIn {
            ord_stat_cd: None,
            ord_dtm_from: None,
            ord_dtm_to: None,
            mbr_no: None,
            srch_kwd: Some(kwd.into()),
            page_no: None,
            page_size: None,
        }
        .query()
    }

    #[test]
    fn the_search_box_takes_an_order_number_or_the_orderer() {
        assert_eq!(search("#20402")["order_number"], "20402");
        assert_eq!(search(" 27184 ")["order_number"], "27184");
        assert_eq!(search("Alex Kim")["customer"], "Alex Kim");
        assert!(search("  ").is_empty());
    }

    #[test]
    fn the_parcel_left_at_its_first_carrier_scan() {
        let events = json!([
            {"at": "2026-11-01 09:00", "delivery_state": "packed"},
            {"at": "2026-11-02 18:10", "delivery_state": "in_transit"},
            {"at": "2026-11-02 16:40", "delivery_state": "picked_up"},
        ]);
        assert_eq!(dlv_start(&events), "2026-11-02 16:40");
        assert_eq!(dlv_start(&Value::Null), Value::Null);
    }
}
