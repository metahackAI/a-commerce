//! Returns: what a returned line refunds, the suggestion `evaluate_return_request` makes
//! (`return_suggestions`), and whether and how a shopper can return a line
//! (`get_return_options`, `POST /v1/fulfillment/returns`).

use acom_core::Money;
use acom_policy::PolicySet;
use rust_decimal::Decimal;
use serde::Serialize;
use serde_json::{Value, json};

use super::{Rules, holds, money_of, money_short, money_text};

/// One order line in a claim, with what was paid for it.
#[derive(Debug, Clone, PartialEq)]
pub struct ClaimLine {
    pub order_item_id: String,
    pub name: String,
    /// List price per unit when ordered.
    pub unit_price: Money,
    /// Units on the order line.
    pub ordered: i32,
    /// Discount on the whole order line, including its share of order coupons.
    pub line_discount: Money,
    /// Units in this claim.
    pub quantity: i32,
}

impl ClaimLine {
    /// What the claimed units cost the shopper: list price × units, less their rounded share
    /// of the line's discount when `refund.coupon_deducted`, so the share and the refund always
    /// add up to the list price (conventions §3: round per line).
    pub fn paid(&self, coupon_deducted: bool) -> Money {
        let gross = self
            .unit_price
            .times(Decimal::from(self.quantity))
            .rounded();
        let value = if coupon_deducted {
            gross.value - self.discount_share().value
        } else {
            gross.value
        };
        Money::new(value.max(Decimal::ZERO), self.unit_price.currency)
    }

    /// The part of the line's discount the claimed units carry.
    pub fn discount_share(&self) -> Money {
        if self.ordered <= 0 {
            return Money::zero(self.line_discount.currency);
        }
        self.line_discount
            .times(Decimal::from(self.quantity) / Decimal::from(self.ordered))
            .rounded()
    }
}

/// The refund for a return claim: the sum of the rounded line amounts.
pub fn refund_amount(lines: &[ClaimLine], coupon_deducted: bool) -> Option<Money> {
    let mut total: Option<Money> = None;
    for l in lines {
        let paid = l.paid(coupon_deducted);
        total = Some(match total {
            None => paid,
            Some(t) => t.checked_add(paid).ok()?,
        });
    }
    total
}

/// Evidence entries that are photos (everything but receipts).
pub fn photo_count(evidence: &Value) -> i64 {
    evidence
        .as_array()
        .map(|l| {
            l.iter()
                .filter(|e| e.get("evidence_type").and_then(Value::as_str) != Some("receipt"))
                .count() as i64
        })
        .unwrap_or(0)
}

/// What `evaluate_return_request` knows about one claim.
#[derive(Debug, Clone, PartialEq)]
pub struct EvalFacts {
    pub reason: String,
    pub liability: String,
    pub photos: i64,
    /// `None` while nothing has been delivered.
    pub days_since_delivery: Option<i64>,
    pub refund_amount: Money,
    /// The customer's return requests in the last `flag_return_history.days` days, this one
    /// included.
    pub recent_returns: i64,
}

/// A flag a person should look at; it does not change the suggestion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Flag {
    pub rule: String,
    pub detail: String,
}

/// What the policy suggests for one return request.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    /// `refund`, `ask_for_photos` or `decline`.
    pub suggested: String,
    /// The policy key the suggestion rests on.
    pub rule: String,
    pub rule_text: String,
    pub liability: String,
    pub refund_amount: Option<Money>,
    pub return_fees: String,
    pub needs_person: bool,
    pub flags: Vec<Flag>,
}

/// The sentence for a rule key, written from the policy values.
pub fn rule_text(rules: &Rules, rule: &str) -> String {
    let fees = |f: &str| match f {
        "free_return" => "return shipping free",
        "return_fees_customer_responsibility" => "the customer pays return shipping",
        _ => "return shipping charged",
    };
    match rule {
        "defect_refund" => format!(
            "Defects are refunded {}, {}",
            if rules.defect_refund_type == "full_refund" {
                "in full"
            } else {
                "as store credit"
            },
            fees(&rules.defect_return_fees)
        ),
        "changed_mind_return_shipping" => format!(
            "Change-of-mind returns within {} days are refunded; {}",
            rules.return_window_days,
            fees(&rules.changed_mind_return_fees)
        ),
        "damage_claim_needs_photo" => "Damage in transit needs a photo before a refund".to_owned(),
        "return_window_days" => format!(
            "Returns are accepted within {} days of delivery",
            rules.return_window_days
        ),
        other => other.replace('_', " "),
    }
}

/// Whether a refund of `amount` needs a person (`approval.yml thresholds.refund`).
pub fn refund_needs_person(policy: &PolicySet, amount: Money) -> bool {
    policy
        .approval_for("refund_return", &json!({"amount": amount}))
        .needed
}

/// `$200`: the refund amount above which a person confirms, when the threshold is an amount.
pub fn refund_limit(policy: &PolicySet) -> Option<Money> {
    policy
        .lookup("approval.thresholds.refund.when.value")
        .and_then(money_of)
}

/// The first `return_suggestions` entry, in policy order, whose condition holds. A suggestion
/// whose rule is switched off in the policy (`damage_claim_needs_photo: false`) is skipped.
pub fn evaluate(rules: &Rules, policy: &PolicySet, f: &EvalFacts) -> Option<Suggestion> {
    let facts = json!({
        "days_since_delivery": f.days_since_delivery,
        "reason": f.reason,
        "photos": f.photos,
        "liability": f.liability,
    });
    let chosen = rules.suggestion_order.iter().find_map(|name| {
        let s = rules.suggestion(name)?;
        let rule = match s.get("rule").and_then(Value::as_str) {
            Some(r) => r.to_owned(),
            None => s
                .get("rule_by_liability")?
                .get(&f.liability)?
                .as_str()?
                .to_owned(),
        };
        if rules.lookup(&rule) == Some(&Value::Bool(false)) {
            return None;
        }
        holds(s.get("when")?, &facts, rules).then(|| (name.clone(), rule))
    })?;
    let (suggested, rule) = chosen;
    let refund = suggested == "refund";
    let needs_person = if refund {
        refund_needs_person(policy, f.refund_amount)
    } else {
        // Asking for photos or declining is a customer message.
        policy
            .lookup("approval.customer_messages")
            .and_then(Value::as_str)
            == Some("always_ask")
    };
    let mut flags = Vec::new();
    if f.recent_returns >= rules.flag_returns {
        flags.push(Flag {
            rule: "return_suggestions.flag_return_history".into(),
            detail: format!(
                "{} returns in the last {} days",
                f.recent_returns, rules.flag_days
            ),
        });
    }
    Some(Suggestion {
        rule_text: rule_text(rules, &rule),
        liability: f.liability.clone(),
        refund_amount: refund.then_some(f.refund_amount),
        return_fees: rules.return_fees_for(&f.liability),
        needs_person,
        flags,
        suggested,
        rule,
    })
}

/// Why a line cannot be returned now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReturnBlock {
    /// Not delivered yet: cancel instead.
    NotShippedYet,
    OutsideWindow {
        days: i64,
        limit: i64,
    },
    /// `return.non_returnable_when` matched (`final_sale`, `gift_card`) or the line is gone.
    NotReturnable {
        why: String,
    },
    ClaimInProgress {
        claim_id: String,
    },
    TooMany {
        returnable: i32,
    },
}

/// One order line as `get_return_options` sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct ReturnableLine {
    pub line: ClaimLine,
    pub order_item_status: String,
    pub fulfillment_stage: Option<String>,
    /// Days since the order was delivered; `None` while not delivered.
    pub days_since_delivery: Option<i64>,
    /// Units already in other return or exchange claims that are still open or completed.
    pub already_claimed: i32,
    /// An open claim on the line, if any.
    pub open_claim: Option<String>,
    /// `gift_card` sale type or a `final_sale` tag.
    pub not_returnable_because: Option<String>,
}

/// The quantity a shopper can return now, or why not. `asked` defaults to every unit left.
pub fn returnable(
    rules: &Rules,
    l: &ReturnableLine,
    asked: Option<i32>,
) -> Result<i32, ReturnBlock> {
    if let Some(claim_id) = &l.open_claim {
        return Err(ReturnBlock::ClaimInProgress {
            claim_id: claim_id.clone(),
        });
    }
    match l.order_item_status.as_str() {
        "delivered" | "in_transit" => {}
        "cancelled" | "returned" => {
            return Err(ReturnBlock::NotReturnable {
                why: l.order_item_status.clone(),
            });
        }
        _ => return Err(ReturnBlock::NotShippedYet),
    }
    if let Some(why) = &l.not_returnable_because
        && rules.non_returnable_when.contains(why)
    {
        return Err(ReturnBlock::NotReturnable { why: why.clone() });
    }
    if let Some(days) = l.days_since_delivery
        && days > rules.return_window_days
    {
        return Err(ReturnBlock::OutsideWindow {
            days,
            limit: rules.return_window_days,
        });
    }
    let left = l.line.ordered - l.already_claimed;
    if left <= 0 {
        return Err(ReturnBlock::NotReturnable {
            why: "returned".into(),
        });
    }
    let q = asked.unwrap_or(left);
    if q < 1 || q > left {
        return Err(ReturnBlock::TooMany { returnable: left });
    }
    Ok(q)
}

/// A time window as the wire writes it (RFC 3339 with the store offset).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Window {
    pub start: String,
    pub end: String,
}

/// How the item goes back to the store.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SendBack {
    pub methods: Vec<String>,
    pub return_fees: String,
    pub cost: Money,
    pub pickup_window: Option<Window>,
}

/// One choice in `get_return_options`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ReturnOption {
    /// `replacement` or `refund`.
    pub kind: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub variant_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_quantity: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ships_on: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arrives_by: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refund_amount: Option<Money>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refund_to: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub money_back_business_days: Option<Value>,
    pub you_pay: Money,
    /// `store_automatic` or `store_person`.
    pub confirmed_by: String,
    pub send_back: SendBack,
    pub facts: Vec<String>,
}

/// What `get_return_options` needs besides the line.
#[derive(Debug, Clone, PartialEq)]
pub struct OptionInputs {
    pub reason: String,
    pub quantity: i32,
    pub variant_id: String,
    pub has_size: bool,
    /// Sellable units of the same variant; `None` when stock is not tracked.
    pub in_stock: Option<i32>,
    /// `card ending 0937`.
    pub refund_to: String,
    /// Store-local days, already written for people and for the wire.
    pub ships_on: chrono::NaiveDate,
    pub arrives_by: chrono::NaiveDate,
    pub today: chrono::NaiveDate,
    pub pickup_window: Option<Window>,
}

/// The replacement and refund choices for a returnable line, in that order.
pub fn return_options(
    rules: &Rules,
    policy: &PolicySet,
    line: &ClaimLine,
    o: &OptionInputs,
) -> Vec<ReturnOption> {
    let liability = rules.liability_for(&o.reason);
    let store_pays = rules.store_pays_return(&liability);
    let currency = line.unit_price.currency;
    let return_cost = if store_pays {
        Money::zero(currency)
    } else {
        rules.shipping_rate
    };
    let send_back = SendBack {
        methods: rules.send_back_methods.clone(),
        return_fees: rules.return_fees_for(&liability),
        cost: return_cost,
        pickup_window: o.pickup_window.clone(),
    };
    let cost_fact = |you_pay: Money| {
        if you_pay.value.is_zero() {
            "No cost to you".to_owned()
        } else {
            format!("You pay {} for return shipping", money_text(you_pay))
        }
    };
    let mut out = Vec::new();

    // The same variant again, new: only when the store is at fault (`exchange.replacement`).
    let replacement_ok = rules.replacement_option == "creates_exchange"
        && liability != "customer_fault"
        && o.in_stock.is_none_or(|n| n >= o.quantity);
    if replacement_ok {
        let photo_check = o.reason == "damaged_in_transit" && rules.damage_claim_needs_photo;
        let ships = if o.ships_on == o.today {
            "Ships today".to_owned()
        } else {
            format!("Ships {}", super::day_text(o.ships_on))
        };
        out.push(ReturnOption {
            kind: "replacement".into(),
            title: if o.has_size {
                "New one, same size".into()
            } else {
                "New one, same item".into()
            },
            variant_id: Some(o.variant_id.clone()),
            available_quantity: o.in_stock,
            ships_on: Some(o.ships_on.to_string()),
            arrives_by: Some(o.arrives_by.to_string()),
            refund_amount: None,
            refund_to: None,
            money_back_business_days: None,
            you_pay: return_cost,
            confirmed_by: if photo_check {
                "store_person".into()
            } else {
                "store_automatic".into()
            },
            send_back: send_back.clone(),
            facts: vec![
                ships,
                format!("Arrives {}", super::day_text(o.arrives_by)),
                cost_fact(return_cost),
            ],
        });
    }

    let amount = line.paid(rules.coupon_deducted);
    let person = refund_needs_person(policy, amount);
    let full = amount
        == line
            .unit_price
            .times(Decimal::from(line.quantity))
            .rounded();
    let (min, max) = rules.money_back_days;
    let mut facts = vec![format!("{} back to {}", money_text(amount), o.refund_to)];
    if person {
        facts.push(match refund_limit(policy) {
            Some(limit) => format!(
                "A person at the store confirms refunds over {}",
                money_short(limit)
            ),
            None => "A person at the store confirms this refund".to_owned(),
        });
    }
    if !return_cost.value.is_zero() {
        facts.push(cost_fact(return_cost));
    }
    facts.push(format!("Money back {min}–{max} days after it is confirmed"));
    out.push(ReturnOption {
        kind: "refund".into(),
        title: if full { "Full refund" } else { "Refund" }.into(),
        variant_id: None,
        available_quantity: None,
        ships_on: None,
        arrives_by: None,
        refund_amount: Some(amount),
        refund_to: Some(o.refund_to.clone()),
        money_back_business_days: Some(json!({"min": min, "max": max})),
        you_pay: return_cost,
        confirmed_by: if person {
            "store_person".into()
        } else {
            "store_automatic".into()
        },
        send_back,
        facts,
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::delivery::days_since;
    use crate::model::tests::{policy, rules, usd};
    use acom_core::StoreTime;
    use chrono::NaiveDate;
    use chrono_tz::Tz;

    const NY: Tz = chrono_tz::America::New_York;

    fn line(id: &str, price: &str, ordered: i32, discount: &str, quantity: i32) -> ClaimLine {
        ClaimLine {
            order_item_id: id.into(),
            name: id.into(),
            unit_price: usd(price),
            ordered,
            line_discount: usd(discount),
            quantity,
        }
    }

    /// One B2 return request as the seed has it (seed/demo/06_sales.yml, 07_fulfillment.yml).
    struct Request {
        order: &'static str,
        reason: &'static str,
        photos: i64,
        delivered_at: &'static str,
        lines: Vec<ClaimLine>,
        recent_returns: i64,
    }

    fn b2() -> Vec<Request> {
        let r = |order, reason, photos, delivered_at, lines, recent_returns| Request {
            order,
            reason,
            photos,
            delivered_at,
            lines,
            recent_returns,
        };
        vec![
            r(
                "20402",
                "defect",
                2,
                "2026-10-31T14:02:00-04:00",
                vec![line("oln_20402_1", "312.00", 1, "0.00", 1)],
                1,
            ),
            r(
                "20431",
                "not_as_described",
                1,
                "2026-11-02T15:20:00-05:00",
                vec![line("oln_20431_1", "289.00", 1, "0.00", 1)],
                4,
            ),
            r(
                "20418",
                "wrong_size",
                0,
                "2026-10-30T12:00:00-04:00",
                vec![line("oln_20418_1", "128.00", 2, "0.00", 2)],
                1,
            ),
            r(
                "20377",
                "changed_mind",
                0,
                "2026-10-28T13:30:00-04:00",
                vec![
                    line("oln_20377_1", "128.00", 1, "0.00", 1),
                    line("oln_20377_2", "89.00", 1, "0.00", 1),
                ],
                1,
            ),
            r(
                "20366",
                "damaged_in_transit",
                0,
                "2026-11-04T14:30:00-05:00",
                vec![line("oln_20366_1", "118.00", 2, "0.00", 2)],
                1,
            ),
            r(
                "20290",
                "changed_mind",
                0,
                "2026-09-28T16:00:00-04:00",
                vec![line("oln_20290_1", "248.00", 1, "0.00", 1)],
                1,
            ),
            r(
                "20333",
                "wrong_size",
                0,
                "2026-10-12T11:00:00-04:00",
                vec![line("oln_20333_1", "289.00", 1, "0.00", 1)],
                1,
            ),
            r(
                "20341",
                "defect",
                1,
                "2026-10-14T16:15:00-04:00",
                vec![line("oln_20341_1", "312.00", 1, "0.00", 1)],
                1,
            ),
            // Coupon orders: the $10.00 birthday coupon sits on the tee that stays.
            r(
                "20352",
                "wrong_size",
                0,
                "2026-10-16T13:40:00-04:00",
                vec![line("oln_20352_1", "128.00", 2, "0.00", 2)],
                1,
            ),
            r(
                "20388",
                "changed_mind",
                0,
                "2026-10-29T11:50:00-04:00",
                vec![line("oln_20388_1", "248.00", 1, "0.00", 1)],
                1,
            ),
            r(
                "20395",
                "changed_mind",
                0,
                "2026-10-30T10:25:00-04:00",
                vec![line("oln_20395_1", "142.00", 2, "0.00", 2)],
                1,
            ),
            r(
                "20409",
                "damaged_in_transit",
                0,
                "2026-10-31T15:30:00-04:00",
                vec![line("oln_20409_1", "289.00", 1, "0.00", 1)],
                1,
            ),
        ]
    }

    fn suggest(req: &Request) -> Suggestion {
        let r = rules();
        let st = StoreTime::new(NY);
        let now = st.parse("2026-11-06T10:40").unwrap();
        let facts = EvalFacts {
            reason: req.reason.into(),
            liability: r.liability_for(req.reason),
            photos: req.photos,
            days_since_delivery: Some(days_since(st.parse(req.delivered_at).unwrap(), now, NY)),
            refund_amount: refund_amount(&req.lines, r.coupon_deducted).unwrap(),
            recent_returns: req.recent_returns,
        };
        evaluate(&r, &policy(), &facts).unwrap()
    }

    #[test]
    fn scene_b2_sorts_twelve_requests_into_nine_refunds_two_photo_requests_one_decline() {
        let mut refunds = Vec::new();
        let mut photos = Vec::new();
        let mut declines = Vec::new();
        let mut total = usd("0");
        for req in b2() {
            let s = suggest(&req);
            match s.suggested.as_str() {
                "refund" => {
                    total = total.checked_add(s.refund_amount.unwrap()).unwrap();
                    refunds.push(req.order);
                }
                "ask_for_photos" => {
                    assert_eq!(s.rule, "damage_claim_needs_photo");
                    assert_eq!(s.refund_amount, None);
                    photos.push(req.order);
                }
                "decline" => {
                    assert_eq!(s.rule, "return_window_days");
                    declines.push(req.order);
                }
                other => panic!("unexpected suggestion {other}"),
            }
        }
        assert_eq!(refunds.len(), 9, "{refunds:?}");
        assert_eq!(total, usd("2463.00"));
        assert_eq!(photos, ["20366", "20409"]);
        assert_eq!(declines, ["20290"]);
    }

    #[test]
    fn scene_b2_rows_match_the_spec_table() {
        let all = b2();
        let by = |o: &str| suggest(all.iter().find(|r| r.order == o).unwrap());

        let boots = by("20402");
        assert_eq!(boots.suggested, "refund");
        assert_eq!(boots.rule, "defect_refund");
        assert_eq!(
            boots.rule_text,
            "Defects are refunded in full, return shipping free"
        );
        assert_eq!(boots.liability, "store_fault");
        assert_eq!(boots.refund_amount, Some(usd("312.00")));
        assert_eq!(boots.return_fees, "free_return");
        assert!(boots.needs_person);
        assert!(boots.flags.is_empty());

        let coat = by("20431");
        assert_eq!(coat.rule, "defect_refund");
        assert_eq!(coat.refund_amount, Some(usd("289.00")));
        assert_eq!(
            coat.flags,
            [Flag {
                rule: "return_suggestions.flag_return_history".into(),
                detail: "4 returns in the last 90 days".into(),
            }]
        );

        let mind = by("20377");
        assert_eq!(mind.rule, "changed_mind_return_shipping");
        assert_eq!(mind.refund_amount, Some(usd("217.00")));
        assert_eq!(mind.return_fees, "return_fees_customer_responsibility");

        let hidden: Vec<Money> = ["20333", "20341", "20352", "20388", "20395"]
            .iter()
            .map(|o| by(o).refund_amount.unwrap())
            .collect();
        let sum = hidden
            .iter()
            .fold(usd("0"), |a, m| a.checked_add(*m).unwrap());
        assert_eq!(sum, usd("1389.00"));
    }

    #[test]
    fn coupon_share_is_taken_off_only_the_lines_that_carry_it() {
        // #20352: the coupon sits on the tee, so the two jackets refund in full.
        let jackets = line("oln_20352_1", "128.00", 2, "0.00", 2);
        assert_eq!(jackets.paid(true), usd("256.00"));
        // Returning the tee itself refunds what was paid for it.
        let tee = line("oln_20352_2", "58.00", 1, "10.00", 1);
        assert_eq!(tee.paid(true), usd("48.00"));
        assert_eq!(tee.paid(false), usd("58.00"));
        // One of two discounted units carries half the discount, rounded per line.
        let half = line("oln_x", "71.20", 2, "0.05", 1);
        assert_eq!(half.paid(true), usd("71.17"));
        assert_eq!(half.discount_share(), usd("0.03"));
    }

    #[test]
    fn under_the_limit_a_refund_needs_no_person() {
        let r = rules();
        let p = policy();
        assert!(!refund_needs_person(&p, usd("189.00")));
        assert!(!refund_needs_person(&p, usd("200.00")));
        assert!(refund_needs_person(&p, usd("200.01")));
        assert_eq!(refund_limit(&p), Some(usd("200.00")));
        let small = EvalFacts {
            reason: "defect".into(),
            liability: "store_fault".into(),
            photos: 1,
            days_since_delivery: Some(3),
            refund_amount: usd("189.00"),
            recent_returns: 1,
        };
        assert!(!evaluate(&r, &p, &small).unwrap().needs_person);
        let undelivered = EvalFacts {
            days_since_delivery: None,
            ..small
        };
        assert!(evaluate(&r, &p, &undelivered).is_none());
    }

    fn alex_line() -> ReturnableLine {
        ReturnableLine {
            line: line("oln_20402_1", "312.00", 1, "0.00", 1),
            order_item_status: "delivered".into(),
            fulfillment_stage: None,
            days_since_delivery: Some(6),
            already_claimed: 0,
            open_claim: None,
            not_returnable_because: None,
        }
    }

    #[test]
    fn scene_f5_offers_a_replacement_and_a_full_refund() {
        let r = rules();
        let l = alex_line();
        assert_eq!(returnable(&r, &l, None), Ok(1));
        let date = |s| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let o = OptionInputs {
            reason: "defect".into(),
            quantity: 1,
            variant_id: "var_chelsea_boots_9_tan".into(),
            has_size: true,
            in_stock: Some(6),
            refund_to: "card ending 0937".into(),
            ships_on: date("2026-11-06"),
            arrives_by: date("2026-11-10"),
            today: date("2026-11-06"),
            pickup_window: Some(Window {
                start: "2026-11-09T09:00:00-05:00".into(),
                end: "2026-11-09T12:00:00-05:00".into(),
            }),
        };
        let options = return_options(&r, &policy(), &l.line, &o);
        assert_eq!(options.len(), 2);
        let rep = &options[0];
        assert_eq!(rep.kind, "replacement");
        assert_eq!(rep.available_quantity, Some(6));
        assert_eq!(rep.ships_on.as_deref(), Some("2026-11-06"));
        assert_eq!(rep.arrives_by.as_deref(), Some("2026-11-10"));
        assert_eq!(rep.you_pay, usd("0"));
        assert_eq!(rep.confirmed_by, "store_automatic");
        assert_eq!(
            rep.facts,
            ["Ships today", "Arrives Tue Nov 10", "No cost to you"]
        );
        assert_eq!(rep.send_back.methods, ["pickup", "drop_off"]);
        assert_eq!(rep.send_back.return_fees, "free_return");

        let refund = &options[1];
        assert_eq!(refund.title, "Full refund");
        assert_eq!(refund.refund_amount, Some(usd("312.00")));
        assert_eq!(refund.confirmed_by, "store_person");
        assert_eq!(
            refund.facts,
            [
                "$312.00 back to card ending 0937",
                "A person at the store confirms refunds over $200",
                "Money back 3–5 days after it is confirmed",
            ]
        );
        let wire = serde_json::to_value(refund).unwrap();
        assert_eq!(
            wire["money_back_business_days"],
            json!({"min": 3, "max": 5})
        );
        assert_eq!(
            wire["send_back"]["pickup_window"]["start"],
            "2026-11-09T09:00:00-05:00"
        );
        assert!(wire.get("variant_id").is_none());
    }

    #[test]
    fn a_change_of_mind_return_offers_no_replacement_and_shows_return_shipping() {
        let r = rules();
        let l = line("oln_20388_1", "248.00", 1, "0.00", 1);
        let date = |s| NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap();
        let o = OptionInputs {
            reason: "changed_mind".into(),
            quantity: 1,
            variant_id: "var_cashmere_sweater_s_navy".into(),
            has_size: true,
            in_stock: Some(3),
            refund_to: "card ending 4408".into(),
            ships_on: date("2026-11-06"),
            arrives_by: date("2026-11-10"),
            today: date("2026-11-06"),
            pickup_window: None,
        };
        let options = return_options(&r, &policy(), &l, &o);
        assert_eq!(options.len(), 1);
        assert_eq!(options[0].refund_amount, Some(usd("248.00")));
        assert_eq!(options[0].you_pay, usd("6.00"));
        assert_eq!(
            options[0].send_back.return_fees,
            "return_fees_customer_responsibility"
        );
    }

    #[test]
    fn blocks_returns_the_policy_does_not_allow() {
        let r = rules();
        let late = ReturnableLine {
            days_since_delivery: Some(39),
            ..alex_line()
        };
        assert_eq!(
            returnable(&r, &late, None),
            Err(ReturnBlock::OutsideWindow {
                days: 39,
                limit: 30
            })
        );
        let open = ReturnableLine {
            open_claim: Some("clm_20402".into()),
            ..alex_line()
        };
        assert!(matches!(
            returnable(&r, &open, None),
            Err(ReturnBlock::ClaimInProgress { .. })
        ));
        let processing = ReturnableLine {
            order_item_status: "processing".into(),
            days_since_delivery: None,
            ..alex_line()
        };
        assert_eq!(
            returnable(&r, &processing, None),
            Err(ReturnBlock::NotShippedYet)
        );
        let final_sale = ReturnableLine {
            not_returnable_because: Some("final_sale".into()),
            ..alex_line()
        };
        assert!(matches!(
            returnable(&r, &final_sale, None),
            Err(ReturnBlock::NotReturnable { .. })
        ));
        assert_eq!(
            returnable(&r, &alex_line(), Some(2)),
            Err(ReturnBlock::TooMany { returnable: 1 })
        );
    }

    #[test]
    fn photos_are_every_evidence_but_receipts() {
        let e = json!([{"id": "evd_877", "evidence_type": "product_defect"},
                       {"id": "evd_878", "evidence_type": "product_defect"},
                       {"id": "evd_9", "evidence_type": "receipt"}]);
        assert_eq!(photo_count(&e), 2);
        assert_eq!(photo_count(&json!([])), 0);
    }
}
