//! `partner.contract`: the agreement with a seller (commission, settlement cycle and method,
//! payout account, period). A contract keeps its id; each change adds a numbered version and
//! never edits an earlier one. What the contract operations share: the version in force, the
//! settlement terms the dashboard estimates with, and a version as the tools return it.

pub mod get_contract;
pub mod set_contract;

pub use get_contract::GetContract;
pub use set_contract::SetContract;

use std::collections::BTreeMap;
use std::str::FromStr;

use acom_contract::{ErrorCode, ToolError};
use acom_core::StoreTime;
use acom_core::enums::{SettlementCycle, SettlementMethod};
use acom_store::TxCx;
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};

use crate::CONTRACT_NOT_FOUND;
use crate::model::{self, SettlementTerms, num};
use crate::repo::{self, VersionRow};
use crate::rules::{self, Rules};
use crate::seller::payout_shown;
use crate::viewer::Viewer;

/// `contract_not_found`, pointing at `set_contract`.
pub(crate) fn contract_not_found(seller_id: &str, version: Option<i32>) -> ToolError {
    ToolError::new(
        CONTRACT_NOT_FOUND,
        match version {
            Some(v) => format!("The contract of {seller_id} has no version {v}."),
            None => format!("{seller_id} has no contract yet."),
        },
    )
    .detail(json!({"entity": "contract", "seller_id": seller_id, "version": version}))
    .next("set_contract")
}

/// The version in force at `now`; else the latest that has started; else the first one.
pub(crate) fn current(versions: &[VersionRow], now: DateTime<Utc>) -> Option<&VersionRow> {
    versions
        .iter()
        .rev()
        .find(|v| v.valid_from <= now && now <= v.valid_through)
        .or_else(|| versions.iter().rev().find(|v| v.valid_from <= now))
        .or_else(|| versions.first())
}

/// Category rates of a version, from its stored list (`[{category_id, rate}]`).
pub(crate) fn category_rates(v: &VersionRow) -> BTreeMap<String, Decimal> {
    v.commission_rate
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|r| {
            Some((
                r["category_id"].as_str()?.to_owned(),
                rules::decimal(&r["rate"])?,
            ))
        })
        .collect()
}

/// The settlement terms a version sets.
pub(crate) fn terms_of(v: &VersionRow, st: &StoreTime) -> Result<SettlementTerms, ToolError> {
    let broken = |what: &str| {
        ToolError::new(
            ErrorCode::INTERNAL,
            format!("contract version {} has a bad {what}", v.version_number),
        )
    };
    Ok(SettlementTerms {
        default_rate: v.default_commission_rate,
        by_category: category_rates(v),
        cycle: SettlementCycle::from_str(&v.settlement_cycle)
            .map_err(|_| broken("settlement cycle"))?,
        method: SettlementMethod::from_str(&v.settlement_method)
            .map_err(|_| broken("settlement method"))?,
        anchor: model::store_date(st, v.valid_from),
    })
}

/// The settlement terms of the seller's contract version in force now, if it has a contract.
pub(crate) async fn terms_now(
    tx: &mut TxCx,
    seller_id: &str,
    now: DateTime<Utc>,
) -> Result<Option<SettlementTerms>, ToolError> {
    let Some((contract_id, _)) = repo::contract_of(tx, seller_id).await? else {
        return Ok(None);
    };
    let versions = repo::versions(tx, &contract_id).await?;
    current(&versions, now)
        .map(|v| terms_of(v, &tx.time))
        .transpose()
}

/// The commission rates of a version as the tools show them: the default first (no
/// `category_id`), then each category's own rate.
pub(crate) fn rates_wire(v: &VersionRow) -> Value {
    let mut list =
        vec![json!({"category_id": null, "rate": num(v.default_commission_rate.normalize())})];
    for (c, r) in category_rates(v) {
        list.push(json!({"category_id": c, "rate": num(r.normalize())}));
    }
    Value::Array(list)
}

/// One version as `get_contract` returns it.
pub(crate) fn version_wire(
    contract_id: &str,
    seller_id: &str,
    v: &VersionRow,
    now: DateTime<Utc>,
    st: &StoreTime,
    viewer: &Viewer,
    rules: &Rules,
) -> Value {
    json!({
        "id": contract_id,
        "@type": "acom:SellerContract",
        "seller_id": seller_id,
        "version": v.version_number,
        "acom:commission_rate": rates_wire(v),
        "acom:default_commission_rate": num(v.default_commission_rate.normalize()),
        "acom:settlement_cycle": v.settlement_cycle,
        "acom:settlement_method": v.settlement_method,
        "acom:payout_account": payout_shown(seller_id, &v.payout_account_ref, viewer, rules),
        "valid_from": st.format(v.valid_from),
        "valid_through": st.format(v.valid_through),
        "acom:contract_status": model::contract_status(v.valid_from, v.valid_through, now),
    })
}
