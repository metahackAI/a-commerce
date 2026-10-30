//! Display: the corner list of a page and the banner list save.

use acom_contract::{OpMeta, ToolError};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use super::{bad, grades, items, need, opt, proc_err, row_failed, saved, screen, tier_of};
use crate::codes::{CornerTpCd, DispPageCd, DispStatCd, Grades, RowStat};
use crate::{Backend, Builder, Screen};

pub fn register(b: &mut Builder) {
    b.add(GetDispCornerList).add(SaveDispBnrList);
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct CornerIn {
    /// Display page code.
    pub disp_page_cd: DispPageCd,
    /// Start date-time.
    pub strt_dtm: String,
    /// End date-time.
    pub end_dtm: String,
    /// Corner type code.
    pub corner_tp_cd: Option<CornerTpCd>,
}

#[derive(Debug)]
pub struct GetDispCornerList;

impl Screen for GetDispCornerList {
    const META: OpMeta = screen!(
        "merchandising.disp.getDispCornerList",
        "getDispCornerList",
        Offer,
        Read,
        Operator,
        SEARCH,
        "Display corner list."
    );
    type Input = CornerIn;

    async fn call(&self, be: &Backend<'_>, i: CornerIn) -> Result<Value, ToolError> {
        let g = grades(be).await?;
        let mut q = Map::new();
        q.insert("page".into(), json!(i.disp_page_cd.word()));
        q.insert("valid_from".into(), json!(i.strt_dtm));
        q.insert("valid_through".into(), json!(i.end_dtm));
        q.insert("limit".into(), json!(100));
        opt(&mut q, "slot_type", i.corner_tp_cd.map(CornerTpCd::word));
        let out = be.read("list_display_slots", Value::Object(q)).await?;
        let list: Vec<Value> = items(&out)
            .iter()
            .map(|s| {
                let banners: Vec<Value> = s["bookings"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|b| {
                        json!({"dispNo": b["display_booking_id"],
                               "tgtMbrGrdCd": g.code(b["audience"].as_str()),
                               "bnrTitl": b["headline"],
                               "strtDtm": b["valid_from"], "endDtm": b["valid_through"],
                               "dispStatCd": DispStatCd::of(&b["status"])})
                    })
                    .collect();
                json!({"cornerNo": s["id"], "cornerNm": s["name"],
                       "cornerTpCd": CornerTpCd::of(&s["slot_type"]),
                       "bnrList": banners})
            })
            .collect();
        Ok(json!({"list": list}))
    }
}

// One row of the banner grid as the screen saves it.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BnrRow {
    /// Row status.
    pub row_stat: RowStat,
    /// Corner no.
    pub corner_no: Option<String>,
    /// Target member grade code.
    pub tgt_mbr_grd_cd: Option<String>,
    /// Start date-time.
    pub strt_dtm: Option<String>,
    /// End date-time.
    pub end_dtm: Option<String>,
    /// Banner sub title.
    pub bnr_sub_titl: Option<String>,
    /// Banner title.
    pub bnr_titl: Option<String>,
    /// Button name.
    pub btn_nm: Option<String>,
    /// Link URL.
    pub lnk_url: Option<String>,
    /// Image product no.
    pub img_prd_no: Option<String>,
    /// Image color name.
    pub img_clr_nm: Option<String>,
    /// Image no.
    pub img_no: Option<String>,
    /// Promotion no.
    pub prmt_no: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct BnrSaveIn {
    /// Grid rows.
    pub rows: Vec<BnrRow>,
}

fn banner_call(r: &BnrRow, g: &Grades) -> Result<Value, ToolError> {
    if r.row_stat != RowStat::I {
        return Err(proc_err("Row status not available on this screen."));
    }
    let mut content = Map::new();
    opt(&mut content, "eyebrow", r.bnr_sub_titl.as_ref());
    content.insert("headline".into(), json!(need(&r.bnr_titl, "bnrTitl")?));
    content.insert("cta_label".into(), json!(need(&r.btn_nm, "btnNm")?));
    content.insert("url".into(), json!(need(&r.lnk_url, "lnkUrl")?));
    match (&r.img_prd_no, &r.img_no) {
        (Some(p), _) => {
            content.insert("image_source".into(), json!("product"));
            content.insert("image_product_id".into(), json!(p));
            opt(&mut content, "image_color", r.img_clr_nm.as_ref());
        }
        (None, Some(img)) => {
            content.insert("image_source".into(), json!("image"));
            content.insert("image_id".into(), json!(img));
        }
        (None, None) => return Err(bad("`imgPrdNo` or `imgNo` is required.")),
    }
    let mut q = Map::new();
    q.insert("slot_id".into(), json!(need(&r.corner_no, "cornerNo")?));
    q.insert(
        "audience".into(),
        json!(tier_of(g, need(&r.tgt_mbr_grd_cd, "tgtMbrGrdCd")?)?),
    );
    q.insert("valid_from".into(), json!(need(&r.strt_dtm, "strtDtm")?));
    q.insert("valid_through".into(), json!(need(&r.end_dtm, "endDtm")?));
    q.insert("content".into(), Value::Object(content));
    opt(&mut q, "promotion_id", r.prmt_no.as_ref());
    Ok(Value::Object(q))
}

#[derive(Debug)]
pub struct SaveDispBnrList;

impl Screen for SaveDispBnrList {
    const META: OpMeta = screen!(
        "merchandising.disp.saveDispBnrList",
        "saveDispBnrList",
        Offer,
        Write,
        Operator,
        SCHEDULE,
        "Display banner list save."
    );
    type Input = BnrSaveIn;

    async fn call(&self, be: &Backend<'_>, i: BnrSaveIn) -> Result<Value, ToolError> {
        if i.rows.is_empty() {
            return Err(bad("No rows."));
        }
        let g = grades(be).await?;
        let mut out = Vec::new();
        for (n, r) in i.rows.iter().enumerate() {
            let done = async { be.run("schedule_banner", banner_call(r, &g)?).await }
                .await
                .map_err(|e| row_failed(n + 1, out.len(), &e))?;
            out.push(json!({"rowNo": n + 1, "rowStat": r.row_stat.as_code(), "dispNo": done.id}));
        }
        Ok(saved(out))
    }
}
