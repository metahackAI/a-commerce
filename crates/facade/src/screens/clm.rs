//! Claims: the claim grid and its count, the refund popup (init and save), and the customer
//! message send.

use acom_contract::{OpMeta, ToolError};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Number, Value, json};

use super::{Paging, amt, count, count_only, items, opt, proc_err, screen};
use crate::codes::{
    ClmRsnCd, ClmStatCd, MsgTpCd, PayMthdCd, RfndMthdCd, RspbCd, RtnDlvCstCd, SndChnlCd,
};
use crate::{Backend, Builder, Screen};

pub fn register(b: &mut Builder) {
    b.add(GetClmGridList)
        .add(GetClmGridListCnt)
        .add(GetRfndPopInitData)
        .add(SaveRfndProc)
        .add(SaveCustMsgSend);
}

// The claim search form; the count screen takes the same form.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ClmSrchIn {
    /// Claim status code.
    pub clm_stat_cd: Option<ClmStatCd>,
    /// Claim reason code.
    pub clm_rsn_cd: Option<ClmRsnCd>,
    /// Claim amount from.
    #[schemars(with = "Option<f64>")]
    pub clm_amt_from: Option<Number>,
    /// Page no.
    pub page_no: Option<u32>,
    /// Page size.
    pub page_size: Option<u32>,
}

impl ClmSrchIn {
    fn query(&self, be: &Backend<'_>) -> Map<String, Value> {
        let mut q = Map::new();
        opt(&mut q, "status", self.clm_stat_cd.map(|c| vec![c.word()]));
        opt(&mut q, "reason", self.clm_rsn_cd.map(|c| vec![c.word()]));
        if let Some(n) = &self.clm_amt_from {
            q.insert(
                "min_amount".into(),
                json!({"value": n, "currency": be.cx().currency.as_str()}),
            );
        }
        q
    }
}

#[derive(Debug)]
pub struct GetClmGridList;

impl Screen for GetClmGridList {
    const META: OpMeta = screen!(
        "fulfillment.clm.getClmGridList",
        "getClmGridList",
        Transact,
        Read,
        Operator,
        SEARCH,
        "Claim grid list."
    );
    type Input = ClmSrchIn;

    async fn call(&self, be: &Backend<'_>, i: ClmSrchIn) -> Result<Value, ToolError> {
        let paging = Paging::new(i.page_no, i.page_size)?;
        let mut q = i.query(be);
        paging.apply(&mut q);
        let out = be.read("search_return_requests", Value::Object(q)).await?;
        let list = items(&out)
            .iter()
            .map(|c| {
                json!({"clmNo": c["claim_id"], "ordNo": c["order_id"], "mbrNo": c["customer_id"],
                       "prdNm": c["item"], "clmQty": c["quantity"], "clmAmt": amt(&c["amount"]),
                       "clmRsnCd": ClmRsnCd::of(&c["reason"]), "clmRsnCntt": c["reason_text"],
                       "atchFileCnt": c["photos"], "dlvCmplDt": c["delivered_at"],
                       "clmReqDtm": c["requested_at"], "clmStatCd": ClmStatCd::of(&c["status"])})
            })
            .collect();
        Ok(paging.grid(list))
    }
}

#[derive(Debug)]
pub struct GetClmGridListCnt;

impl Screen for GetClmGridListCnt {
    const META: OpMeta = screen!(
        "fulfillment.clm.getClmGridListCnt",
        "getClmGridListCnt",
        Transact,
        Read,
        Operator,
        SEARCH,
        "Claim grid list count."
    );
    type Input = ClmSrchIn;

    async fn call(&self, be: &Backend<'_>, i: ClmSrchIn) -> Result<Value, ToolError> {
        let mut q = i.query(be);
        count_only(&mut q);
        let out = be.read("search_return_requests", Value::Object(q)).await?;
        Ok(count(&out))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct ClmNoIn {
    /// Claim no.
    pub clm_no: String,
}

fn clm_tp(t: &Value) -> &'static str {
    match t.as_str() {
        Some("return") => "R",
        Some("cancel") => "C",
        Some("exchange") => "X",
        _ => "9",
    }
}

#[derive(Debug)]
pub struct GetRfndPopInitData;

impl Screen for GetRfndPopInitData {
    const META: OpMeta = screen!(
        "fulfillment.rfnd.getRfndPopInitData",
        "getRfndPopInitData",
        Transact,
        Read,
        Operator,
        SEARCH,
        "Refund popup init data."
    );
    type Input = ClmNoIn;

    async fn call(&self, be: &Backend<'_>, i: ClmNoIn) -> Result<Value, ToolError> {
        let c = be.read("get_claim", json!({"claim_id": i.clm_no})).await?;
        let o = be
            .read("get_order", json!({"order_id": c["order_id"]}))
            .await?;
        let claimed: Vec<&Value> = c["items"].as_array().into_iter().flatten().collect();
        let lines: Vec<Value> = o["ordered_item"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|l| {
                let q = claimed
                    .iter()
                    .find(|x| x["order_item_id"] == l["id"])?
                    .get("quantity")?;
                Some(
                    json!({"ordSeq": l["id"], "prdNo": l["product_id"], "prdNm": l["name"],
                            "optVal1": l["color"], "optVal2": l["size"],
                            "ordQty": l["order_quantity"], "clmQty": q,
                            "salePrc": amt(&l["price"]), "dcAmt": amt(&l["discount"])}),
                )
            })
            .collect();
        Ok(json!({
            "clmNo": c["id"], "clmTpCd": clm_tp(&c["claim_type"]),
            "clmStatCd": ClmStatCd::of(&c["status"]),
            "ordNo": c["order_id"], "mbrNo": c["customer_id"],
            "clmRsnCd": ClmRsnCd::of(&c["reason"]), "clmRsnCntt": c["reason_text"],
            "rspbCd": RspbCd::of(&c["liability"]),
            "rtnDlvCstCd": RtnDlvCstCd::of(&c["return_fees"]),
            "clmAmt": amt(&c["amount"]), "rfndAmt": amt(&c["refund_estimate"]),
            "atchFileCnt": c["evidence"].as_array().map_or(0, Vec::len),
            "clmReqDtm": c["requested_at"], "rfndNo": c["refund_id"],
            "dlvCmplDtm": o["delivery"]["delivered_at"],
            "payMthdCd": PayMthdCd::of(&o["payment"]["payment_method"]),
            "cardNo": o["payment"]["card_last4"],
            "prdList": lines,
        }))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct RfndSaveIn {
    /// Claim no.
    pub clm_no: String,
    /// Refund method code.
    pub rfnd_mthd_cd: Option<RfndMthdCd>,
}

#[derive(Debug)]
pub struct SaveRfndProc;

impl Screen for SaveRfndProc {
    const META: OpMeta = screen!(
        "fulfillment.rfnd.saveRfndProc",
        "saveRfndProc",
        Transact,
        Write,
        Operator,
        PAY,
        "Refund process save."
    );
    type Input = RfndSaveIn;

    async fn call(&self, be: &Backend<'_>, i: RfndSaveIn) -> Result<Value, ToolError> {
        if i.rfnd_mthd_cd.is_some_and(|m| m != RfndMthdCd::R01) {
            return Err(proc_err("Refund method not available."));
        }
        let done = be
            .run("refund_return", json!({"claim_id": i.clm_no}))
            .await?;
        Ok(json!({"resultCd": "0000", "resultMsg": "Processed.", "rfndNo": done.id}))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MsgSendIn {
    /// Member no.
    pub mbr_no: String,
    /// Order no.
    pub ord_no: Option<String>,
    /// Message type code.
    pub msg_tp_cd: MsgTpCd,
    /// Send channel code.
    pub snd_chnl_cd: Option<SndChnlCd>,
    /// Message title.
    pub msg_titl: Option<String>,
    /// Message content.
    pub msg_cntt: String,
}

#[derive(Debug)]
pub struct SaveCustMsgSend;

impl Screen for SaveCustMsgSend {
    const META: OpMeta = screen!(
        "messaging.msg.saveCustMsgSend",
        "saveCustMsgSend",
        Engage,
        Write,
        Operator,
        SEND,
        "Customer message send save."
    );
    type Input = MsgSendIn;

    async fn call(&self, be: &Backend<'_>, i: MsgSendIn) -> Result<Value, ToolError> {
        let mut q = Map::new();
        q.insert("customer_id".into(), json!(i.mbr_no));
        opt(&mut q, "order_id", i.ord_no.as_ref());
        q.insert("kind".into(), json!(i.msg_tp_cd.word()));
        opt(&mut q, "channel", i.snd_chnl_cd.map(SndChnlCd::word));
        opt(&mut q, "subject", i.msg_titl.as_ref());
        q.insert("text".into(), json!(i.msg_cntt));
        let done = be.run("send_customer_message", Value::Object(q)).await?;
        Ok(json!({"resultCd": "0000", "resultMsg": "Sent.", "msgNo": done.id}))
    }
}
