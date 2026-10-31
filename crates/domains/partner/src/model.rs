//! Pure partner logic, no I/O: masking sensitive seller data, store-day periods, the metrics a
//! seller is graded on, the grade they give, and the settlement estimate. The operations feed
//! these with rows read through other domains' services and the store's rules.

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use acom_core::enums::{ContractStatus, GradeLevel, SettlementCycle, SettlementMethod};
use acom_core::{Currency, Money, StoreTime};
use chrono::{DateTime, Datelike, Duration, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde_json::Value;

use crate::rules::Rules;

/// A decimal as an exact JSON number (`0.12`, `48200.00`).
pub fn num(d: Decimal) -> Value {
    serde_json::Number::from_str(&d.to_string())
        .map(Value::Number)
        .unwrap_or(Value::Null)
}

/// A rate as a JSON number without trailing zeros, or `null`.
pub fn rate_value(r: Option<Decimal>) -> Value {
    r.map_or(Value::Null, |d| num(d.normalize()))
}

// ── masking ──────────────────────────────────────────────────────────────────────────────

/// Hides all but the last `visible` letters and digits; separators (`-`, space, `.`, `/`) stay
/// so the shape is still recognisable: `512-73-4410` → `***-**-4410`.
pub fn mask(value: &str, visible: usize) -> String {
    let total = value.chars().filter(|c| c.is_alphanumeric()).count();
    let keep_from = total.saturating_sub(visible);
    let mut seen = 0;
    value
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                seen += 1;
                if seen > keep_from { c } else { '*' }
            } else if matches!(c, '-' | ' ' | '.' | '/') {
                c
            } else {
                '*'
            }
        })
        .collect()
}

/// A tax id reduced to its letters and digits, upper case, for comparing two spellings.
pub fn normalized_tax_id(s: &str) -> String {
    s.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_uppercase())
        .collect()
}

/// Whether a tax id has the form of a registration number: letters, digits and separators
/// only, with a digit count inside the policy's bounds.
pub fn tax_id_well_formed(s: &str, digits: (usize, usize)) -> bool {
    let ok_chars = s
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | ' ' | '.' | '/'));
    let n = s.chars().filter(char::is_ascii_digit).count();
    ok_chars && n >= digits.0 && n <= digits.1
}

/// The readable id suffix of a new seller: the legal name in lowercase words, without the legal
/// form at the end (`Harbor Goods LLC` → `harbor_goods`).
pub fn seller_slug(legal_name: &str) -> String {
    const LEGAL_FORMS: &[&str] = &[
        "llc",
        "inc",
        "co",
        "corp",
        "corporation",
        "ltd",
        "limited",
        "company",
        "plc",
        "llp",
        "lp",
        "gmbh",
    ];
    let mut words: Vec<String> = legal_name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect();
    while words.len() > 1
        && words
            .last()
            .is_some_and(|w| LEGAL_FORMS.contains(&w.as_str()))
    {
        words.pop();
    }
    if words.is_empty() {
        "seller".to_owned()
    } else {
        words.join("_")
    }
}

// ── store days ───────────────────────────────────────────────────────────────────────────

/// The store day of an instant.
pub fn store_date(st: &StoreTime, t: DateTime<Utc>) -> NaiveDate {
    t.with_timezone(&st.zone).date_naive()
}

/// The instant a store day starts.
pub fn day_start(st: &StoreTime, d: NaiveDate) -> DateTime<Utc> {
    st.parse(&d.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|_| d.and_hms_opt(0, 0, 0).unwrap_or_default().and_utc())
}

/// The first and last day of the month a day is in.
pub fn month_of(d: NaiveDate) -> (NaiveDate, NaiveDate) {
    let first = d.with_day(1).unwrap_or(d);
    let next = first
        .checked_add_months(chrono::Months::new(1))
        .unwrap_or(first);
    (first, next.pred_opt().unwrap_or(first))
}

/// The calendar month before the one `today` is in.
pub fn last_month(today: NaiveDate) -> (NaiveDate, NaiveDate) {
    let (first, _) = month_of(today);
    month_of(first.pred_opt().unwrap_or(first))
}

/// The next regular grade review: the coming `day` of a month, after today.
pub fn next_review_date(today: NaiveDate, day: u32) -> NaiveDate {
    let this = today.with_day(day).unwrap_or(today);
    if this > today {
        this
    } else {
        this.checked_add_months(chrono::Months::new(1))
            .unwrap_or(this)
    }
}

/// Where a contract version stands at `now`.
pub fn contract_status(
    from: DateTime<Utc>,
    through: DateTime<Utc>,
    now: DateTime<Utc>,
) -> ContractStatus {
    if now < from {
        ContractStatus::Scheduled
    } else if now > through {
        ContractStatus::Ended
    } else {
        ContractStatus::Active
    }
}

/// Whether two closed periods share a moment.
pub fn overlaps(a: (DateTime<Utc>, DateTime<Utc>), b: (DateTime<Utc>, DateTime<Utc>)) -> bool {
    a.0 <= b.1 && b.0 <= a.1
}

// ── seller activity and metrics ──────────────────────────────────────────────────────────

/// One order line of a seller's product.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub order_id: String,
    pub order_item_id: String,
    pub order_date: DateTime<Utc>,
    pub confirmed_at: Option<DateTime<Utc>>,
    /// `order_item_status` word.
    pub item_status: String,
    pub quantity: i32,
    /// Unit price.
    pub price: Decimal,
    /// Discount on the whole line.
    pub discount: Decimal,
    /// The product's primary category, for its commission rate.
    pub category_id: Option<String>,
}

impl Line {
    /// Paid lines count; lines still waiting for payment are not sales yet.
    fn counted(&self) -> bool {
        self.item_status != "payment_due"
    }

    fn cancelled(&self) -> bool {
        self.item_status == "cancelled"
    }

    /// Sold and kept: not cancelled and not returned.
    fn sold(&self) -> bool {
        self.counted() && !self.cancelled() && self.item_status != "returned"
    }

    /// What the line brought in: price × quantity − discount.
    pub fn amount(&self) -> Decimal {
        self.price * Decimal::from(self.quantity) - self.discount
    }
}

/// One outgoing parcel of an order.
#[derive(Debug, Clone, PartialEq)]
pub struct Shipment {
    pub order_id: String,
    /// The lines in it; `None` means the whole order.
    pub items: Option<Vec<String>>,
    pub shipped_at: Option<DateTime<Utc>>,
    pub delivered_at: Option<DateTime<Utc>>,
}

impl Shipment {
    fn carries(&self, line: &Line) -> bool {
        self.order_id == line.order_id
            && self
                .items
                .as_ref()
                .is_none_or(|i| i.iter().any(|x| *x == line.order_item_id))
    }
}

/// One order line in a claim.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimLine {
    pub claim_id: String,
    /// `cancel`, `return` or `exchange`.
    pub claim_type: String,
    pub claim_status: String,
    pub liability: String,
    pub order_item_id: String,
}

impl ClaimLine {
    fn open(&self) -> bool {
        !matches!(
            self.claim_status.as_str(),
            "completed" | "declined" | "withdrawn" | "converted"
        )
    }
}

/// What a seller did in a period: the figures behind the dashboard and the grade.
#[derive(Debug, Clone, PartialEq)]
pub struct Metrics {
    /// Orders with a line of the seller's that was sold and kept.
    pub order_count: i64,
    pub revenue: Money,
    /// Paid lines of the seller's (cancelled ones included).
    pub lines: i64,
    pub on_time: i64,
    pub late_shipments: i64,
    /// On-time share of the orders that shipped or are past their ship-by time.
    pub shipping_compliance_rate: Option<Decimal>,
    /// Lines the store or seller cancelled (the customer's own cancels are not held against
    /// the seller).
    pub unfulfilled_lines: i64,
    pub order_fulfillment_rate: Option<Decimal>,
    /// Lines that went back in a return or exchange.
    pub claimed_lines: i64,
    pub claim_rate: Option<Decimal>,
    /// Claims on these lines still being handled.
    pub open_claims: i64,
    pub questions_asked: i64,
    pub questions_answered: i64,
    /// Answered share of the product questions asked in the period.
    pub service_score: Option<Decimal>,
}

fn rate(n: i64, d: i64) -> Option<Decimal> {
    (d > 0).then(|| (Decimal::from(n) / Decimal::from(d)).round_dp(4))
}

/// Shipped share, fulfillment, claims and service over the lines of a period.
#[allow(clippy::too_many_arguments)]
pub fn metrics(
    lines: &[Line],
    shipments: &[Shipment],
    claims: &[ClaimLine],
    questions: (i64, i64),
    now: DateTime<Utc>,
    ship_within_days: i64,
    currency: Currency,
) -> Metrics {
    let counted: Vec<&Line> = lines.iter().filter(|l| l.counted()).collect();
    let sold: Vec<&Line> = counted.iter().copied().filter(|l| l.sold()).collect();
    let orders: BTreeSet<&str> = sold.iter().map(|l| l.order_id.as_str()).collect();
    let revenue = Money::new(sold.iter().map(|l| l.amount()).sum(), currency).rounded();

    // Shipping: per order, the first parcel with a line of the seller's against the ship-by
    // time. Orders not shipped and not yet due wait; shipped lines without a parcel record are
    // not measured.
    let within = Duration::days(ship_within_days);
    let mut by_order: BTreeMap<&str, Vec<&Line>> = BTreeMap::new();
    for l in counted.iter().copied().filter(|l| !l.cancelled()) {
        by_order.entry(l.order_id.as_str()).or_default().push(l);
    }
    let (mut on_time, mut late) = (0, 0);
    for ls in by_order.values() {
        let due = ls[0].order_date + within;
        let shipped = shipments
            .iter()
            .filter(|s| ls.iter().any(|l| s.carries(l)))
            .filter_map(|s| s.shipped_at)
            .min();
        match shipped {
            Some(t) if t <= due => on_time += 1,
            Some(_) => late += 1,
            None if ls
                .iter()
                .any(|l| matches!(l.item_status.as_str(), "processing" | "problem"))
                && now > due =>
            {
                late += 1
            }
            None => {}
        }
    }

    // Fulfillment: a cancelled line counts against the seller unless the customer cancelled.
    let customer_cancels: BTreeSet<&str> = claims
        .iter()
        .filter(|c| c.claim_type == "cancel" && c.liability == "customer_fault")
        .map(|c| c.order_item_id.as_str())
        .collect();
    let unfulfilled = counted
        .iter()
        .filter(|l| l.cancelled() && !customer_cancels.contains(l.order_item_id.as_str()))
        .count() as i64;
    let lines_total = counted.len() as i64;

    // Claims: returns and exchanges on lines that were not cancelled.
    let not_cancelled: BTreeSet<&str> = counted
        .iter()
        .filter(|l| !l.cancelled())
        .map(|l| l.order_item_id.as_str())
        .collect();
    let claimed: BTreeSet<&str> = claims
        .iter()
        .filter(|c| c.claim_type != "cancel" && c.claim_status != "withdrawn")
        .map(|c| c.order_item_id.as_str())
        .filter(|id| not_cancelled.contains(id))
        .collect();
    let open: BTreeSet<&str> = claims
        .iter()
        .filter(|c| c.open())
        .map(|c| c.claim_id.as_str())
        .collect();

    let (asked, answered) = questions;
    Metrics {
        order_count: orders.len() as i64,
        revenue,
        lines: lines_total,
        on_time,
        late_shipments: late,
        shipping_compliance_rate: rate(on_time, on_time + late),
        unfulfilled_lines: unfulfilled,
        order_fulfillment_rate: rate(lines_total - unfulfilled, lines_total),
        claimed_lines: claimed.len() as i64,
        claim_rate: rate(claimed.len() as i64, not_cancelled.len() as i64),
        open_claims: open.len() as i64,
        questions_asked: asked,
        questions_answered: answered,
        service_score: rate(answered, asked),
    }
}

/// The grade the metrics give: the best level of the policy whose bounds every graded metric
/// meets. A metric without data (no questions asked, nothing shipped yet) does not hold a seller
/// back; below every level is `watch`.
pub fn grade_for(m: &Metrics, rules: &Rules) -> GradeLevel {
    let at_least = |v: Option<Decimal>, min: Option<Decimal>| match (v, min) {
        (Some(v), Some(min)) => v >= min,
        _ => true,
    };
    let at_most = |v: Option<Decimal>, max: Option<Decimal>| match (v, max) {
        (Some(v), Some(max)) => v <= max,
        _ => true,
    };
    for (level, bar) in &rules.levels {
        let ok = (!rules.grades_with("shipping_compliance")
            || at_least(m.shipping_compliance_rate, bar.shipping_compliance_min))
            && (!rules.grades_with("order_fulfillment")
                || at_least(m.order_fulfillment_rate, bar.order_fulfillment_min))
            && (!rules.grades_with("claim_rate") || at_most(m.claim_rate, bar.claim_rate_max))
            && (!rules.grades_with("service_score")
                || at_least(m.service_score, bar.service_score_min));
        if ok {
            return *level;
        }
    }
    GradeLevel::Watch
}

// ── settlement (payouts happen outside A-Commerce; this is the estimate) ────────────────

/// The settlement terms of the contract version in force.
#[derive(Debug, Clone, PartialEq)]
pub struct SettlementTerms {
    pub default_rate: Decimal,
    pub by_category: BTreeMap<String, Decimal>,
    pub cycle: SettlementCycle,
    pub method: SettlementMethod,
    /// The version's first day: biweekly cycles count from it.
    pub anchor: NaiveDate,
}

impl SettlementTerms {
    /// The store's commission on a sale in a category.
    pub fn rate_for(&self, category: Option<&str>) -> Decimal {
        category
            .and_then(|c| self.by_category.get(c))
            .copied()
            .unwrap_or(self.default_rate)
    }

    /// The first and last day of the settlement cycle a day is in.
    pub fn cycle_of(&self, d: NaiveDate) -> (NaiveDate, NaiveDate) {
        match self.cycle {
            SettlementCycle::Weekly => {
                let start = d - Duration::days(i64::from(d.weekday().num_days_from_monday()));
                (start, start + Duration::days(6))
            }
            SettlementCycle::Biweekly => {
                let k = (d - self.anchor).num_days().div_euclid(14);
                let start = self.anchor + Duration::days(14 * k);
                (start, start + Duration::days(13))
            }
            SettlementCycle::Monthly => month_of(d),
        }
    }

    /// When the cycle ending on `end` is paid out.
    pub fn payout_date(&self, end: NaiveDate, delay_days: i64) -> NaiveDate {
        end + Duration::days(1 + delay_days)
    }

    /// The first order day worth reading for the estimate: one cycle before the earliest cycle
    /// that is not paid yet (a sale becomes payable some days after its order).
    pub fn window_start(&self, today: NaiveDate, delay_days: i64) -> NaiveDate {
        let (unpaid, _) = self.cycle_of(today - Duration::days(delay_days));
        self.cycle_of(unpaid.pred_opt().unwrap_or(unpaid)).0
    }
}

/// What the store owes the seller and has not paid out yet.
#[derive(Debug, Clone, PartialEq)]
pub struct Settlement {
    /// Payable sales, less the commission, whose payout date is still ahead.
    pub pending: Money,
    pub next_payout_date: NaiveDate,
    pub pending_lines: i64,
}

/// The settlement estimate at `today`: a sale becomes payable on delivery or on confirmation
/// (the contract's method) and is paid with its cycle, `delay_days` after the cycle ends.
pub fn settlement(
    lines: &[Line],
    shipments: &[Shipment],
    terms: &SettlementTerms,
    today: NaiveDate,
    delay_days: i64,
    currency: Currency,
    st: &StoreTime,
) -> Settlement {
    let mut pending = Decimal::ZERO;
    let mut count = 0;
    let mut next = terms.payout_date(terms.cycle_of(today).1, delay_days);
    for l in lines.iter().filter(|l| l.sold()) {
        let payable = match terms.method {
            SettlementMethod::OnDelivery => shipments
                .iter()
                .filter(|s| s.carries(l))
                .filter_map(|s| s.delivered_at)
                .min(),
            SettlementMethod::OnConfirmation => l.confirmed_at,
        };
        let Some(at) = payable else { continue };
        let day = store_date(st, at);
        if day > today {
            continue;
        }
        let paid_on = terms.payout_date(terms.cycle_of(day).1, delay_days);
        if paid_on <= today {
            continue;
        }
        let rate = terms.rate_for(l.category_id.as_deref());
        let net = Money::new(l.amount() * (Decimal::ONE - rate), currency).rounded();
        pending += net.value;
        count += 1;
        next = next.min(paid_on);
    }
    Settlement {
        pending: Money::new(pending, currency).rounded(),
        next_payout_date: next,
        pending_lines: count,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::tests::rules;

    fn at(s: &str) -> DateTime<Utc> {
        StoreTime::new(chrono_tz::America::New_York)
            .parse(s)
            .unwrap()
    }

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn line(order: &str, item: &str, date: &str, status: &str, price: i64) -> Line {
        Line {
            order_id: order.into(),
            order_item_id: item.into(),
            order_date: at(date),
            confirmed_at: None,
            item_status: status.into(),
            quantity: 1,
            price: Decimal::from(price),
            discount: Decimal::ZERO,
            category_id: None,
        }
    }

    fn shipped(order: &str, at_s: Option<&str>, delivered: Option<&str>) -> Shipment {
        Shipment {
            order_id: order.into(),
            items: None,
            shipped_at: at_s.map(at),
            delivered_at: delivered.map(at),
        }
    }

    #[test]
    fn masks_all_but_the_last_four_and_keeps_the_shape() {
        assert_eq!(mask("512-73-4410", 4), "***-**-4410");
        assert_eq!(mask("47-2218840", 4), "**-***8840");
        assert_eq!(mask("pa_ridge_2093", 4), "*********2093");
        assert_eq!(mask("12", 4), "12");
    }

    #[test]
    fn tax_ids_compare_by_letters_and_digits() {
        assert_eq!(normalized_tax_id("47-2218840"), "472218840");
        assert!(tax_id_well_formed("47-2218840", (9, 13)));
        assert!(!tax_id_well_formed("47-22", (9, 13)));
        assert!(!tax_id_well_formed("47_2218840;", (9, 13)));
    }

    #[test]
    fn seller_ids_drop_the_legal_form() {
        assert_eq!(seller_slug("Harbor Goods LLC"), "harbor_goods");
        assert_eq!(seller_slug("Fieldstone Boot Co."), "fieldstone_boot");
        assert_eq!(seller_slug("Inc"), "inc");
    }

    #[test]
    fn periods_follow_store_days() {
        assert_eq!(
            last_month(day("2026-11-06")),
            (day("2026-10-01"), day("2026-10-31"))
        );
        assert_eq!(next_review_date(day("2026-11-06"), 1), day("2026-12-01"));
        assert_eq!(next_review_date(day("2026-11-06"), 15), day("2026-11-15"));
        let now = at("2026-11-06T11:00");
        assert_eq!(
            contract_status(at("2026-07-01"), at("2026-12-31T23:59"), now),
            ContractStatus::Active
        );
        assert_eq!(
            contract_status(at("2027-01-01"), at("2027-12-31T23:59"), now),
            ContractStatus::Scheduled
        );
        assert!(!overlaps(
            (at("2026-07-01"), at("2026-12-31T23:59")),
            (at("2027-01-01"), at("2027-06-30T23:59"))
        ));
        assert!(overlaps(
            (at("2026-07-01"), at("2026-12-31T23:59")),
            (at("2026-12-01"), at("2027-06-30T23:59"))
        ));
    }

    #[test]
    fn metrics_hold_late_shipments_and_store_cancels_against_the_seller() {
        let lines = vec![
            line("ord_1", "oln_1", "2026-10-10T10:00", "delivered", 100),
            line("ord_2", "oln_2", "2026-10-12T10:00", "delivered", 50),
            line("ord_3", "oln_3", "2026-10-20T10:00", "cancelled", 70),
            line("ord_4", "oln_4", "2026-11-04T10:00", "processing", 30),
            line("ord_5", "oln_5", "2026-11-05T20:00", "payment_due", 30),
        ];
        let ships = vec![
            shipped("ord_1", Some("2026-10-11T12:00"), Some("2026-10-13T12:00")),
            shipped("ord_2", Some("2026-10-16T12:00"), Some("2026-10-18T12:00")),
        ];
        let claims = vec![ClaimLine {
            claim_id: "clm_2".into(),
            claim_type: "return".into(),
            claim_status: "requested".into(),
            liability: "customer_fault".into(),
            order_item_id: "oln_2".into(),
        }];
        let m = metrics(
            &lines,
            &ships,
            &claims,
            (4, 3),
            at("2026-11-06T11:00"),
            2,
            Currency::USD,
        );
        assert_eq!(m.order_count, 3);
        assert_eq!(m.revenue.value, Decimal::from(180));
        // ord_1 on time, ord_2 late, ord_4 past its ship-by time.
        assert_eq!((m.on_time, m.late_shipments), (1, 2));
        assert_eq!(m.shipping_compliance_rate, Some(Decimal::new(3333, 4)));
        // The cancel had no customer claim, so the store cancelled it.
        assert_eq!(m.unfulfilled_lines, 1);
        assert_eq!(m.order_fulfillment_rate, Some(Decimal::new(75, 2)));
        assert_eq!(m.claim_rate, Some(Decimal::new(3333, 4)));
        assert_eq!(m.open_claims, 1);
        assert_eq!(m.service_score, Some(Decimal::new(75, 2)));
        assert_eq!(grade_for(&m, &rules()), GradeLevel::Watch);
    }

    #[test]
    fn a_clean_period_grades_platinum_and_missing_data_does_not_hold_back() {
        let lines = vec![line("ord_1", "oln_1", "2026-10-10T10:00", "delivered", 100)];
        let ships = vec![shipped("ord_1", Some("2026-10-11T12:00"), None)];
        let m = metrics(
            &lines,
            &ships,
            &[],
            (0, 0),
            at("2026-11-06T11:00"),
            2,
            Currency::USD,
        );
        assert_eq!(m.service_score, None);
        assert_eq!(grade_for(&m, &rules()), GradeLevel::Platinum);
    }

    #[test]
    fn settlement_counts_delivered_sales_whose_payout_is_ahead() {
        let st = StoreTime::new(chrono_tz::America::New_York);
        let terms = SettlementTerms {
            default_rate: Decimal::new(12, 2),
            by_category: BTreeMap::from([("cat_shoes".to_owned(), Decimal::new(10, 2))]),
            cycle: SettlementCycle::Monthly,
            method: SettlementMethod::OnDelivery,
            anchor: day("2026-07-01"),
        };
        let mut shoes = line("ord_1", "oln_1", "2026-10-20T10:00", "delivered", 200);
        shoes.category_id = Some("cat_shoes".into());
        let lines = vec![
            // Delivered in September: paid on Oct 15.
            line("ord_0", "oln_0", "2026-09-25T10:00", "delivered", 100),
            // Delivered in October: paid on Nov 15.
            shoes,
            // Delivered in November: paid on Dec 15.
            line("ord_2", "oln_2", "2026-11-02T10:00", "delivered", 50),
            // Not delivered yet.
            line("ord_3", "oln_3", "2026-11-05T10:00", "processing", 80),
        ];
        let ships = vec![
            shipped("ord_0", None, Some("2026-09-28T12:00")),
            shipped("ord_1", None, Some("2026-10-23T12:00")),
            shipped("ord_2", None, Some("2026-11-04T12:00")),
        ];
        let s = settlement(
            &lines,
            &ships,
            &terms,
            day("2026-11-06"),
            14,
            Currency::USD,
            &st,
        );
        // 200 × 0.90 + 50 × 0.88
        assert_eq!(s.pending.value, Decimal::from(224));
        assert_eq!(s.pending_lines, 2);
        assert_eq!(s.next_payout_date, day("2026-11-15"));
        assert_eq!(terms.window_start(day("2026-11-06"), 14), day("2026-09-01"));
        let biweekly = SettlementTerms {
            cycle: SettlementCycle::Biweekly,
            anchor: day("2026-03-01"),
            ..terms
        };
        assert_eq!(
            biweekly.cycle_of(day("2026-11-06")),
            (day("2026-10-25"), day("2026-11-07"))
        );
    }
}
