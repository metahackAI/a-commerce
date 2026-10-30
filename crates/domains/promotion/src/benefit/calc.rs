//! The benefit calculation itself. Pure: the cart, checkout, order placement and the
//! `calculate_benefits` tool all get the same numbers from the same facts.
//!
//! Layers run in `discount_layers.order`: item prices (lowest sale price per line), one order
//! discount (`pick_one`), then add-ons (loyalty points, free shipping). Coupons tied to an
//! applied promotion record its benefit and add nothing (`coupon.records_benefit_only`); at most
//! `member_benefits_per_order` coupons are recorded per order. Amounts are rounded per line to
//! the currency's minor unit and every total is the sum of rounded parts.

use acom_core::{Currency, Money};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::Serialize;

/// One cart line as the calculator sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub cart_line_id: String,
    pub variant_id: String,
    pub product_id: String,
    pub offer_id: String,
    pub quantity: u32,
    /// Unit list price.
    pub list_price: Money,
    /// Unit cost, for the price floor.
    pub cost: Option<Money>,
}

/// A sale price that applies to one offer for this shopper right now.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemPrice {
    pub offer_id: String,
    pub promotion_id: Option<String>,
    /// What the cart shows ("VIP weekend").
    pub label: String,
    pub price: Money,
}

/// A coupon the shopper can use in this order.
#[derive(Debug, Clone, PartialEq)]
pub struct UsableCoupon {
    pub coupon_id: String,
    pub code: String,
    pub promotion_id: String,
    /// `coupon_stacking` word.
    pub stacking: String,
    /// Limited to a member tier (a tier coupon).
    pub tier_coupon: bool,
    /// The shopper's own issued coupon (`mcp_…`), when they hold one.
    pub member_coupon_id: Option<String>,
}

/// An order discount the shopper may get.
#[derive(Debug, Clone, PartialEq)]
pub struct OrderOffer {
    pub promotion_id: String,
    pub label: String,
    /// 0–100.
    pub percent_off: Option<Decimal>,
    pub amount_off: Option<Money>,
    /// Item subtotal before member benefits the order must reach.
    pub min_subtotal: Option<Money>,
    /// Only lines of these products count; `None` for the whole order.
    pub product_ids: Option<Vec<String>>,
    /// Needs one of the shopper's coupons; otherwise everyone eligible gets it.
    pub needs_coupon: bool,
}

/// The policy values the calculation uses.
#[derive(Debug, Clone, PartialEq)]
pub struct Rules {
    /// `promotion.member_benefits_per_order`.
    pub member_benefits_per_order: usize,
    /// `promotion.discount_layers.max_total_discount_rate`, 0–1.
    pub max_total_discount_rate: Decimal,
    /// `order.shipping.standard_rate`.
    pub standard_shipping: Money,
    /// `order.free_shipping_min_subtotal.amount`.
    pub free_shipping_min: Option<Money>,
    /// `order.free_shipping_min_subtotal.measured` is `before_member_benefits`.
    pub free_shipping_before_benefits: bool,
    /// `catalog.price_floor.multiplier` on cost.
    pub price_floor_multiplier: Option<Decimal>,
}

/// Everything one calculation needs.
#[derive(Debug, Clone, PartialEq)]
pub struct Inputs {
    pub lines: Vec<Line>,
    pub item_prices: Vec<ItemPrice>,
    pub coupons: Vec<UsableCoupon>,
    pub order_offers: Vec<OrderOffer>,
    /// The running free-shipping promotion to credit, if there is one.
    pub free_shipping_promotion: Option<String>,
    pub spend_loyalty: Option<Money>,
    pub loyalty_balance: Money,
    pub rules: Rules,
    /// Given coupons that cannot be used, with the rule that stops them.
    pub refused_coupons: Vec<RuleViolation>,
}

/// One line's sale price (`layers.item_price[]`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemBenefit {
    pub cart_line_id: String,
    pub promotion_id: Option<String>,
    pub label: String,
    pub list_price: Money,
    pub item_price: Money,
    /// (list price − item price) × quantity.
    pub amount: Money,
    /// The coupon that records this benefit on the order.
    pub recorded_by: Option<String>,
}

/// The order discount (`layers.order_discount[]`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OrderBenefit {
    pub promotion_id: String,
    pub label: String,
    pub amount: Money,
    pub recorded_by: Option<String>,
}

/// An add-on (`layers.add_on[]`): `free_shipping` or `loyalty`.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AddOnBenefit {
    pub kind: &'static str,
    pub promotion_id: Option<String>,
    pub amount: Money,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize)]
pub struct Layers {
    pub item_price: Vec<ItemBenefit>,
    pub order_discount: Vec<OrderBenefit>,
    pub add_on: Vec<AddOnBenefit>,
}

/// A rule the calculation could not satisfy (not an error).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RuleViolation {
    /// Policy key.
    pub rule: String,
    pub detail: String,
}

/// A line whose price after benefits is under the price floor.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FloorItem {
    pub cart_line_id: String,
    pub variant_id: String,
    pub unit_price: Money,
    pub floor: Money,
}

/// A coupon recorded on the order, with the benefit it records.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedCoupon {
    pub coupon_id: String,
    pub code: String,
    pub promotion_id: String,
    pub member_coupon_id: Option<String>,
    pub benefit: Money,
}

/// What one line pays after every layer.
#[derive(Debug, Clone, PartialEq)]
pub struct LineResult {
    pub cart_line_id: String,
    /// Unit price after the item layer.
    pub unit_price: Money,
    pub item_discount: Money,
    /// This line's share of the order discount.
    pub order_discount: Money,
    /// The promotion that priced the line (item layer first, else the order discount).
    pub promotion_id: Option<String>,
}

/// One discount line as a cart shows it: one per promotion.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DiscountLine {
    pub label: String,
    pub amount: Money,
    pub promotion_id: Option<String>,
    /// `discount_layer` word.
    pub discount_layer: &'static str,
}

/// The result of one calculation.
#[derive(Debug, Clone, PartialEq)]
pub struct Breakdown {
    pub currency: Currency,
    pub subtotal: Money,
    pub layers: Layers,
    /// Item and order discounts; free shipping and loyalty are not discounts.
    pub discount_total: Money,
    pub loyalty_applied: Money,
    pub shipping: Money,
    pub total: Money,
    pub rule_violations: Vec<RuleViolation>,
    pub below_price_floor_items: Vec<FloorItem>,
    pub lines: Vec<LineResult>,
    pub coupons_recorded: Vec<RecordedCoupon>,
}

impl Breakdown {
    /// The discounts a cart shows: one line per promotion, item prices first.
    pub fn discounts(&self) -> Vec<DiscountLine> {
        let mut out: Vec<DiscountLine> = Vec::new();
        for b in &self.layers.item_price {
            match out
                .iter_mut()
                .find(|d| d.promotion_id == b.promotion_id && d.label == b.label)
            {
                Some(d) => d.amount = money(d.amount.value + b.amount.value, self.currency),
                None => out.push(DiscountLine {
                    label: b.label.clone(),
                    amount: b.amount,
                    promotion_id: b.promotion_id.clone(),
                    discount_layer: "item_price",
                }),
            }
        }
        for b in &self.layers.order_discount {
            out.push(DiscountLine {
                label: b.label.clone(),
                amount: b.amount,
                promotion_id: Some(b.promotion_id.clone()),
                discount_layer: "order_discount",
            });
        }
        out
    }
}

/// Rounds half away from zero to the currency's minor unit and writes that many decimals.
pub fn money(value: Decimal, currency: Currency) -> Money {
    let mut v = value.round_dp_with_strategy(
        currency.minor_units(),
        RoundingStrategy::MidpointAwayFromZero,
    );
    v.rescale(currency.minor_units());
    Money::new(v, currency)
}

/// Runs the layers. Fails only when lines are priced in different currencies.
pub fn calculate(i: &Inputs) -> Result<Breakdown, String> {
    let currency = i
        .lines
        .first()
        .map_or(i.rules.standard_shipping.currency, |l| {
            l.list_price.currency
        });
    if let Some(l) = i.lines.iter().find(|l| l.list_price.currency != currency) {
        return Err(format!(
            "line {} is priced in {}, the cart in {currency}",
            l.cart_line_id, l.list_price.currency
        ));
    }
    let m = |v: Decimal| money(v, currency);
    let qty = |l: &Line| Decimal::from(l.quantity);
    let mut violations = i.refused_coupons.clone();

    let line_totals: Vec<Decimal> = i
        .lines
        .iter()
        .map(|l| m(l.list_price.value * qty(l)).value)
        .collect();
    let subtotal: Decimal = line_totals.iter().copied().sum();

    // Item prices: the lowest sale price below the list price, per line.
    let mut layers = Layers::default();
    let mut item_discounts = vec![Decimal::ZERO; i.lines.len()];
    let mut unit_prices: Vec<Decimal> = i.lines.iter().map(|l| l.list_price.value).collect();
    let mut line_promotions: Vec<Option<String>> = vec![None; i.lines.len()];
    for (n, l) in i.lines.iter().enumerate() {
        let best = i
            .item_prices
            .iter()
            .filter(|p| p.offer_id == l.offer_id && p.price.value < l.list_price.value)
            .min_by(|a, b| a.price.value.cmp(&b.price.value));
        if let Some(p) = best {
            let amount = m((l.list_price.value - p.price.value) * qty(l)).value;
            item_discounts[n] = amount;
            unit_prices[n] = p.price.value;
            line_promotions[n] = p.promotion_id.clone();
            layers.item_price.push(ItemBenefit {
                cart_line_id: l.cart_line_id.clone(),
                promotion_id: p.promotion_id.clone(),
                label: p.label.clone(),
                list_price: m(l.list_price.value),
                item_price: m(p.price.value),
                amount: m(amount),
                recorded_by: None,
            });
        }
    }

    // Coupons that record the applied item prices, largest benefit first, up to the limit.
    let mut by_promotion: Vec<(String, Decimal)> = Vec::new();
    for b in &layers.item_price {
        if let Some(p) = &b.promotion_id {
            match by_promotion.iter_mut().find(|(id, _)| id == p) {
                Some((_, a)) => *a += b.amount.value,
                None => by_promotion.push((p.clone(), b.amount.value)),
            }
        }
    }
    by_promotion.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    let mut recorded: Vec<(&UsableCoupon, Decimal)> = Vec::new();
    for (promotion, amount) in &by_promotion {
        let Some(c) = i.coupons.iter().find(|c| &c.promotion_id == promotion) else {
            continue;
        };
        if recorded.len() >= i.rules.member_benefits_per_order {
            violations.push(RuleViolation {
                rule: "member_benefits_per_order".into(),
                detail: format!(
                    "{} is not recorded: one order records at most {} member benefit(s)",
                    c.code, i.rules.member_benefits_per_order
                ),
            });
            continue;
        }
        if stacks_with(c, recorded.iter().map(|(r, _)| *r)) {
            recorded.push((c, *amount));
        }
    }
    for b in &mut layers.item_price {
        b.recorded_by = recorded
            .iter()
            .find(|(c, _)| Some(&c.promotion_id) == b.promotion_id.as_ref())
            .map(|(c, _)| c.coupon_id.clone());
    }

    // One order discount: the largest one the shopper may combine with what is recorded.
    let nets: Vec<Decimal> = line_totals
        .iter()
        .zip(&item_discounts)
        .map(|(t, d)| t - d)
        .collect();
    let mut best: Option<(&OrderOffer, Option<&UsableCoupon>, Decimal, Vec<usize>)> = None;
    for o in &i.order_offers {
        if o.min_subtotal.is_some_and(|min| subtotal < min.value) {
            continue;
        }
        let coupon = if o.needs_coupon {
            let Some(c) = i.coupons.iter().find(|c| c.promotion_id == o.promotion_id) else {
                continue;
            };
            if recorded.len() >= i.rules.member_benefits_per_order
                || !stacks_with(c, recorded.iter().map(|(r, _)| *r))
            {
                continue;
            }
            Some(c)
        } else {
            None
        };
        let eligible: Vec<usize> = i
            .lines
            .iter()
            .enumerate()
            .filter(|(_, l)| {
                o.product_ids
                    .as_ref()
                    .is_none_or(|ids| ids.contains(&l.product_id))
            })
            .map(|(n, _)| n)
            .collect();
        let base: Decimal = eligible.iter().map(|n| nets[*n]).sum();
        let amount = match (o.percent_off, o.amount_off) {
            (Some(p), _) => m(base * p / Decimal::ONE_HUNDRED).value,
            (None, Some(a)) => a.value.min(base),
            (None, None) => Decimal::ZERO,
        };
        if amount <= Decimal::ZERO {
            continue;
        }
        if best.as_ref().is_none_or(|(_, _, a, _)| amount > *a) {
            best = Some((o, coupon, amount, eligible));
        }
    }
    let mut order_shares = vec![Decimal::ZERO; i.lines.len()];
    if let Some((o, coupon, amount, eligible)) = best {
        let base: Decimal = eligible.iter().map(|n| nets[*n]).sum();
        let mut given = Decimal::ZERO;
        for (k, n) in eligible.iter().enumerate() {
            let share = if k + 1 == eligible.len() {
                amount - given
            } else if base > Decimal::ZERO {
                m(amount * nets[*n] / base).value
            } else {
                Decimal::ZERO
            };
            order_shares[*n] = share;
            given += share;
            if line_promotions[*n].is_none() {
                line_promotions[*n] = Some(o.promotion_id.clone());
            }
        }
        if let Some(c) = coupon {
            recorded.push((c, amount));
        }
        layers.order_discount.push(OrderBenefit {
            promotion_id: o.promotion_id.clone(),
            label: o.label.clone(),
            amount: m(amount),
            recorded_by: coupon.map(|c| c.coupon_id.clone()),
        });
    }

    let item_total: Decimal = item_discounts.iter().copied().sum();
    let order_total: Decimal = order_shares.iter().copied().sum();
    let discount_total = item_total + order_total;

    // Add-ons: loyalty points, then shipping.
    let mut loyalty = Decimal::ZERO;
    if let Some(want) = i.spend_loyalty.filter(|w| w.value > Decimal::ZERO) {
        loyalty = want
            .value
            .min(i.loyalty_balance.value)
            .min(subtotal - discount_total)
            .max(Decimal::ZERO);
        if loyalty < want.value {
            violations.push(RuleViolation {
                rule: "loyalty.spend".into(),
                detail: format!(
                    "Only {} of the {} asked can be spent",
                    m(loyalty),
                    m(want.value)
                ),
            });
        }
        if loyalty > Decimal::ZERO {
            layers.add_on.push(AddOnBenefit {
                kind: "loyalty",
                promotion_id: None,
                amount: m(loyalty),
            });
        }
    }
    let measured = if i.rules.free_shipping_before_benefits {
        subtotal
    } else {
        subtotal - discount_total
    };
    let shipping = if i.lines.is_empty() {
        Decimal::ZERO
    } else if i
        .rules
        .free_shipping_min
        .is_some_and(|min| measured >= min.value)
    {
        layers.add_on.push(AddOnBenefit {
            kind: "free_shipping",
            promotion_id: i.free_shipping_promotion.clone(),
            amount: m(i.rules.standard_shipping.value),
        });
        Decimal::ZERO
    } else {
        i.rules.standard_shipping.value
    };

    if discount_total + loyalty > subtotal * i.rules.max_total_discount_rate {
        violations.push(RuleViolation {
            rule: "discount_layers.max_total_discount_rate".into(),
            detail: format!(
                "Discounts of {} are over {}% of the {} subtotal",
                m(discount_total + loyalty),
                i.rules.max_total_discount_rate * Decimal::ONE_HUNDRED,
                m(subtotal)
            ),
        });
    }

    let mut lines = Vec::with_capacity(i.lines.len());
    let mut below_floor = Vec::new();
    for (n, l) in i.lines.iter().enumerate() {
        let net = nets[n] - order_shares[n];
        if let (Some(cost), Some(k)) = (l.cost, i.rules.price_floor_multiplier) {
            let floor = m(cost.value * k).value;
            let unit = m(net / qty(l)).value;
            if unit < floor {
                below_floor.push(FloorItem {
                    cart_line_id: l.cart_line_id.clone(),
                    variant_id: l.variant_id.clone(),
                    unit_price: m(unit),
                    floor: m(floor),
                });
            }
        }
        lines.push(LineResult {
            cart_line_id: l.cart_line_id.clone(),
            unit_price: m(unit_prices[n]),
            item_discount: m(item_discounts[n]),
            order_discount: m(order_shares[n]),
            promotion_id: line_promotions[n].clone(),
        });
    }

    Ok(Breakdown {
        currency,
        subtotal: m(subtotal),
        layers,
        discount_total: m(discount_total),
        loyalty_applied: m(loyalty),
        shipping: m(shipping),
        total: m(subtotal - discount_total - loyalty + shipping),
        rule_violations: violations,
        below_price_floor_items: below_floor,
        lines,
        coupons_recorded: recorded
            .into_iter()
            .map(|(c, amount)| RecordedCoupon {
                coupon_id: c.coupon_id.clone(),
                code: c.code.clone(),
                promotion_id: c.promotion_id.clone(),
                member_coupon_id: c.member_coupon_id.clone(),
                benefit: m(amount),
            })
            .collect(),
    })
}

/// Whether `c` may be used with the coupons already recorded (`coupon_stacking`).
fn stacks_with<'a>(c: &UsableCoupon, recorded: impl Iterator<Item = &'a UsableCoupon>) -> bool {
    for r in recorded {
        if r.coupon_id == c.coupon_id
            || r.stacking == "no_other_coupons"
            || c.stacking == "no_other_coupons"
            || (r.stacking == "no_tier_coupons" && c.tier_coupon)
            || (c.stacking == "no_tier_coupons" && r.tier_coupon)
        {
            return false;
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    fn usd(s: &str) -> Money {
        Money::new(Decimal::from_str(s).unwrap(), Currency::USD)
    }

    fn rules() -> Rules {
        Rules {
            member_benefits_per_order: 1,
            max_total_discount_rate: Decimal::from_str("0.50").unwrap(),
            standard_shipping: usd("6.00"),
            free_shipping_min: Some(usd("80.00")),
            free_shipping_before_benefits: true,
            price_floor_multiplier: Some(Decimal::from_str("1.15").unwrap()),
        }
    }

    fn line(id: &str, product: &str, price: &str, cost: &str, quantity: u32) -> Line {
        Line {
            cart_line_id: id.into(),
            variant_id: format!("var_{product}_m"),
            product_id: format!("prd_{product}"),
            offer_id: format!("off_{product}_m"),
            quantity,
            list_price: usd(price),
            cost: Some(usd(cost)),
        }
    }

    fn vip_price(product: &str, price: &str) -> ItemPrice {
        ItemPrice {
            offer_id: format!("off_{product}_m"),
            promotion_id: Some("promo_1042".into()),
            label: "VIP weekend".into(),
            price: usd(price),
        }
    }

    fn vip_coupon() -> UsableCoupon {
        UsableCoupon {
            coupon_id: "cpn_vipweekend20".into(),
            code: "VIPWEEKEND20".into(),
            promotion_id: "promo_1042".into(),
            stacking: "no_tier_coupons".into(),
            tier_coupon: true,
            member_coupon_id: None,
        }
    }

    fn inputs(lines: Vec<Line>) -> Inputs {
        Inputs {
            lines,
            item_prices: vec![],
            coupons: vec![],
            order_offers: vec![],
            free_shipping_promotion: Some("promo_free_ship_80".into()),
            spend_loyalty: None,
            loyalty_balance: usd("0.00"),
            rules: rules(),
            refused_coupons: vec![],
        }
    }

    /// F2: Jamie (VIP) on Sat Nov 7 with the windbreaker M Slate in the cart.
    #[test]
    fn jamie_gets_the_vip_weekend_price_and_free_shipping() {
        let mut i = inputs(vec![line("cln_1", "windbreaker", "89.00", "36.00", 1)]);
        i.item_prices = vec![vip_price("windbreaker", "71.20")];
        i.coupons = vec![vip_coupon()];
        let b = calculate(&i).unwrap();
        assert_eq!(b.subtotal, usd("89.00"));
        assert_eq!(b.discount_total, usd("17.80"));
        assert_eq!(b.shipping, usd("0.00"));
        assert_eq!(b.total, usd("71.20"));
        assert_eq!(
            b.discounts(),
            vec![DiscountLine {
                label: "VIP weekend".into(),
                amount: usd("17.80"),
                promotion_id: Some("promo_1042".into()),
                discount_layer: "item_price",
            }]
        );
        assert_eq!(
            b.layers.item_price[0].recorded_by.as_deref(),
            Some("cpn_vipweekend20")
        );
        assert_eq!(b.coupons_recorded.len(), 1);
        assert_eq!(b.coupons_recorded[0].benefit, usd("17.80"));
        assert_eq!(b.layers.add_on[0].kind, "free_shipping");
        assert_eq!(b.layers.add_on[0].amount, usd("6.00"));
        assert!(b.rule_violations.is_empty());
        assert!(b.below_price_floor_items.is_empty());
        let json = serde_json::to_value(&b.layers).unwrap();
        assert_eq!(
            json["item_price"][0]["amount"]["value"].to_string(),
            "17.80"
        );
    }

    /// F1: the same products, VIP weekend prices for Jamie, list prices for a guest.
    #[test]
    fn vip_and_guest_see_different_prices_for_the_same_products() {
        let products = [
            ("windbreaker", "89.00", "36.00", "71.20"),
            ("field_jacket", "128.00", "56.00", "102.40"),
            ("seersucker_overshirt", "79.00", "58.00", "67.15"),
        ];
        for (p, list, cost, vip) in products {
            let mut jamie = inputs(vec![line("cln_1", p, list, cost, 1)]);
            jamie.item_prices = vec![vip_price(p, vip)];
            jamie.coupons = vec![vip_coupon()];
            let b = calculate(&jamie).unwrap();
            assert_eq!(b.lines[0].unit_price, usd(vip), "{p}");
            assert!(b.below_price_floor_items.is_empty(), "{p}");
            let guest = calculate(&inputs(vec![line("cln_1", p, list, cost, 1)])).unwrap();
            assert_eq!(guest.lines[0].unit_price, usd(list), "{p}");
            assert_eq!(guest.discount_total, usd("0.00"), "{p}");
        }
        // The chore coat has no sale price: everyone pays $142.00.
        let coat = calculate(&inputs(vec![line(
            "cln_1",
            "chore_coat",
            "142.00",
            "60.00",
            1,
        )]))
        .unwrap();
        assert_eq!(coat.total, usd("142.00"));
    }

    #[test]
    fn charges_standard_shipping_under_the_threshold_before_benefits() {
        // $79.00 before benefits: shipping is charged even though nothing is discounted.
        let b = calculate(&inputs(vec![line(
            "cln_1",
            "overshirt",
            "79.00",
            "30.00",
            1,
        )]))
        .unwrap();
        assert_eq!(b.shipping, usd("6.00"));
        assert_eq!(b.total, usd("85.00"));
        assert!(b.layers.add_on.is_empty());
        // $89.00 discounted to $71.20 still ships free: measured before member benefits.
        let mut i = inputs(vec![line("cln_1", "windbreaker", "89.00", "36.00", 1)]);
        i.item_prices = vec![vip_price("windbreaker", "71.20")];
        assert_eq!(calculate(&i).unwrap().shipping, usd("0.00"));
        i.rules.free_shipping_before_benefits = false;
        assert_eq!(calculate(&i).unwrap().shipping, usd("6.00"));
    }

    #[test]
    fn empty_cart_costs_nothing() {
        let b = calculate(&inputs(vec![])).unwrap();
        assert_eq!(b.total, usd("0.00"));
        assert_eq!(b.shipping, usd("0.00"));
    }

    fn gold_offer() -> OrderOffer {
        OrderOffer {
            promotion_id: "promo_gold_10".into(),
            label: "Gold tier 10%".into(),
            percent_off: Some(Decimal::from(10)),
            amount_off: None,
            min_subtotal: None,
            product_ids: None,
            needs_coupon: true,
        }
    }

    fn gold_coupon() -> UsableCoupon {
        UsableCoupon {
            coupon_id: "cpn_gold_10".into(),
            code: "GOLD10".into(),
            promotion_id: "promo_gold_10".into(),
            stacking: "no_other_coupons".into(),
            tier_coupon: true,
            member_coupon_id: Some("mcp_gold_1".into()),
        }
    }

    #[test]
    fn a_tier_coupon_takes_ten_percent_and_splits_it_over_lines() {
        let mut i = inputs(vec![
            line("cln_1", "linen_shirt", "68.00", "26.00", 1),
            line("cln_2", "merino_tee", "58.00", "22.00", 2),
        ]);
        i.order_offers = vec![gold_offer()];
        i.coupons = vec![gold_coupon()];
        let b = calculate(&i).unwrap();
        assert_eq!(b.subtotal, usd("184.00"));
        assert_eq!(b.discount_total, usd("18.40"));
        assert_eq!(b.total, usd("165.60"));
        assert_eq!(b.lines[0].order_discount, usd("6.80"));
        assert_eq!(b.lines[1].order_discount, usd("11.60"));
        assert_eq!(
            b.coupons_recorded[0].member_coupon_id.as_deref(),
            Some("mcp_gold_1")
        );
        assert_eq!(
            b.layers.order_discount[0].recorded_by.as_deref(),
            Some("cpn_gold_10")
        );
    }

    #[test]
    fn one_member_benefit_per_order() {
        // A VIP price recorded by its coupon leaves no room for a tier coupon.
        let mut i = inputs(vec![line("cln_1", "windbreaker", "89.00", "36.00", 1)]);
        i.item_prices = vec![vip_price("windbreaker", "71.20")];
        i.coupons = vec![vip_coupon(), gold_coupon()];
        i.order_offers = vec![gold_offer()];
        let b = calculate(&i).unwrap();
        assert_eq!(b.total, usd("71.20"));
        assert!(b.layers.order_discount.is_empty());
        assert_eq!(b.coupons_recorded.len(), 1);
        // With two benefits allowed, `no_other_coupons` still keeps them apart.
        i.rules.member_benefits_per_order = 2;
        assert!(calculate(&i).unwrap().layers.order_discount.is_empty());
    }

    #[test]
    fn picks_the_largest_order_discount_and_honours_minimums() {
        let mut i = inputs(vec![line("cln_1", "boots", "312.00", "130.00", 1)]);
        let birthday = OrderOffer {
            promotion_id: "promo_birthday".into(),
            label: "Birthday coupon".into(),
            percent_off: None,
            amount_off: Some(usd("10.00")),
            min_subtotal: None,
            product_ids: None,
            needs_coupon: false,
        };
        let big_spend = OrderOffer {
            promotion_id: "promo_spend".into(),
            label: "$40 off $400".into(),
            percent_off: None,
            amount_off: Some(usd("40.00")),
            min_subtotal: Some(usd("400.00")),
            product_ids: None,
            needs_coupon: false,
        };
        i.order_offers = vec![birthday, big_spend, gold_offer()];
        let b = calculate(&i).unwrap();
        // Gold needs a coupon the shopper does not have; $40 needs $400.
        assert_eq!(b.layers.order_discount[0].promotion_id, "promo_birthday");
        assert_eq!(b.total, usd("302.00"));
    }

    #[test]
    fn reports_lines_under_the_price_floor_and_too_much_discount() {
        // Overshirt at 20%: $63.20 is under $58.00 × 1.15 = $66.70.
        let mut i = inputs(vec![line(
            "cln_1",
            "seersucker_overshirt",
            "79.00",
            "58.00",
            1,
        )]);
        i.item_prices = vec![vip_price("seersucker_overshirt", "63.20")];
        let b = calculate(&i).unwrap();
        assert_eq!(b.below_price_floor_items.len(), 1);
        assert_eq!(b.below_price_floor_items[0].floor, usd("66.70"));
        i.item_prices = vec![vip_price("seersucker_overshirt", "30.00")];
        let b = calculate(&i).unwrap();
        assert!(
            b.rule_violations
                .iter()
                .any(|v| v.rule == "discount_layers.max_total_discount_rate")
        );
    }

    #[test]
    fn loyalty_beyond_the_balance_is_a_violation_not_an_error() {
        let mut i = inputs(vec![line("cln_1", "windbreaker", "89.00", "36.00", 1)]);
        i.spend_loyalty = Some(usd("5.00"));
        let b = calculate(&i).unwrap();
        assert_eq!(b.loyalty_applied, usd("0.00"));
        assert_eq!(b.rule_violations[0].rule, "loyalty.spend");
        i.loyalty_balance = usd("20.00");
        let b = calculate(&i).unwrap();
        assert_eq!(b.loyalty_applied, usd("5.00"));
        assert_eq!(b.total, usd("84.00"));
    }

    #[test]
    fn totals_add_up_exactly() {
        let mut i = inputs(vec![
            line("cln_1", "a", "33.33", "10.00", 3),
            line("cln_2", "b", "19.99", "5.00", 1),
            line("cln_3", "c", "0.01", "0.00", 7),
        ]);
        i.order_offers = vec![OrderOffer {
            needs_coupon: false,
            ..gold_offer()
        }];
        let b = calculate(&i).unwrap();
        let shares: Decimal = b.lines.iter().map(|l| l.order_discount.value).sum();
        assert_eq!(shares, b.layers.order_discount[0].amount.value);
        assert_eq!(
            b.total.value,
            b.subtotal.value - b.discount_total.value - b.loyalty_applied.value + b.shipping.value
        );
    }

    #[test]
    fn refuses_mixed_currencies() {
        let mut l = line("cln_2", "b", "10.00", "1.00", 1);
        l.list_price = Money::new(Decimal::from(1000), Currency::KRW);
        assert!(calculate(&inputs(vec![line("cln_1", "a", "10.00", "1.00", 1), l])).is_err());
    }
}
