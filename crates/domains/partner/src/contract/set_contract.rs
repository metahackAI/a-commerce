//! `set_contract`: sets up or amends a seller's contract. It decides what the seller is paid,
//! so a person always approves it (W$). Earlier versions are never changed: each run adds one
//! version, under the same contract id. Rates must stay inside `commission`, and versions of one
//! contract must not overlap (`contract.no_period_overlap`). The payout account is the payout
//! provider's reference, never an account number.

use std::collections::BTreeSet;

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::{SellerStatus, SettlementCycle, SettlementMethod};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{current, rates_wire};
use crate::model::{self, num};
use crate::repo::{self, VersionRow};
use crate::rules::Rules;
use crate::seller::payout_shown;
use crate::viewer::{self, Viewer};
use crate::{
    COMMISSION_OUT_OF_BOUNDS, CONTRACT_OVERLAP, SELLER_NOT_ACTIVE, SELLER_NOT_FOUND, parse_time,
    seller_not_found, store,
};

/// One commission rate.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommissionRateInput {
    /// The category (`cat_…`) this rate is for; leave it out for the default rate, which every
    /// category without its own rate uses.
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_outerwear")]
    pub category_id: Option<String>,
    /// The store's commission as a rate from 0 to 1 (0.12 is 12%).
    #[serde(with = "rust_decimal::serde::arbitrary_precision")]
    #[schemars(with = "f64", range(min = 0, max = 1), example = 0.12)]
    pub rate: Decimal,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetContractInput {
    /// The seller, from `search_sellers`; it must be approved or active.
    #[schemars(regex(pattern = r"^slr_[a-z0-9_]+$"), example = &"slr_harbor_goods")]
    pub seller_id: String,
    /// The default rate (an entry without `category_id`) and rates for single categories. An
    /// amendment without a default keeps the default of the version in force.
    #[serde(rename = "acom:commission_rate", alias = "commission_rate")]
    #[schemars(length(min = 1, max = 30),
               example = json!([{"rate": 0.12}, {"category_id": "cat_outerwear", "rate": 0.10}]))]
    pub commission_rate: Vec<CommissionRateInput>,
    /// How often the seller is paid out: weekly, biweekly or monthly.
    #[serde(rename = "acom:settlement_cycle", alias = "settlement_cycle")]
    #[schemars(example = &"monthly")]
    pub settlement_cycle: SettlementCycle,
    /// When a sale becomes payable: on_delivery (when it arrives) or on_confirmation (when the
    /// shopper confirms the purchase).
    #[serde(rename = "acom:settlement_method", alias = "settlement_method")]
    #[schemars(example = &"on_delivery")]
    pub settlement_method: SettlementMethod,
    /// The payout provider's reference for the seller's verified bank account; never the
    /// account number itself.
    #[serde(rename = "acom:payout_account_ref", alias = "payout_account_ref")]
    #[schemars(length(min = 4, max = 80), regex(pattern = r"^[A-Za-z0-9_-]+$"),
               example = &"pa_harbor_checking_4410")]
    pub payout_account_ref: String,
    /// First moment the terms apply (store time).
    #[schemars(example = &"2027-01-01T00:00")]
    pub valid_from: String,
    /// Last moment the terms apply (store time).
    #[schemars(example = &"2027-12-31T23:59")]
    pub valid_through: String,
}

/// The version to add, as previewed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContractPlan {
    pub seller_id: String,
    pub contract_id: String,
    /// The seller has no contract yet: the run creates it.
    pub new_contract: bool,
    pub version: VersionRow,
    /// Rates outside `commission` (the check failed when not empty).
    pub out_of_bounds: Vec<String>,
    /// The version the period overlaps, when the policy forbids it.
    pub overlaps: Option<i32>,
}

pub struct SetContract;

impl Operation for SetContract {
    const META: OpMeta = OpMeta {
        address: "partner.contract.set_contract",
        tool: "set_contract",
        group: Group::Operate,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/partner/sellers/{seller_id}/contract",
        summary: "Set up or amend a seller's contract: commission rates (default and by category), settlement cycle and method, payout account reference and period. Adds a new version; earlier versions stay. A person always approves.",
        consumes: &[Slot::one("seller_id")],
        produces: &[Slot::one("contract_id")],
        checks: &["commission", "contract.no_period_overlap"],
        errors: &[
            COMMISSION_OUT_OF_BOUNDS,
            CONTRACT_OVERLAP,
            SELLER_NOT_ACTIVE,
            SELLER_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = SetContractInput;
    type Output = Value;
}

fn percent(r: Decimal) -> String {
    format!("{}%", (r * Decimal::ONE_HUNDRED).normalize())
}

/// The rates of an input: the default (when given) and the category rates, checked for form.
#[allow(clippy::type_complexity)]
fn read_rates(
    input: &[CommissionRateInput],
) -> Result<(Option<Decimal>, Vec<(String, Decimal)>), ToolError> {
    let mut default = None;
    let mut by_category = Vec::new();
    let mut seen = BTreeSet::new();
    for r in input {
        if r.rate < Decimal::ZERO || r.rate > Decimal::ONE {
            return Err(ToolError::invalid_input(format!(
                "A commission rate is a rate from 0 to 1 (0.12 is 12%), not {}.",
                r.rate
            ))
            .detail(json!({"field": "acom:commission_rate", "reason": "range"})));
        }
        match &r.category_id {
            None if default.is_some() => {
                return Err(ToolError::invalid_input(
                    "Give one default rate (one entry without category_id).",
                )
                .detail(json!({"field": "acom:commission_rate", "reason": "format"})));
            }
            None => default = Some(r.rate),
            Some(c) if !seen.insert(c.clone()) => {
                return Err(ToolError::invalid_input(format!("{c} has two rates."))
                    .detail(json!({"field": "acom:commission_rate", "reason": "format"})));
            }
            Some(c) => by_category.push((c.clone(), r.rate)),
        }
    }
    Ok((default, by_category))
}

/// `commission`: every rate inside the store's bounds.
fn commission_check(rules: &Rules, rates: &[(Option<&str>, Decimal)]) -> (Check, Vec<String>) {
    let bounds = format!(
        "{}–{}",
        percent(rules.commission_min),
        percent(rules.commission_max)
    );
    let out: Vec<String> = rates
        .iter()
        .filter(|(_, r)| *r < rules.commission_min || *r > rules.commission_max)
        .map(|(c, r)| format!("{} {}", c.unwrap_or("default"), percent(*r)))
        .collect();
    let shown: Vec<String> = rates.iter().map(|(_, r)| percent(*r)).collect();
    let mut shown_unique: Vec<String> = Vec::new();
    for s in shown {
        if !shown_unique.contains(&s) {
            shown_unique.push(s);
        }
    }
    (
        Check {
            rule: "commission".into(),
            passed: out.is_empty(),
            detail: Some(if out.is_empty() {
                format!("{} is within {bounds}", shown_unique.join(", "))
            } else {
                format!("{} is outside {bounds}", out.join(", "))
            }),
            fix_tool: (!out.is_empty()).then(|| "set_contract".into()),
        },
        out,
    )
}

/// `contract.no_period_overlap`: the new period against the existing versions.
fn overlap_check(
    rules: &Rules,
    versions: &[VersionRow],
    period: (DateTime<Utc>, DateTime<Utc>),
    cx: &Cx,
) -> (Check, Option<i32>) {
    let hit = versions
        .iter()
        .find(|v| model::overlaps((v.valid_from, v.valid_through), period));
    let (passed, detail) = match (hit, rules.no_period_overlap) {
        (None, _) => (
            true,
            match versions.iter().max_by_key(|v| v.valid_through) {
                Some(last)
                    if model::store_date(&cx.time, last.valid_through).succ_opt()
                        == Some(model::store_date(&cx.time, period.0)) =>
                {
                    format!("Version {} ends the day before", last.version_number)
                }
                Some(_) => "No other version covers this period".into(),
                None => "First version of the contract".into(),
            },
        ),
        (Some(v), false) => (
            true,
            format!(
                "Overlaps version {}; the store allows overlapping versions",
                v.version_number
            ),
        ),
        (Some(v), true) => (
            false,
            format!(
                "Overlaps version {} ({} – {})",
                v.version_number,
                cx.time.format(v.valid_from),
                cx.time.format(v.valid_through)
            ),
        ),
    };
    (
        Check {
            rule: "contract.no_period_overlap".into(),
            passed,
            detail: Some(detail),
            fix_tool: (!passed).then(|| "get_contract".into()),
        },
        hit.filter(|_| !passed).map(|v| v.version_number),
    )
}

/// Whether a stored version has the planned terms (rates compared as numbers).
fn same_terms(a: &VersionRow, b: &VersionRow) -> bool {
    a.version_number == b.version_number
        && a.default_commission_rate == b.default_commission_rate
        && super::category_rates(a) == super::category_rates(b)
        && a.settlement_cycle == b.settlement_cycle
        && a.settlement_method == b.settlement_method
        && a.payout_account_ref == b.payout_account_ref
        && a.valid_from == b.valid_from
        && a.valid_through == b.valid_through
}

fn terms_wire(v: &VersionRow, seller_id: &str, rules: &Rules, cx: &Cx) -> Value {
    json!({
        "version": v.version_number,
        "default_rate": num(v.default_commission_rate.normalize()),
        "acom:commission_rate": rates_wire(v),
        "acom:settlement_cycle": v.settlement_cycle,
        "acom:settlement_method": v.settlement_method,
        "acom:payout_account": payout_shown(seller_id, &v.payout_account_ref, &Viewer::Store, rules),
        "valid_from": cx.time.format(v.valid_from),
        "valid_through": cx.time.format(v.valid_through),
    })
}

impl WriteOp<Cx> for SetContract {
    type Plan = ContractPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SetContractInput,
    ) -> Result<Planned<ContractPlan>, ToolError> {
        viewer::staff(cx)?;
        let from = parse_time(cx, "valid_from", &input.valid_from)?;
        let through = parse_time(cx, "valid_through", &input.valid_through)?;
        if through <= from {
            return Err(
                ToolError::invalid_input("`valid_through` must come after `valid_from`.")
                    .detail(json!({"field": "valid_through", "reason": "range"})),
            );
        }
        let (default, by_category) = read_rates(&input.commission_rate)?;
        let reference = input.payout_account_ref.trim().to_owned();
        if reference.len() < 4 || reference.chars().all(|c| c.is_ascii_digit()) {
            return Err(ToolError::invalid_input(
                "Give the payout provider's reference for the bank account (for example pa_harbor_checking_4410), not the account number.",
            )
            .detail(json!({"field": "acom:payout_account_ref", "reason": "format"})));
        }
        let rules = Rules::load(&cx.policy)?;
        let now = cx.clock.now();

        let mut tx = cx.read().await.map_err(store)?;
        viewer::viewer(cx, &mut tx)
            .await?
            .store_only("set a seller's contract")?;
        let s = repo::seller(&mut tx, &input.seller_id)
            .await?
            .ok_or_else(|| seller_not_found(&input.seller_id))?;
        let contract = repo::contract_of(&mut tx, &s.id).await?;
        let versions = match &contract {
            Some((id, _)) => repo::versions(&mut tx, id).await?,
            None => Vec::new(),
        };
        let contract_id = match &contract {
            Some((id, _)) => id.clone(),
            None => {
                let base = format!("ctr_{}", s.id.trim_start_matches("slr_"));
                let mut id = base.clone();
                let mut n = 1;
                while repo::contract_exists(&mut tx, &id).await? {
                    n += 1;
                    id = format!("{base}_{n}");
                }
                id
            }
        };
        let known = acom_catalog::service::for_partner::categories(&mut tx).await?;
        tx.commit().await.map_err(store)?;

        if s.seller_status != SellerStatus::Approved.as_str()
            && s.seller_status != SellerStatus::Active.as_str()
        {
            return Err(ToolError::new(
                SELLER_NOT_ACTIVE,
                format!(
                    "{} is {}; contracts are set for approved or active sellers.",
                    s.legal_name,
                    s.seller_status.replace('_', " ")
                ),
            )
            .detail(json!({"entity": "seller", "from": s.seller_status}))
            .next("set_seller_status"));
        }
        if !known.is_empty()
            && let Some((unknown, _)) = by_category
                .iter()
                .find(|(c, _)| !known.iter().any(|k| k.id == *c))
        {
            return Err(
                ToolError::invalid_input(format!("The store has no category {unknown}."))
                    .detail(json!({"field": "acom:commission_rate", "reason": "enum"})),
            );
        }
        let default =
            match default.or_else(|| current(&versions, now).map(|v| v.default_commission_rate)) {
                Some(d) => d,
                None => {
                    return Err(ToolError::invalid_input(
                        "A first contract needs a default rate: add an entry without category_id.",
                    )
                    .detail(json!({"field": "acom:commission_rate", "reason": "missing"})));
                }
            };

        let mut rates: Vec<(Option<&str>, Decimal)> = vec![(None, default)];
        rates.extend(by_category.iter().map(|(c, r)| (Some(c.as_str()), *r)));
        let (commission, out_of_bounds) = commission_check(&rules, &rates);
        let (overlap, overlaps) = overlap_check(&rules, &versions, (from, through), cx);
        let number = versions.iter().map(|v| v.version_number).max().unwrap_or(0) + 1;
        let version = VersionRow {
            version_number: number,
            default_commission_rate: default,
            commission_rate: json!(
                by_category
                    .iter()
                    .map(|(c, r)| json!({"category_id": c, "rate": num(*r)}))
                    .collect::<Vec<_>>()
            ),
            settlement_cycle: input.settlement_cycle.as_str().to_owned(),
            settlement_method: input.settlement_method.as_str().to_owned(),
            payout_account_ref: reference,
            valid_from: from,
            valid_through: through,
        };
        let mut basis = vec![Basis {
            entity: "partner.seller".into(),
            id: s.id.clone(),
            version: i64::from(s.version),
        }];
        if let Some((id, v)) = &contract {
            basis.push(Basis {
                entity: "partner.seller_contract".into(),
                id: id.clone(),
                version: i64::from(*v),
            });
        }
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "seller_contract".into(),
                id: contract_id.clone(),
                from: match versions.last() {
                    Some(v) => json!({"version": v.version_number,
                                      "default_rate": num(v.default_commission_rate.normalize())}),
                    None => Value::Null,
                },
                to: terms_wire(&version, &s.id, &rules, cx),
                note: None,
            }],
            checks: vec![commission, overlap],
            basis,
            approval: ApprovalNeed::Needed {
                reason: "Sets commission and payout terms".into(),
            },
            plan: ContractPlan {
                seller_id: s.id,
                contract_id,
                new_contract: contract.is_none(),
                version,
                out_of_bounds,
                overlaps,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ContractPlan) -> Result<Committed, ToolError> {
        if !plan.out_of_bounds.is_empty() {
            let rules = Rules::load(&tx.policy)?;
            return Err(ToolError::new(
                COMMISSION_OUT_OF_BOUNDS,
                format!(
                    "{} is outside the store's commission range {}–{}.",
                    plan.out_of_bounds.join(", "),
                    percent(rules.commission_min),
                    percent(rules.commission_max)
                ),
            )
            .detail(json!({"rule": "commission", "items": plan.out_of_bounds}))
            .next("set_contract"));
        }
        if let Some(v) = plan.overlaps {
            return Err(ToolError::new(
                CONTRACT_OVERLAP,
                format!("The period overlaps version {v} of the contract."),
            )
            .detail(json!({"rule": "contract.no_period_overlap", "items": [v]}))
            .next("get_contract"));
        }
        if plan.new_contract {
            repo::insert_contract(tx, &plan.contract_id, &plan.seller_id).await?;
        } else {
            repo::touch_contract(tx, &plan.contract_id).await?;
        }
        repo::insert_version(tx, &plan.contract_id, &plan.version).await?;
        Ok(Committed {
            id: plan.contract_id.clone(),
            produced: Map::from_iter([
                ("contract_id".to_owned(), json!(plan.contract_id)),
                ("version".to_owned(), json!(plan.version.version_number)),
            ]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ContractPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(store)?;
        let versions = repo::versions(&mut tx, &plan.contract_id).await?;
        tx.commit().await.map_err(store)?;
        let mismatches = match versions
            .iter()
            .find(|v| v.version_number == plan.version.version_number)
        {
            None => vec![format!(
                "version {} of {} is missing",
                plan.version.version_number, plan.contract_id
            )],
            Some(v) if !same_terms(v, &plan.version) => vec![format!(
                "version {} of {} has other terms",
                v.version_number, plan.contract_id
            )],
            Some(_) => Vec::new(),
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &ContractPlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        repo::delete_version(tx, &plan.contract_id, plan.version.version_number).await?;
        if plan.new_contract {
            repo::delete_contract(tx, &plan.contract_id).await
        } else {
            repo::touch_contract(tx, &plan.contract_id).await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::tests::rules;
    use crate::tests::{input, offline};

    fn rate(c: Option<&str>, r: i64) -> CommissionRateInput {
        CommissionRateInput {
            category_id: c.map(str::to_owned),
            rate: Decimal::new(r, 2),
        }
    }

    #[test]
    fn rates_over_the_bound_fail_the_commission_check() {
        let r = rules();
        let (check, out) = commission_check(
            &r,
            &[
                (None, Decimal::new(12, 2)),
                (Some("cat_shoes"), Decimal::new(45, 2)),
            ],
        );
        assert!(!check.passed);
        assert_eq!(out, ["cat_shoes 45%"]);
        assert_eq!(check.fix_tool.as_deref(), Some("set_contract"));
        let (check, _) = commission_check(&r, &[(None, Decimal::new(12, 2))]);
        assert!(check.passed);
        assert_eq!(check.detail.as_deref(), Some("12% is within 0%–40%"));
    }

    #[test]
    fn one_default_and_one_rate_per_category() {
        assert!(read_rates(&[rate(None, 12), rate(None, 10)]).is_err());
        assert!(read_rates(&[rate(Some("cat_a"), 12), rate(Some("cat_a"), 10)]).is_err());
        let (d, c) = read_rates(&[rate(None, 12), rate(Some("cat_a"), 10)]).unwrap();
        assert_eq!(d, Some(Decimal::new(12, 2)));
        assert_eq!(c.len(), 1);
        assert!(read_rates(&[rate(None, 120)]).is_err());
    }

    #[tokio::test]
    async fn an_account_number_is_not_a_payout_reference() {
        let e = SetContract
            .preview(
                &offline("adm_sj"),
                input(json!({"seller_id": "slr_fieldstone",
                             "acom:commission_rate": [{"rate": 0.12}],
                             "acom:settlement_cycle": "monthly",
                             "acom:settlement_method": "on_delivery",
                             "acom:payout_account_ref": "000123456789",
                             "valid_from": "2027-01-01", "valid_through": "2027-12-31T23:59"})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
        assert!(e.message.contains("not the account number"));
    }
}
