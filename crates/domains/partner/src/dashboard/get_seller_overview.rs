//! `get_seller_overview`: one seller's operations in one call — sales, the settlement estimate,
//! shipping, claims, grade and open tasks — for the seller's own staff (role `seller`) and the
//! store's operators (role `operator`). Payouts are made outside A-Commerce, so the settlement
//! section is an estimate and says so.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::Money;
use acom_store::Cx;
use chrono::{Duration, NaiveDate};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::SECTIONS;
use crate::model::{self, rate_value};
use crate::rules::Rules;
use crate::{SELLER_NOT_FOUND, activity, contract, repo, seller_not_found, store, viewer};
use acom_contract::ErrorCode;

/// Which days the overview covers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OverviewPeriod {
    /// The 30 store days before today (the default).
    #[serde(rename = "last_30_days")]
    Last30Days,
    /// The 7 store days before today.
    #[serde(rename = "last_7_days")]
    Last7Days,
    /// The days from `valid_from` to `valid_through`.
    Custom,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetSellerOverviewInput {
    /// The seller, from `search_sellers`.
    #[schemars(regex(pattern = r"^slr_[a-z0-9_]+$"), example = &"slr_fieldstone")]
    pub seller_id: String,
    /// last_30_days: the 30 days before today (default); last_7_days: the 7 days before today;
    /// custom: `valid_from` to `valid_through`.
    #[schemars(example = &"last_30_days")]
    pub period: Option<OverviewPeriod>,
    /// First store day; needed with `custom`.
    #[schemars(example = &"2026-10-01")]
    pub valid_from: Option<NaiveDate>,
    /// Last store day, included; needed with `custom`.
    #[schemars(example = &"2026-10-31")]
    pub valid_through: Option<NaiveDate>,
    /// Sections to return: sales, settlement, fulfillment, claims, grade, tasks. Default: all
    /// the caller's role may see.
    #[schemars(example = ["sales", "claims"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetSellerOverview;

impl Operation for GetSellerOverview {
    const META: OpMeta = OpMeta {
        address: "partner.dashboard.get_seller_overview",
        tool: "get_seller_overview",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/partner/sellers/{seller_id}/overview",
        summary: "One seller's operations at once: orders and revenue, the payout estimate and next payout date, shipping compliance and late shipments, open claims and claim rate, grade, and open tasks, for the last 30 or 7 days or chosen days.",
        consumes: &[Slot::one("seller_id")],
        produces: &[],
        checks: &["dashboard.visible_metrics_by_role"],
        errors: &[SELLER_NOT_FOUND, ErrorCode::FORBIDDEN],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetSellerOverviewInput;
    type Output = Value;
}

/// The store days an overview covers.
pub(crate) fn days(
    input: &GetSellerOverviewInput,
    today: NaiveDate,
) -> Result<(NaiveDate, NaiveDate), ToolError> {
    let yesterday = today - Duration::days(1);
    match input.period.unwrap_or(OverviewPeriod::Last30Days) {
        OverviewPeriod::Last30Days => Ok((today - Duration::days(30), yesterday)),
        OverviewPeriod::Last7Days => Ok((today - Duration::days(7), yesterday)),
        OverviewPeriod::Custom => {
            let (Some(from), Some(through)) = (input.valid_from, input.valid_through) else {
                return Err(ToolError::invalid_input(
                    "A custom period needs both valid_from and valid_through.",
                )
                .detail(json!({"field": "valid_from", "reason": "missing"})));
            };
            if through < from {
                return Err(
                    ToolError::invalid_input("`valid_through` is before `valid_from`.")
                        .detail(json!({"field": "valid_through", "reason": "range"})),
                );
            }
            if from > today {
                return Err(ToolError::invalid_input("The period starts in the future.")
                    .detail(json!({"field": "valid_from", "reason": "future"})));
            }
            Ok((from, through))
        }
    }
}

/// The sections to build: the asked ones, each of which the role must see.
pub(crate) fn sections(
    asked: Option<&[String]>,
    role: &str,
    rules: &Rules,
) -> Result<Vec<String>, ToolError> {
    let visible = rules.visible_metrics.get(role).cloned().unwrap_or_default();
    let Some(asked) = asked.filter(|a| !a.is_empty()) else {
        return Ok(visible);
    };
    if let Some(unknown) = asked.iter().find(|f| !SECTIONS.contains(&f.as_str())) {
        return Err(ToolError::invalid_input(format!(
            "`{unknown}` is not a section; use {}.",
            SECTIONS.join(", ")
        ))
        .detail(json!({"field": "fields", "reason": "enum", "allowed": SECTIONS})));
    }
    let hidden: Vec<&String> = asked.iter().filter(|f| !visible.contains(f)).collect();
    if !hidden.is_empty() {
        return Err(ToolError::new(
            ErrorCode::FORBIDDEN,
            format!(
                "The {role} role does not see {}; it sees {}.",
                hidden
                    .iter()
                    .map(|s| s.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                visible.join(", ")
            ),
        )
        .detail(
            json!({"reason": "role", "rule": "dashboard.visible_metrics_by_role",
                       "allowed": visible}),
        )
        .next("get_seller_overview"));
    }
    Ok(asked.to_vec())
}

impl ReadOp<Cx> for GetSellerOverview {
    async fn read(&self, cx: &Cx, input: GetSellerOverviewInput) -> Result<Value, ToolError> {
        viewer::staff(cx)?;
        let now = cx.clock.now();
        let today = model::store_date(&cx.time, now);
        let (from, through) = days(&input, today)?;
        let rules = Rules::load(&cx.policy)?;
        let mut tx = cx.read().await.map_err(store)?;
        let viewer = viewer::viewer(cx, &mut tx).await?;
        viewer.may_see(&input.seller_id)?;
        let wanted = sections(input.fields.as_deref(), viewer.role(), &rules)?;
        let want = |s: &str| wanted.iter().any(|w| w == s);
        let s = repo::seller(&mut tx, &input.seller_id)
            .await?
            .ok_or_else(|| seller_not_found(&input.seller_id))?;
        let products = activity::products(&mut tx, &s.id).await?;
        let start = model::day_start(&cx.time, from);
        let until = model::day_start(&cx.time, through + Duration::days(1));
        let a = activity::gather(&mut tx, &products, start, until).await?;
        let q = activity::questions(&mut tx, &products, start, until).await?;
        let m = model::metrics(
            &a.lines,
            &a.shipments,
            &a.claims,
            (q.asked, q.answered),
            now.min(until),
            rules.ship_within_days,
            cx.currency,
        );
        let terms = if want("settlement") {
            contract::terms_now(&mut tx, &s.id, now).await?
        } else {
            None
        };
        let settlement = match &terms {
            Some(t) => Some(activity::settlement(&mut tx, &rules, &products, t).await?),
            None => None,
        };
        let grade = repo::grade_at(&mut tx, &s.id, now).await?;
        tx.commit().await.map_err(store)?;

        let mut out = json!({
            "seller_id": s.id,
            "legal_name": s.legal_name,
            "acom:seller_status": s.seller_status,
            "period": {"valid_from": from.to_string(), "valid_through": through.to_string()},
            "acom:role": viewer.role(),
        });
        if want("sales") {
            out["sales"] = json!({"order_count": m.order_count, "revenue": m.revenue});
        }
        if want("settlement") {
            out["settlement"] = match (&terms, &settlement) {
                (Some(t), Some(st)) => json!({
                    "pending_amount": st.pending,
                    "next_payout_date": st.next_payout_date.to_string(),
                    "acom:settlement_cycle": t.cycle,
                    "acom:settlement_method": t.method,
                    "note": "Estimate: payouts are made outside A-Commerce.",
                }),
                _ => json!({
                    "pending_amount": Money::zero(cx.currency),
                    "next_payout_date": null,
                    "acom:settlement_cycle": null,
                    "note": "No contract is in force, so nothing is paid out.",
                }),
            };
        }
        if want("fulfillment") {
            out["fulfillment"] = json!({
                "acom:shipping_compliance_rate": rate_value(m.shipping_compliance_rate),
                "acom:order_fulfillment_rate": rate_value(m.order_fulfillment_rate),
                "late_shipments": m.late_shipments,
            });
        }
        if want("claims") {
            out["claims"] = json!({"open_claims": m.open_claims,
                                   "acom:claim_rate": rate_value(m.claim_rate)});
        }
        if want("grade") {
            out["grade"] = json!({
                "acom:grade_level": grade.map(|g| g.grade_level),
                "acom:next_review_date":
                    model::next_review_date(today, rules.review_day_of_month).to_string(),
            });
        }
        if want("tasks") {
            out["tasks"] = json!({
                "waiting_for_approval": products
                    .iter()
                    .filter(|p| p.approval_status == "pending_review")
                    .count(),
                "unanswered_inquiries": q.waiting,
            });
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::tests::rules;
    use crate::tests::{input, offline};

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn the_last_30_days_end_yesterday() {
        let i: GetSellerOverviewInput = input(json!({"seller_id": "slr_fieldstone"}));
        assert_eq!(
            days(&i, day("2026-11-06")).unwrap(),
            (day("2026-10-07"), day("2026-11-05"))
        );
        let i: GetSellerOverviewInput =
            input(json!({"seller_id": "slr_fieldstone", "period": "last_7_days"}));
        assert_eq!(
            days(&i, day("2026-11-06")).unwrap(),
            (day("2026-10-30"), day("2026-11-05"))
        );
        let i: GetSellerOverviewInput =
            input(json!({"seller_id": "slr_fieldstone", "period": "last_30_days"}));
        assert_eq!(days(&i, day("2026-11-06")).unwrap().0, day("2026-10-07"));
        let i: GetSellerOverviewInput =
            input(json!({"seller_id": "slr_fieldstone", "period": "custom"}));
        assert!(days(&i, day("2026-11-06")).is_err());
    }

    #[test]
    fn a_role_cannot_ask_for_a_section_it_does_not_see() {
        let mut r = rules();
        r.visible_metrics
            .insert("seller".into(), vec!["sales".into(), "grade".into()]);
        let e = sections(Some(&["settlement".to_owned()]), "seller", &r).unwrap_err();
        assert_eq!(e.code, "forbidden");
        assert_eq!(e.next_tool.as_deref(), Some("get_seller_overview"));
        assert_eq!(sections(None, "seller", &r).unwrap(), ["sales", "grade"]);
        assert_eq!(
            sections(Some(&["claims".to_owned()]), "operator", &r).unwrap(),
            ["claims"]
        );
        assert_eq!(
            sections(Some(&["revenue".to_owned()]), "operator", &r)
                .unwrap_err()
                .code,
            "invalid_input"
        );
    }

    #[tokio::test]
    async fn shoppers_get_no_overview() {
        let e = GetSellerOverview
            .read(
                &offline("cus_jamie"),
                input(json!({"seller_id": "slr_fieldstone"})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
    }
}
