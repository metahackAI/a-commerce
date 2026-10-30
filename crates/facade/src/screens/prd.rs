//! Products: the product grid and its count, and the product detail in two versions.

use acom_contract::{OpMeta, ToolError};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{Paging, count, count_only, grades, items, opt, screen, tier_of};
use crate::codes::SellStatCd;
use crate::{Backend, Builder, Screen};

pub fn register(b: &mut Builder) {
    b.add(GetPrdGridList)
        .add(GetPrdGridListCnt)
        .add(GetPrdDtlInfo)
        .add(GetPrdDtlInfoV2);
}

// The product search form; the count screen takes the same form. (A type doc would become
// the schema description, so input types use plain comments.)
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PrdSrchIn {
    /// Search keyword.
    pub srch_kwd: Option<String>,
    /// Category no.
    pub ctg_no: Option<String>,
    /// Sell status code.
    pub sell_stat_cd: Option<SellStatCd>,
    /// Stock qty from.
    pub stk_qty_from: Option<i64>,
    /// Stock qty to.
    pub stk_qty_to: Option<i64>,
    /// Apply member grade code.
    pub aply_mbr_grd_cd: Option<String>,
    /// Apply date-time.
    pub aply_dtm: Option<String>,
    /// Page no.
    pub page_no: Option<u32>,
    /// Page size.
    pub page_size: Option<u32>,
}

impl PrdSrchIn {
    async fn query(&self, be: &Backend<'_>) -> Result<Map<String, Value>, ToolError> {
        let mut q = Map::new();
        opt(&mut q, "query", self.srch_kwd.as_ref());
        opt(&mut q, "category_id", self.ctg_no.as_ref());
        opt(
            &mut q,
            "availability",
            self.sell_stat_cd.map(|c| vec![c.word()]),
        );
        opt(&mut q, "min_inventory_level", self.stk_qty_from);
        opt(&mut q, "max_inventory_level", self.stk_qty_to);
        opt(&mut q, "as_of", self.aply_dtm.as_ref());
        if let Some(code) = &self.aply_mbr_grd_cd {
            let g = grades(be).await?;
            q.insert("valid_for_member_tier".into(), json!(tier_of(&g, code)?));
        }
        Ok(q)
    }
}

/// A product as a grid row.
fn prd_row(p: &Value) -> Value {
    let mut row = json!({
        "prdNo": p.get("id").or_else(|| p.get("product_id")),
        "prdNm": p["name"],
        "ctgNo": p["category_id"],
        "brndNo": p["brand_id"],
        "salePrc": p["price"],
        "mbrPrc": p["member_price"]["price"],
        "stkQty": p["inventory_level"],
        "sellStatCd": SellStatCd::of(&p["availability"]),
    });
    if let Some(c) = p.get("cost") {
        row["costPrc"] = c.clone();
    }
    row
}

#[derive(Debug)]
pub struct GetPrdGridList;

impl Screen for GetPrdGridList {
    const META: OpMeta = screen!(
        "search.prd.getPrdGridList",
        "getPrdGridList",
        Offer,
        Read,
        Both,
        SEARCH,
        "Product grid list."
    );
    type Input = PrdSrchIn;

    async fn call(&self, be: &Backend<'_>, i: PrdSrchIn) -> Result<Value, ToolError> {
        let paging = Paging::new(i.page_no, i.page_size)?;
        let mut q = i.query(be).await?;
        paging.apply(&mut q);
        let out = be.read("search_products", Value::Object(q)).await?;
        Ok(paging.grid(items(&out).iter().map(prd_row).collect()))
    }
}

#[derive(Debug)]
pub struct GetPrdGridListCnt;

impl Screen for GetPrdGridListCnt {
    const META: OpMeta = screen!(
        "search.prd.getPrdGridListCnt",
        "getPrdGridListCnt",
        Offer,
        Read,
        Both,
        SEARCH,
        "Product grid list count."
    );
    type Input = PrdSrchIn;

    async fn call(&self, be: &Backend<'_>, i: PrdSrchIn) -> Result<Value, ToolError> {
        let mut q = i.query(be).await?;
        count_only(&mut q);
        let out = be.read("search_products", Value::Object(q)).await?;
        Ok(count(&out))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PrdNoIn {
    /// Product no.
    pub prd_no: String,
}

async fn product(be: &Backend<'_>, prd_no: &str) -> Result<Value, ToolError> {
    be.read("get_product", json!({"product_id": prd_no})).await
}

fn detail(p: &Value) -> Value {
    let mut d = prd_row(p);
    d["prdStatCd"] = p["approval_status"].clone();
    d
}

#[derive(Debug)]
pub struct GetPrdDtlInfo;

impl Screen for GetPrdDtlInfo {
    const META: OpMeta = screen!(
        "catalog.prd.getPrdDtlInfo",
        "getPrdDtlInfo",
        Offer,
        Read,
        Both,
        SEARCH,
        "Product detail info."
    );
    type Input = PrdNoIn;

    async fn call(&self, be: &Backend<'_>, i: PrdNoIn) -> Result<Value, ToolError> {
        Ok(detail(&product(be, &i.prd_no).await?))
    }
}

#[derive(Debug)]
pub struct GetPrdDtlInfoV2;

impl Screen for GetPrdDtlInfoV2 {
    const META: OpMeta = screen!(
        "catalog.prd.getPrdDtlInfoV2",
        "getPrdDtlInfoV2",
        Offer,
        Read,
        Both,
        SEARCH,
        "Product detail info (v2)."
    );
    type Input = PrdNoIn;

    async fn call(&self, be: &Backend<'_>, i: PrdNoIn) -> Result<Value, ToolError> {
        let p = product(be, &i.prd_no).await?;
        let mut d = detail(&p);
        d["optList"] = p["has_variant"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|v| {
                json!({"optNo": v["variant_id"], "optVal1": v["color"], "optVal2": v["size"],
                       "stkQty": v["inventory_level"],
                       "sellStatCd": SellStatCd::of(&v["availability"])})
            })
            .collect();
        d["attrList"] = p["additional_property"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|a| json!({"attrNm": a["name"], "attrVal": a["value"], "unitCd": a.get("unit_code")}))
            .collect();
        Ok(d)
    }
}
