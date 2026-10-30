//! Members: the grade list, and the promotion target member list with its count.

use acom_contract::{OpMeta, ToolError};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{Paging, count, count_only, grades, items, opt, screen, tier_of};
use crate::codes::{Grades, MbrStatCd};
use crate::{Backend, Builder, Screen};

pub fn register(b: &mut Builder) {
    b.add(GetMbrGrdList)
        .add(GetPrmtTgtMbrList)
        .add(GetPrmtTgtMbrListCnt);
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct MbrGrdIn {
    /// Program no.
    pub prgm_no: Option<String>,
}

#[derive(Debug)]
pub struct GetMbrGrdList;

impl Screen for GetMbrGrdList {
    const META: OpMeta = screen!(
        "customer.mbr.getMbrGrdList",
        "getMbrGrdList",
        Engage,
        Read,
        Operator,
        SEARCH,
        "Member grade list."
    );
    type Input = MbrGrdIn;

    async fn call(&self, be: &Backend<'_>, i: MbrGrdIn) -> Result<Value, ToolError> {
        let mut q = Map::new();
        q.insert("limit".into(), json!(100));
        opt(&mut q, "program_id", i.prgm_no.as_ref());
        let out = be.read("list_tiers", Value::Object(q)).await?;
        let list = items(&out)
            .iter()
            .map(|t| {
                json!({"mbrGrdCd": format!("G{:02}", t["rank"].as_i64().unwrap_or(0)),
                       "mbrGrdNm": t["name"], "mbrCnt": t["member_count"], "sortSeq": t["rank"]})
            })
            .collect::<Vec<_>>();
        Ok(json!({"list": list}))
    }
}

// The target member search form; the count screen takes the same form. (A type doc would
// become the schema description, so input types use plain comments.)
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct TgtMbrIn {
    /// Member grade code.
    pub mbr_grd_cd: Option<String>,
    /// Member status code.
    pub mbr_stat_cd: Option<MbrStatCd>,
    /// Search keyword.
    pub srch_kwd: Option<String>,
    /// Page no.
    pub page_no: Option<u32>,
    /// Page size.
    pub page_size: Option<u32>,
}

impl TgtMbrIn {
    async fn query(&self, be: &Backend<'_>) -> Result<(Map<String, Value>, Grades), ToolError> {
        let g = grades(be).await?;
        let mut q = Map::new();
        if let Some(code) = &self.mbr_grd_cd {
            let tier = tier_of(&g, code)?;
            // Every member is a promotion target for "every grade".
            if tier != "everyone" {
                q.insert("tier_id".into(), json!(tier));
            }
        }
        opt(&mut q, "state", self.mbr_stat_cd.map(MbrStatCd::word));
        opt(&mut q, "query", self.srch_kwd.as_ref());
        Ok((q, g))
    }
}

#[derive(Debug)]
pub struct GetPrmtTgtMbrList;

impl Screen for GetPrmtTgtMbrList {
    const META: OpMeta = screen!(
        "customer.mbr.getPrmtTgtMbrList",
        "getPrmtTgtMbrList",
        Engage,
        Read,
        Operator,
        SEARCH,
        "Promotion target member list."
    );
    type Input = TgtMbrIn;

    async fn call(&self, be: &Backend<'_>, i: TgtMbrIn) -> Result<Value, ToolError> {
        let paging = Paging::new(i.page_no, i.page_size)?;
        let (mut q, g) = i.query(be).await?;
        paging.apply(&mut q);
        let out = be.read("search_members", Value::Object(q)).await?;
        let list = items(&out)
            .iter()
            .map(|m| {
                json!({"mbrNo": m["id"], "mbrNm": m["name"], "emailAddr": m["email"],
                       "mbrGrdCd": g.code(m["tier_id"].as_str()),
                       "mbrStatCd": MbrStatCd::of(&m["state"])})
            })
            .collect();
        Ok(paging.grid(list))
    }
}

#[derive(Debug)]
pub struct GetPrmtTgtMbrListCnt;

impl Screen for GetPrmtTgtMbrListCnt {
    const META: OpMeta = screen!(
        "customer.mbr.getPrmtTgtMbrListCnt",
        "getPrmtTgtMbrListCnt",
        Engage,
        Read,
        Operator,
        SEARCH,
        "Promotion target member list count."
    );
    type Input = TgtMbrIn;

    async fn call(&self, be: &Backend<'_>, i: TgtMbrIn) -> Result<Value, ToolError> {
        let (mut q, _) = i.query(be).await?;
        count_only(&mut q);
        let out = be.read("search_members", Value::Object(q)).await?;
        Ok(count(&out))
    }
}
