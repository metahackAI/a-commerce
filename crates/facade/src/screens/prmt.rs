//! Promotions: the promotion grid and its count, the promotion list save (sale prices, pause,
//! end behind one row status), and the coupon list save.

use acom_contract::{OpMeta, ToolError};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Number, Value, json};

use super::{
    Paging, bad, count, count_only, grades, items, need, opt, proc_err, row_failed, saved, screen,
    tier_of,
};
use crate::codes::{
    AplyTgtCd, CpnAplyTpCd, DupAplyCd, Grades, PrmtStatCd, PrmtTpCd, RowStat, prmt_tp,
};
use crate::{Backend, Builder, Screen};

pub fn register(b: &mut Builder) {
    b.add(GetPrmtGridList)
        .add(GetPrmtGridListCnt)
        .add(SavePrmtList)
        .add(SaveCpnList);
}

// The promotion search form; the count screen takes the same form.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PrmtSrchIn {
    /// Promotion status code.
    pub prmt_stat_cd: Option<PrmtStatCd>,
    /// Promotion name.
    pub prmt_nm: Option<String>,
    /// Target member grade code.
    pub tgt_mbr_grd_cd: Option<String>,
    /// Search start date-time.
    pub srch_strt_dtm: Option<String>,
    /// Search end date-time.
    pub srch_end_dtm: Option<String>,
    /// Page no.
    pub page_no: Option<u32>,
    /// Page size.
    pub page_size: Option<u32>,
}

impl PrmtSrchIn {
    async fn query(&self, be: &Backend<'_>) -> Result<(Map<String, Value>, Grades), ToolError> {
        let g = grades(be).await?;
        let mut q = Map::new();
        opt(&mut q, "status", self.prmt_stat_cd.map(|c| vec![c.word()]));
        opt(&mut q, "query", self.prmt_nm.as_ref());
        if let Some(code) = &self.tgt_mbr_grd_cd {
            q.insert("valid_for_member_tier".into(), json!(tier_of(&g, code)?));
        }
        opt(&mut q, "valid_from", self.srch_strt_dtm.as_ref());
        opt(&mut q, "valid_through", self.srch_end_dtm.as_ref());
        Ok((q, g))
    }
}

#[derive(Debug)]
pub struct GetPrmtGridList;

impl Screen for GetPrmtGridList {
    const META: OpMeta = screen!(
        "promotion.prmt.getPrmtGridList",
        "getPrmtGridList",
        Engage,
        Read,
        Operator,
        SEARCH,
        "Promotion grid list."
    );
    type Input = PrmtSrchIn;

    async fn call(&self, be: &Backend<'_>, i: PrmtSrchIn) -> Result<Value, ToolError> {
        let paging = Paging::new(i.page_no, i.page_size)?;
        let (mut q, g) = i.query(be).await?;
        paging.apply(&mut q);
        let out = be.read("search_promotions", Value::Object(q)).await?;
        let list = items(&out)
            .iter()
            .map(|p| {
                let tier = p["valid_for_member_tier"].as_str();
                json!({
                    "prmtNo": p["id"], "prmtNm": p["name"],
                    "prmtTpCd": prmt_tp(p["discount_layer"].as_str().unwrap_or(""), tier),
                    "prmtStatCd": PrmtStatCd::of(&p["status"]),
                    "tgtMbrGrdCd": g.code(tier),
                    "strtDtm": p["valid_from"], "endDtm": p["valid_through"],
                })
            })
            .collect();
        Ok(paging.grid(list))
    }
}

#[derive(Debug)]
pub struct GetPrmtGridListCnt;

impl Screen for GetPrmtGridListCnt {
    const META: OpMeta = screen!(
        "promotion.prmt.getPrmtGridListCnt",
        "getPrmtGridListCnt",
        Engage,
        Read,
        Operator,
        SEARCH,
        "Promotion grid list count."
    );
    type Input = PrmtSrchIn;

    async fn call(&self, be: &Backend<'_>, i: PrmtSrchIn) -> Result<Value, ToolError> {
        let (mut q, _) = i.query(be).await?;
        count_only(&mut q);
        let out = be.read("search_promotions", Value::Object(q)).await?;
        Ok(count(&out))
    }
}

// One row of the promotion grid as the screen saves it.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PrmtRow {
    /// Row status.
    pub row_stat: RowStat,
    /// Promotion no.
    pub prmt_no: Option<String>,
    /// Promotion name.
    pub prmt_nm: Option<String>,
    /// Promotion type code.
    pub prmt_tp_cd: Option<PrmtTpCd>,
    /// Promotion status code.
    pub prmt_stat_cd: Option<PrmtStatCd>,
    /// Target member grade code.
    pub tgt_mbr_grd_cd: Option<String>,
    /// Start date-time.
    pub strt_dtm: Option<String>,
    /// End date-time.
    pub end_dtm: Option<String>,
    /// Apply target code.
    pub aply_tgt_cd: Option<AplyTgtCd>,
    /// Category no.
    pub ctg_no: Option<String>,
    /// Product no list.
    pub prd_no_list: Option<Vec<String>>,
    /// Discount rate.
    #[schemars(with = "Option<f64>")]
    pub dc_rt: Option<Number>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PrmtSaveIn {
    /// Grid rows.
    pub rows: Vec<PrmtRow>,
}

/// The back-end write one promotion row stands for.
fn prmt_call(r: &PrmtRow, g: &Grades) -> Result<(&'static str, Value), ToolError> {
    match r.row_stat {
        RowStat::I => {
            let tier = match need(&r.prmt_tp_cd, "prmtTpCd")? {
                PrmtTpCd::T01 => "everyone".to_owned(),
                PrmtTpCd::T03 => {
                    let tier = tier_of(g, need(&r.tgt_mbr_grd_cd, "tgtMbrGrdCd")?)?;
                    if tier == "everyone" {
                        return Err(bad("Invalid member grade code."));
                    }
                    tier
                }
                _ => return Err(proc_err("Promotion type not available on this screen.")),
            };
            let mut q = Map::new();
            q.insert("name".into(), json!(need(&r.prmt_nm, "prmtNm")?));
            q.insert("valid_for_member_tier".into(), json!(tier));
            q.insert("valid_from".into(), json!(need(&r.strt_dtm, "strtDtm")?));
            q.insert("valid_through".into(), json!(need(&r.end_dtm, "endDtm")?));
            q.insert("percent_off".into(), json!(need(&r.dc_rt, "dcRt")?));
            match need(&r.aply_tgt_cd, "aplyTgtCd")? {
                AplyTgtCd::C => {
                    q.insert("scope_mode".into(), json!("category"));
                    q.insert("category_id".into(), json!(need(&r.ctg_no, "ctgNo")?));
                }
                AplyTgtCd::P => {
                    q.insert("scope_mode".into(), json!("products"));
                    q.insert(
                        "product_ids".into(),
                        json!(need(&r.prd_no_list, "prdNoList")?),
                    );
                }
            }
            Ok(("set_sale_prices", Value::Object(q)))
        }
        RowStat::U => {
            let id = need(&r.prmt_no, "prmtNo")?;
            match need(&r.prmt_stat_cd, "prmtStatCd")? {
                PrmtStatCd::P30 => Ok(("pause_promotion", json!({"promotion_id": id}))),
                PrmtStatCd::P90 => Ok(("end_promotion", json!({"promotion_id": id}))),
                _ => Err(proc_err("Status change not available on this screen.")),
            }
        }
        RowStat::D => Ok((
            "end_promotion",
            json!({"promotion_id": need(&r.prmt_no, "prmtNo")?}),
        )),
    }
}

#[derive(Debug)]
pub struct SavePrmtList;

impl Screen for SavePrmtList {
    const META: OpMeta = screen!(
        "promotion.prmt.savePrmtList",
        "savePrmtList",
        Engage,
        Write,
        Operator,
        UPDATE,
        "Promotion list save."
    );
    type Input = PrmtSaveIn;

    async fn call(&self, be: &Backend<'_>, i: PrmtSaveIn) -> Result<Value, ToolError> {
        if i.rows.is_empty() {
            return Err(bad("No rows."));
        }
        let g = grades(be).await?;
        let mut out = Vec::new();
        for (n, r) in i.rows.iter().enumerate() {
            let done = async {
                let (tool, input) = prmt_call(r, &g)?;
                be.run(tool, input).await
            }
            .await
            .map_err(|e| row_failed(n + 1, out.len(), &e))?;
            out.push(json!({"rowNo": n + 1, "rowStat": r.row_stat.as_code(), "prmtNo": done.id}));
        }
        Ok(saved(out))
    }
}

// One row of the coupon grid as the screen saves it.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CpnRow {
    /// Row status.
    pub row_stat: RowStat,
    /// Coupon code.
    pub cpn_cd: Option<String>,
    /// Promotion no.
    pub prmt_no: Option<String>,
    /// Target member grade code.
    pub tgt_mbr_grd_cd: Option<String>,
    /// Coupon apply type code.
    pub cpn_aply_tp_cd: Option<CpnAplyTpCd>,
    /// Duplicate apply code.
    pub dup_aply_cd: Option<DupAplyCd>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CpnSaveIn {
    /// Grid rows.
    pub rows: Vec<CpnRow>,
}

fn cpn_call(r: &CpnRow, g: &Grades) -> Result<Value, ToolError> {
    if r.row_stat != RowStat::I {
        return Err(proc_err("Row status not available on this screen."));
    }
    let mut q = Map::new();
    q.insert("code".into(), json!(need(&r.cpn_cd, "cpnCd")?));
    q.insert("promotion_id".into(), json!(need(&r.prmt_no, "prmtNo")?));
    q.insert(
        "valid_for_member_tier".into(),
        json!(tier_of(g, need(&r.tgt_mbr_grd_cd, "tgtMbrGrdCd")?)?),
    );
    q.insert(
        "application".into(),
        json!(need(&r.cpn_aply_tp_cd, "cpnAplyTpCd")?.word()),
    );
    opt(&mut q, "stacking", r.dup_aply_cd.map(DupAplyCd::word));
    Ok(Value::Object(q))
}

#[derive(Debug)]
pub struct SaveCpnList;

impl Screen for SaveCpnList {
    const META: OpMeta = screen!(
        "promotion.cpn.saveCpnList",
        "saveCpnList",
        Engage,
        Write,
        Operator,
        CREATE,
        "Coupon list save."
    );
    type Input = CpnSaveIn;

    async fn call(&self, be: &Backend<'_>, i: CpnSaveIn) -> Result<Value, ToolError> {
        if i.rows.is_empty() {
            return Err(bad("No rows."));
        }
        let g = grades(be).await?;
        let mut out = Vec::new();
        for (n, r) in i.rows.iter().enumerate() {
            let done = async { be.run("create_coupon", cpn_call(r, &g)?).await }
                .await
                .map_err(|e| row_failed(n + 1, out.len(), &e))?;
            out.push(json!({"rowNo": n + 1, "rowStat": r.row_stat.as_code(), "cpnNo": done.id}));
        }
        Ok(saved(out))
    }
}
