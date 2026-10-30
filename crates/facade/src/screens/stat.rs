//! Statistics: the sales analysis grid of a period against a comparison basis.

use acom_contract::{OpMeta, ToolError};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{amt, screen};
use crate::codes::CmprBaseCd;
use crate::{Backend, Builder, Screen};

pub fn register(b: &mut Builder) {
    b.add(GetSaleAnlsGridList);
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SaleAnlsIn {
    /// Search start date.
    pub srch_strt_dt: String,
    /// Search end date.
    pub srch_end_dt: String,
    /// Compare base code.
    pub cmpr_base_cd: CmprBaseCd,
}

#[derive(Debug)]
pub struct GetSaleAnlsGridList;

impl Screen for GetSaleAnlsGridList {
    const META: OpMeta = screen!(
        "analytics.stat.getSaleAnlsGridList",
        "getSaleAnlsGridList",
        Operate,
        Read,
        Operator,
        SEARCH,
        "Sales analysis grid list."
    );
    type Input = SaleAnlsIn;

    async fn call(&self, be: &Backend<'_>, i: SaleAnlsIn) -> Result<Value, ToolError> {
        let out = be
            .read(
                "explain_sales_change",
                json!({"valid_from": i.srch_strt_dt, "valid_through": i.srch_end_dt,
                       "compare_mode": i.cmpr_base_cd.word()}),
            )
            .await?;
        let row = json!({
            "srchStrtDt": out["period"]["valid_from"], "srchEndDt": out["period"]["valid_through"],
            "saleAmt": amt(&out["period"]["revenue"]), "ordCnt": out["period"]["order_count"],
            "cmprSaleAmt": amt(&out["compare_to"]["revenue"]),
            "cmprOrdCnt": out["compare_to"]["order_count"],
            "diffAmt": amt(&out["gap"]), "diffRt": out["change_rate"],
        });
        Ok(json!({"list": [row]}))
    }
}
