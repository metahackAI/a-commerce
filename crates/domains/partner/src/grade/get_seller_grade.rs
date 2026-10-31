//! `get_seller_grade`: a seller's grade and what it rests on — shipping compliance, order
//! fulfillment, claim rate and service score over a period (last month by default), computed by
//! the server from the seller's orders, parcels, claims and product questions, by month or day.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::Cx;
use chrono::{Duration, NaiveDate};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{basis_wire, rates_wire};
use crate::model::{self, Line};
use crate::rules::Rules;
use crate::viewer;
use crate::{NO_DATA_FOR_PERIOD, SELLER_NOT_FOUND, activity, repo, seller_not_found, store};

/// How finely the breakdown splits the period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum GradeGranularity {
    /// One row per calendar month (the default).
    Month,
    /// One row per store day.
    Day,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetSellerGradeInput {
    /// The seller, from `search_sellers`.
    #[schemars(regex(pattern = r"^slr_[a-z0-9_]+$"), example = &"slr_fieldstone")]
    pub seller_id: String,
    /// First store day of the period; with `valid_through` left out, the period runs to the
    /// end of that month. Both left out: last month.
    #[schemars(example = &"2026-10-01")]
    pub valid_from: Option<NaiveDate>,
    /// Last store day of the period, included; with `valid_from` left out, the period starts
    /// on the first of that month.
    #[schemars(example = &"2026-10-31")]
    pub valid_through: Option<NaiveDate>,
    /// month: a breakdown row per calendar month (default); day: per store day.
    #[serde(rename = "acom:granularity", alias = "granularity")]
    #[schemars(example = &"month")]
    pub granularity: Option<GradeGranularity>,
}

pub struct GetSellerGrade;

impl Operation for GetSellerGrade {
    const META: OpMeta = OpMeta {
        address: "partner.grade.get_seller_grade",
        tool: "get_seller_grade",
        group: Group::Operate,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/partner/sellers/{seller_id}/grade",
        summary: "A seller's grade and the computed metrics behind it (shipping compliance, order fulfillment, claim rate, service score, rates 0-1) for a period, last month by default, by month or day, with the next review date.",
        consumes: &[Slot::one("seller_id")],
        produces: &[],
        checks: &["grade.computed_metrics"],
        errors: &[SELLER_NOT_FOUND, NO_DATA_FOR_PERIOD],
        audience: Audience::Operator,
        action_type: ActionType::ASSESS,
        person_scope: "",
    };
    type Input = GetSellerGradeInput;
    type Output = Value;
}

/// The period asked for, in store days.
pub(crate) fn period(
    from: Option<NaiveDate>,
    through: Option<NaiveDate>,
    today: NaiveDate,
) -> Result<(NaiveDate, NaiveDate), ToolError> {
    let (from, through) = match (from, through) {
        (None, None) => model::last_month(today),
        (Some(f), None) => (f, model::month_of(f).1.min(today)),
        (None, Some(t)) => (model::month_of(t).0, t),
        (Some(f), Some(t)) => (f, t),
    };
    if through < from {
        return Err(
            ToolError::invalid_input("`valid_through` is before `valid_from`.")
                .detail(json!({"field": "valid_through", "reason": "range"})),
        );
    }
    if from > today {
        return Err(ToolError::invalid_input(
            "The period starts in the future; nothing to grade yet.",
        )
        .detail(json!({"field": "valid_from", "reason": "future"})));
    }
    Ok((from, through))
}

/// The breakdown buckets of a period.
fn buckets(
    from: NaiveDate,
    through: NaiveDate,
    g: GradeGranularity,
) -> Vec<(NaiveDate, NaiveDate)> {
    let mut out = Vec::new();
    let mut d = from;
    while d <= through {
        let end = match g {
            GradeGranularity::Day => d,
            GradeGranularity::Month => model::month_of(d).1.min(through),
        };
        out.push((d, end));
        d = end + Duration::days(1);
    }
    out
}

impl ReadOp<Cx> for GetSellerGrade {
    async fn read(&self, cx: &Cx, input: GetSellerGradeInput) -> Result<Value, ToolError> {
        viewer::staff(cx)?;
        let now = cx.clock.now();
        let today = model::store_date(&cx.time, now);
        let (from, through) = period(input.valid_from, input.valid_through, today)?;
        let rules = Rules::load(&cx.policy)?;
        let mut tx = cx.read().await.map_err(store)?;
        let viewer = viewer::viewer(cx, &mut tx).await?;
        viewer.may_see(&input.seller_id)?;
        let s = repo::seller(&mut tx, &input.seller_id)
            .await?
            .ok_or_else(|| seller_not_found(&input.seller_id))?;
        let products = activity::products(&mut tx, &s.id).await?;
        let start = model::day_start(&cx.time, from);
        let until = model::day_start(&cx.time, through + Duration::days(1));
        let a = activity::gather(&mut tx, &products, start, until).await?;
        let q = activity::questions(&mut tx, &products, start, until).await?;
        let grade_now = repo::grade_at(&mut tx, &s.id, now).await?;
        tx.commit().await.map_err(store)?;

        let judged_at = now.min(until);
        let m = model::metrics(
            &a.lines,
            &a.shipments,
            &a.claims,
            (q.asked, q.answered),
            judged_at,
            rules.ship_within_days,
            cx.currency,
        );
        if m.lines == 0 {
            return Err(ToolError::new(
                NO_DATA_FOR_PERIOD,
                format!(
                    "{} has no paid orders from {from} to {through}; pick another period.",
                    s.legal_name
                ),
            )
            .detail(json!({"valid_from": from.to_string(), "valid_through": through.to_string()}))
            .next("get_seller_grade"));
        }
        let computed = model::grade_for(&m, &rules);
        let breakdown: Vec<Value> = buckets(
            from,
            through,
            input.granularity.unwrap_or(GradeGranularity::Month),
        )
        .into_iter()
        .map(|(b_from, b_through)| {
            let lines: Vec<Line> = a
                .lines
                .iter()
                .filter(|l| {
                    let d = model::store_date(&cx.time, l.order_date);
                    d >= b_from && d <= b_through
                })
                .cloned()
                .collect();
            let bm = model::metrics(
                &lines,
                &a.shipments,
                &a.claims,
                (0, 0),
                judged_at,
                rules.ship_within_days,
                cx.currency,
            );
            let mut row = json!({"valid_from": b_from.to_string(),
                                 "valid_through": b_through.to_string(),
                                 "order_count": bm.order_count,
                                 "late_shipments": bm.late_shipments});
            if let (Some(o), Value::Object(r)) = (row.as_object_mut(), rates_wire(&bm, &rules)) {
                o.extend(r);
                o.remove("acom:service_score");
            }
            row
        })
        .collect();

        let mut out = json!({
            "seller_id": s.id,
            "@type": "acom:SellerGrade",
            "acom:grade_level": grade_now.map(|g| g.grade_level),
            "acom:computed_grade_level": computed,
            "acom:graded_period": {"valid_from": from.to_string(), "valid_through": through.to_string()},
            "acom:next_review_date": model::next_review_date(today, rules.review_day_of_month).to_string(),
            "acom:basis": basis_wire(&m),
            "acom:breakdown": breakdown,
        });
        if let (Some(o), Value::Object(r)) = (out.as_object_mut(), rates_wire(&m, &rules)) {
            o.extend(r);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{input, offline};

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn the_default_period_is_last_month() {
        assert_eq!(
            period(None, None, day("2026-11-06")).unwrap(),
            (day("2026-10-01"), day("2026-10-31"))
        );
        assert_eq!(
            period(Some(day("2026-11-01")), None, day("2026-11-06")).unwrap(),
            (day("2026-11-01"), day("2026-11-06"))
        );
        assert_eq!(
            buckets(
                day("2026-09-20"),
                day("2026-10-31"),
                GradeGranularity::Month
            )
            .len(),
            2
        );
        assert_eq!(
            buckets(day("2026-10-01"), day("2026-10-07"), GradeGranularity::Day).len(),
            7
        );
    }

    #[tokio::test]
    async fn a_backwards_period_is_refused() {
        let e = GetSellerGrade
            .read(
                &offline("adm_sj"),
                input(
                    json!({"seller_id": "slr_fieldstone", "valid_from": "2026-10-31",
                             "valid_through": "2026-10-01"}),
                ),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
    }
}
