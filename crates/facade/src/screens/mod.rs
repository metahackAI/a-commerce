//! The screens, one module per area, and what they share: paging by page number, grade codes,
//! amounts without currency, and the save result.

mod clm;
mod disp;
mod mbr;
mod ord;
mod prd;
mod prmt;
mod stat;

use acom_contract::ToolError;
use serde_json::{Map, Value, json};

use crate::codes::Grades;
use crate::{Backend, Builder, ERR_PARAM, ERR_PROC};

/// The static description of one screen: a POST at the tool endpoint, no links, no checks.
macro_rules! screen {
    ($address:literal, $tool:literal, $group:ident, $kind:ident, $audience:ident, $action:ident, $summary:literal) => {
        acom_contract::OpMeta {
            address: $address,
            tool: $tool,
            group: acom_contract::Group::$group,
            kind: acom_contract::Kind::$kind,
            method: acom_contract::Method::Post,
            path: concat!("/v1/tools/", $tool),
            summary: $summary,
            consumes: &[],
            produces: &[],
            checks: &[],
            errors: &[],
            audience: acom_contract::Audience::$audience,
            action_type: acom_contract::ActionType::$action,
            person_scope: "",
        }
    };
}
pub(crate) use screen;

pub fn register(b: &mut Builder) {
    prd::register(b);
    mbr::register(b);
    prmt::register(b);
    disp::register(b);
    clm::register(b);
    ord::register(b);
    stat::register(b);
}

/// Rows per page when the screen sends none, and at most.
const PAGE_SIZE: u32 = 20;
const MAX_PAGE_SIZE: u32 = 100;

/// `pageNo`/`pageSize` as the back end's `limit` and `cursor`.
pub(crate) struct Paging {
    pub no: u32,
    pub size: u32,
}

impl Paging {
    pub fn new(no: Option<u32>, size: Option<u32>) -> Result<Self, ToolError> {
        let no = no.unwrap_or(1);
        let size = size.unwrap_or(PAGE_SIZE);
        if no == 0 || size == 0 || size > MAX_PAGE_SIZE {
            return Err(bad("Invalid paging."));
        }
        Ok(Self { no, size })
    }

    /// Adds `limit` and `cursor` to a back-end input.
    pub fn apply(&self, q: &mut Map<String, Value>) {
        q.insert("limit".into(), json!(self.size));
        let offset = u64::from(self.no - 1) * u64::from(self.size);
        if offset > 0 {
            q.insert("cursor".into(), json!(format!("o{offset}")));
        }
    }

    /// A grid answer: the rows of this page and nothing about the total.
    pub fn grid(&self, list: Vec<Value>) -> Value {
        json!({"list": list, "pageNo": self.no, "pageSize": self.size})
    }
}

/// For a `…Cnt` screen: one row is enough to learn the total.
pub(crate) fn count_only(q: &mut Map<String, Value>) {
    q.insert("limit".into(), json!(1));
}

pub(crate) fn count(out: &Value) -> Value {
    json!({"totCnt": out["total"].as_u64().unwrap_or(0)})
}

pub(crate) fn bad(message: &str) -> ToolError {
    ToolError::new(ERR_PARAM, message)
}

/// A save row that failed, with how many rows before it were saved.
pub(crate) fn row_failed(row: usize, saved: usize, e: &ToolError) -> ToolError {
    let mut message = format!("Row {row}: {}", e.message);
    if saved > 0 {
        message.push_str(&format!(" ({saved} row(s) before it saved)"));
    }
    ToolError {
        message,
        ..e.clone()
    }
}

pub(crate) fn proc_err(message: &str) -> ToolError {
    ToolError::new(ERR_PROC, message)
}

/// A field a row needs, or a parameter error naming it.
pub(crate) fn need<'v, T>(v: &'v Option<T>, field: &str) -> Result<&'v T, ToolError> {
    v.as_ref()
        .ok_or_else(|| bad(&format!("`{field}` is required.")))
}

/// The back end's list rows.
pub(crate) fn items(out: &Value) -> Vec<Value> {
    out["items"].as_array().cloned().unwrap_or_default()
}

/// An amount as the screens show it: the number, without its currency.
pub(crate) fn amt(v: &Value) -> Value {
    match v {
        Value::Object(_) => v["value"].clone(),
        other => other.clone(),
    }
}

/// The store's tiers as grade codes.
pub(crate) async fn grades(be: &Backend<'_>) -> Result<Grades, ToolError> {
    let out = be.read("list_tiers", json!({"limit": 100})).await?;
    Ok(Grades::from_tiers(&out))
}

/// A grade code as a tier id, or a parameter error.
pub(crate) fn tier_of(grades: &Grades, code: &str) -> Result<String, ToolError> {
    grades
        .tier(code)
        .ok_or_else(|| bad("Invalid member grade code."))
}

/// What a save answers.
pub(crate) fn saved(rows: Vec<Value>) -> Value {
    json!({"resultCd": "0000", "resultMsg": "Saved.", "procCnt": rows.len(), "list": rows})
}

/// Sets `key` only when `v` is present.
pub(crate) fn opt<T: serde::Serialize>(q: &mut Map<String, Value>, key: &str, v: Option<T>) {
    if let Some(v) = v {
        q.insert(key.to_owned(), json!(v));
    }
}
