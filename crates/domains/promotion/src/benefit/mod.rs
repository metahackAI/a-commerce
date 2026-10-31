//! `calculate_benefits`: the one place that decides which discounts, member prices, coupons and
//! shipping benefits a cart gets. The cart, checkout and order placement call it through
//! `service::for_sales`, so a shopper always sees the numbers the order will be charged.
//!
//! `calc` is the calculation (pure); this module gathers its facts from the promotion tables and
//! the policies. Carts belong to sales, which depends on this crate, so sales installs the cart
//! reader the tool uses ([`provide_cart_reader`]) when it registers.

pub mod calc;
pub mod calculate_benefits;
pub(crate) mod repo;

pub use calculate_benefits::{CalculateBenefits, CalculateBenefitsInput};

use std::future::Future;
use std::pin::Pin;
use std::str::FromStr;
use std::sync::OnceLock;

use acom_contract::{ErrorClass, ErrorCode, RegistryBuilder, ToolError};
use acom_core::{Currency, Money};
use acom_policy::PolicySet;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use serde_json::{Value, json};

use calc::{Breakdown, ItemPrice, Line, OrderOffer, RuleViolation, Rules, UsableCoupon};

pub const CART_NOT_FOUND: ErrorCode = ErrorCode::new("cart_not_found", ErrorClass::NotFound);

/// Adds `calculate_benefits` to the registry, and lets customer's `get_tier` count the
/// promotions and coupons that name a tier.
pub(crate) fn register(b: &mut RegistryBuilder<Cx>) {
    acom_customer::service::providers::provide_tier_references(tier_references);
    b.read(CalculateBenefits);
}

/// What a reader returns: a boxed future borrowing the transaction.
pub type Reading<'a, T> = Pin<Box<dyn Future<Output = Result<T, ToolError>> + Send + 'a>>;

/// A cart as the calculation needs it, read by sales.
#[derive(Debug, Clone, PartialEq)]
pub struct BenefitCart {
    pub cart_id: String,
    /// The signed-in shopper who owns it, if any.
    pub customer_id: Option<String>,
    /// The guest session that owns it, if any.
    pub session_id: Option<String>,
    /// The tier of the customer the calculation is for.
    pub tier_id: Option<String>,
    pub lines: Vec<Line>,
}

/// Reads cart `cart_id` priced for customer `customer_id`; `None` when there is no such cart.
pub type CartReader =
    for<'a> fn(&'a mut TxCx, &'a str, &'a str) -> Reading<'a, Option<BenefitCart>>;

static CART_READER: OnceLock<CartReader> = OnceLock::new();

/// Sales installs its cart reader here when it registers.
pub fn provide_cart_reader(f: CartReader) {
    let _ = CART_READER.set(f);
}

/// One calculation request: the priced lines and who is buying.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub customer_id: Option<String>,
    pub tier_id: Option<String>,
    pub lines: Vec<Line>,
    /// Code-entry or downloaded coupons the shopper chose.
    pub coupon_ids: Vec<String>,
    pub spend_loyalty: Option<Money>,
}

fn internal(message: impl Into<String>) -> ToolError {
    ToolError::new(ErrorCode::INTERNAL, message)
}

fn decimal(v: &Value) -> Option<Decimal> {
    match v {
        Value::Number(n) => Decimal::from_str(&n.to_string())
            .or_else(|_| Decimal::from_scientific(&n.to_string()))
            .ok(),
        Value::String(s) => Decimal::from_str(s).ok(),
        _ => None,
    }
}

fn policy_money(policy: &PolicySet, path: &str) -> Result<Option<Money>, ToolError> {
    policy
        .lookup(path)
        .map(|v| {
            serde_json::from_value::<Money>(v.clone())
                .map(|m| calc::money(m.value, m.currency))
                .map_err(|e| internal(format!("policy {path}: {e}")))
        })
        .transpose()
}

/// The policy values the calculation uses (promotion.yml, order.yml, catalog.yml).
pub fn rules(policy: &PolicySet) -> Result<Rules, ToolError> {
    let member_benefits = policy
        .lookup("promotion.member_benefits_per_order")
        .and_then(Value::as_u64)
        .ok_or_else(|| internal("policy promotion.member_benefits_per_order is missing"))?;
    let max_rate = policy
        .lookup("promotion.discount_layers.max_total_discount_rate")
        .and_then(decimal)
        .ok_or_else(|| {
            internal("policy promotion.discount_layers.max_total_discount_rate is missing")
        })?;
    let standard_shipping = policy_money(policy, "order.shipping.standard_rate")?
        .ok_or_else(|| internal("policy order.shipping.standard_rate is missing"))?;
    let multiplier = match policy
        .lookup("catalog.price_floor.base")
        .and_then(Value::as_str)
    {
        Some("cost") => policy
            .lookup("catalog.price_floor.multiplier")
            .and_then(decimal),
        _ => None,
    };
    Ok(Rules {
        member_benefits_per_order: usize::try_from(member_benefits).unwrap_or(usize::MAX),
        max_total_discount_rate: max_rate,
        standard_shipping,
        free_shipping_min: policy_money(policy, "order.free_shipping_min_subtotal.amount")?,
        free_shipping_before_benefits: policy
            .lookup("order.free_shipping_min_subtotal.measured")
            .and_then(Value::as_str)
            == Some("before_member_benefits"),
        price_floor_multiplier: multiplier,
    })
}

/// Whether `now` is inside a period whose end is inclusive to the minute
/// (`valid_through: 23:59` covers 23:59:59).
pub fn in_period(from: DateTime<Utc>, through: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    from <= now && through.is_none_or(|t| now < t + Duration::minutes(1))
}

/// Whether a promotion gives its benefit now: scheduled or running, and inside its period (a
/// scheduled promotion whose start has passed is running by the clock).
pub fn promotion_active(
    status: &str,
    from: DateTime<Utc>,
    through: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> bool {
    matches!(status, "scheduled" | "running") && in_period(from, through, now)
}

/// Whether a tier-limited benefit is for this shopper (`None` limit = everyone).
pub fn tier_matches(limit: Option<&str>, tier: Option<&str>) -> bool {
    limit.is_none_or(|l| tier == Some(l))
}

fn money_of(value: Option<Decimal>, currency: Option<&str>) -> Option<Money> {
    let c = Currency::parse(currency?.trim()).ok()?;
    Some(calc::money(value?, c))
}

/// Gathers the facts for `req` from the promotion tables and policies and runs the layers.
pub async fn calculate(tx: &mut TxCx, req: &Request) -> Result<Breakdown, ToolError> {
    let now = tx.clock.now();
    let tier = req.tier_id.as_deref();
    let rules = rules(&tx.policy)?;

    // Item prices that apply to this shopper now.
    let offer_ids: Vec<String> = req.lines.iter().map(|l| l.offer_id.clone()).collect();
    let mut item_prices = Vec::new();
    let mut active_promotions: Vec<(String, DateTime<Utc>, Option<DateTime<Utc>>)> = Vec::new();
    for r in repo::sale_prices(tx, &offer_ids).await? {
        let promotion_ok = match (&r.promotion_status, r.promotion_from) {
            (Some(s), Some(from)) => promotion_active(s, from, r.promotion_through, now),
            _ => r.promotion_id.is_none(),
        };
        if !(promotion_ok
            && in_period(r.valid_from, r.valid_through, now)
            && tier_matches(r.valid_for_member_tier.as_deref(), tier))
        {
            continue;
        }
        let Some(price) = money_of(Some(r.price), Some(&r.price_currency)) else {
            return Err(internal(format!(
                "sale price of {}: bad currency",
                r.offer_id
            )));
        };
        if let (Some(id), Some(from)) = (&r.promotion_id, r.promotion_from)
            && !active_promotions.iter().any(|(p, _, _)| p == id)
        {
            active_promotions.push((id.clone(), from, r.promotion_through));
        }
        item_prices.push(ItemPrice {
            offer_id: r.offer_id,
            promotion_id: r.promotion_id,
            label: r
                .promotion_name
                .unwrap_or_else(|| "Member price".to_owned()),
            price,
        });
    }

    // Order discounts and add-ons that are on now and meant for this shopper.
    let order_rows: Vec<repo::PromotionRow> = repo::order_promotions(tx)
        .await?
        .into_iter()
        .filter(|p| {
            promotion_active(&p.promotion_status, p.valid_from, p.valid_through, now)
                && tier_matches(p.valid_for_member_tier.as_deref(), tier)
        })
        .collect();
    for p in &order_rows {
        active_promotions.push((p.id.clone(), p.valid_from, p.valid_through));
    }

    // Coupons: those that record an applied promotion, and those the shopper named.
    let promotion_ids: Vec<String> = active_promotions
        .iter()
        .map(|(p, _, _)| p.clone())
        .collect();
    let coupon_rows = repo::coupons(tx, &promotion_ids, &req.coupon_ids).await?;
    let missing: Vec<String> = coupon_rows
        .iter()
        .filter(|c| !promotion_ids.contains(&c.promotion_id))
        .map(|c| c.promotion_id.clone())
        .collect();
    let named_promotions = repo::promotions(tx, &missing).await?;
    let held = match &req.customer_id {
        Some(c) => repo::held_coupons(tx, c).await?,
        None => Vec::new(),
    };
    let mut coupons = Vec::new();
    let mut refused = Vec::new();
    for c in &coupon_rows {
        let period = active_promotions
            .iter()
            .find(|(p, _, _)| p == &c.promotion_id)
            .map(|(_, f, t)| (*f, *t))
            .or_else(|| {
                named_promotions
                    .iter()
                    .find(|p| {
                        p.id == c.promotion_id
                            && promotion_active(
                                &p.promotion_status,
                                p.valid_from,
                                p.valid_through,
                                now,
                            )
                    })
                    .map(|p| (p.valid_from, p.valid_through))
            });
        let given = req.coupon_ids.contains(&c.id);
        let holding = held
            .iter()
            .find(|h| h.coupon_id == c.id && in_period(h.owned_from, h.owned_through, now));
        let check = coupon_check(c, period, tier, holding.is_some(), given, now);
        match check {
            Ok(()) => coupons.push(UsableCoupon {
                coupon_id: c.id.clone(),
                code: c.discount_code.clone(),
                promotion_id: c.promotion_id.clone(),
                stacking: c.stacking.clone(),
                tier_coupon: c.valid_for_member_tier.is_some(),
                member_coupon_id: holding.map(|h| h.id.clone()),
            }),
            Err(rule) if given => refused.push(RuleViolation {
                rule: rule.to_owned(),
                detail: format!("{} cannot be used in this order", c.discount_code),
            }),
            Err(_) => {}
        }
    }
    for id in &req.coupon_ids {
        if !coupon_rows.iter().any(|c| &c.id == id) {
            refused.push(RuleViolation {
                rule: "coupon.validity".into(),
                detail: format!("There is no coupon {id}"),
            });
        }
    }

    let mut order_offers = Vec::new();
    let mut free_shipping_promotion = None;
    for p in &order_rows {
        match p.discount_layer.as_str() {
            "order_discount" => {
                let scope = p.applies_to.get("scope").and_then(Value::as_str);
                let product_ids = match scope {
                    None | Some("order") => None,
                    Some("products") => Some(
                        p.applies_to["product_ids"]
                            .as_array()
                            .map(|ids| {
                                ids.iter()
                                    .filter_map(Value::as_str)
                                    .map(str::to_owned)
                                    .collect()
                            })
                            .unwrap_or_default(),
                    ),
                    // Categories belong to merchandising, which this crate cannot read.
                    Some(_) => continue,
                };
                order_offers.push(OrderOffer {
                    promotion_id: p.id.clone(),
                    label: p.name.clone(),
                    percent_off: p.percent_off,
                    amount_off: money_of(p.amount_off_value, p.amount_off_currency.as_deref()),
                    min_subtotal: money_of(
                        p.eligible_transaction_volume_value,
                        p.eligible_transaction_volume_currency.as_deref(),
                    ),
                    product_ids,
                    needs_coupon: coupon_rows.iter().any(|c| c.promotion_id == p.id),
                });
            }
            _ if free_shipping_promotion.is_none()
                && p.eligible_transaction_volume_value.is_some() =>
            {
                free_shipping_promotion = Some(p.id.clone());
            }
            _ => {}
        }
    }

    let currency = req
        .lines
        .first()
        .map_or(rules.standard_shipping.currency, |l| l.list_price.currency);
    let inputs = calc::Inputs {
        lines: req.lines.clone(),
        item_prices,
        coupons,
        order_offers,
        free_shipping_promotion,
        spend_loyalty: req.spend_loyalty,
        // Loyalty balances are not kept yet, so nothing can be spent.
        loyalty_balance: Money::zero(currency),
        rules,
        refused_coupons: refused,
    };
    calc::calculate(&inputs).map_err(internal)
}

/// Whether a coupon can be used now, or the rule that stops it.
fn coupon_check(
    c: &repo::CouponRow,
    promotion_period: Option<(DateTime<Utc>, Option<DateTime<Utc>>)>,
    tier: Option<&str>,
    held: bool,
    given: bool,
    now: DateTime<Utc>,
) -> Result<(), &'static str> {
    if !tier_matches(c.valid_for_member_tier.as_deref(), tier) {
        return Err("coupon.tier_match");
    }
    // `coupon.validity: inherit_promotion`: the coupon's own dates, else its promotion's.
    let valid = match (c.valid_from, promotion_period) {
        (Some(from), _) => in_period(from, c.valid_through, now) && promotion_period.is_some(),
        (None, Some((from, through))) => in_period(from, c.valid_through.or(through), now),
        (None, None) => false,
    };
    if !valid {
        return Err("coupon.validity");
    }
    let reachable = match c.application.as_str() {
        "automatic_at_checkout" => true,
        "code_entry" => given || held,
        _ => held,
    };
    if reachable {
        Ok(())
    } else {
        Err("coupon.applications")
    }
}

/// Redeems the coupons a placed order records: a held coupon becomes `used`; an automatic or
/// code-entry coupon gets a `used` member coupon row. The same order twice changes nothing.
pub async fn redeem(
    tx: &mut TxCx,
    customer_id: &str,
    order_id: &str,
    recorded: &[calc::RecordedCoupon],
) -> Result<Vec<String>, ToolError> {
    let mut ids = Vec::new();
    for r in recorded {
        if let Some(id) = repo::used_on_order(tx, &r.coupon_id, order_id).await? {
            ids.push(id);
            continue;
        }
        let benefit = (r.benefit.value, r.benefit.currency.as_str());
        match &r.member_coupon_id {
            Some(held) => {
                if !repo::use_held(tx, held, order_id, benefit).await? {
                    return Err(ToolError::new(
                        NOT_ELIGIBLE,
                        format!("Coupon {} was already used.", r.code),
                    )
                    .detail(json!({"coupon_id": r.coupon_id})));
                }
                ids.push(held.clone());
            }
            None => {
                let id = acom_core::ids::MemberCouponId::generate();
                repo::insert_used(
                    tx,
                    id.as_str(),
                    &r.coupon_id,
                    customer_id,
                    order_id,
                    benefit,
                )
                .await?;
                ids.push(id.to_string());
            }
        }
    }
    Ok(ids)
}

/// A coupon the order recorded can no longer be used.
pub const NOT_ELIGIBLE: ErrorCode = ErrorCode::new("not_eligible", ErrorClass::Domain);

fn tier_references<'a>(
    tx: &'a mut TxCx,
    tier_id: &'a str,
) -> acom_customer::service::providers::Reading<'a, acom_customer::service::providers::TierReferences>
{
    Box::pin(async move {
        let (promotions, coupons) = repo::tier_references(tx, tier_id).await?;
        Ok(acom_customer::service::providers::TierReferences {
            active_promotions: promotions,
            active_coupons: coupons,
        })
    })
}

/// The spec's output shape of `calculate_benefits`.
pub fn breakdown_json(cart_id: &str, b: &Breakdown) -> Value {
    json!({
        "cart_id": cart_id,
        "layers": b.layers,
        "subtotal": b.subtotal,
        "discount_total": b.discount_total,
        "loyalty_applied": b.loyalty_applied,
        "shipping": b.shipping,
        "total": b.total,
        "rule_violations": b.rule_violations,
        "below_price_floor_items": b.below_price_floor_items,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn defaults() -> PolicySet {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../policies");
        let texts: Vec<(&str, String)> = acom_policy::FILES
            .iter()
            .map(|f| {
                (
                    *f,
                    std::fs::read_to_string(format!("{dir}/{f}.yml")).unwrap(),
                )
            })
            .collect();
        PolicySet::from_defaults(texts.iter().map(|(f, t)| (*f, t.as_str()))).unwrap()
    }

    #[test]
    fn reads_the_numbers_from_the_policies() {
        let r = rules(&defaults()).unwrap();
        assert_eq!(r.member_benefits_per_order, 1);
        assert_eq!(r.max_total_discount_rate, Decimal::from_str("0.5").unwrap());
        assert_eq!(r.standard_shipping.value.to_string(), "6.00");
        assert_eq!(r.free_shipping_min.unwrap().value.to_string(), "80.00");
        assert!(r.free_shipping_before_benefits);
        assert_eq!(
            r.price_floor_multiplier,
            Some(Decimal::from_str("1.15").unwrap())
        );
    }

    #[test]
    fn the_vip_weekend_runs_saturday_and_sunday_inclusive() {
        let from = at("2026-11-07T00:00:00-05:00");
        let through = Some(at("2026-11-08T23:59:00-05:00"));
        assert!(!promotion_active(
            "scheduled",
            from,
            through,
            at("2026-11-06T11:00:00-05:00")
        ));
        assert!(promotion_active(
            "scheduled",
            from,
            through,
            at("2026-11-07T10:00:00-05:00")
        ));
        assert!(promotion_active(
            "running",
            from,
            through,
            at("2026-11-08T23:59:30-05:00")
        ));
        assert!(!promotion_active(
            "running",
            from,
            through,
            at("2026-11-09T00:00:00-05:00")
        ));
        assert!(!promotion_active(
            "paused",
            from,
            through,
            at("2026-11-07T10:00:00-05:00")
        ));
        assert!(tier_matches(Some("vip"), Some("vip")));
        assert!(!tier_matches(Some("vip"), Some("member")));
        assert!(!tier_matches(Some("vip"), None));
        assert!(tier_matches(None, None));
    }

    fn coupon(application: &str, tier: Option<&str>) -> repo::CouponRow {
        repo::CouponRow {
            id: "cpn_vipweekend20".into(),
            discount_code: "VIPWEEKEND20".into(),
            promotion_id: "promo_1042".into(),
            valid_for_member_tier: tier.map(str::to_owned),
            application: application.into(),
            stacking: "no_tier_coupons".into(),
            valid_from: None,
            valid_through: None,
        }
    }

    #[test]
    fn coupons_reach_the_right_members() {
        let now = at("2026-11-07T10:00:00-05:00");
        let weekend = Some((
            at("2026-11-07T00:00:00-05:00"),
            Some(at("2026-11-08T23:59:00-05:00")),
        ));
        let auto = coupon("automatic_at_checkout", Some("vip"));
        assert_eq!(
            coupon_check(&auto, weekend, Some("vip"), false, false, now),
            Ok(())
        );
        assert_eq!(
            coupon_check(&auto, weekend, Some("gold"), false, false, now),
            Err("coupon.tier_match")
        );
        assert_eq!(
            coupon_check(&auto, None, Some("vip"), false, false, now),
            Err("coupon.validity")
        );
        let download = coupon("download", None);
        assert_eq!(
            coupon_check(&download, weekend, None, false, true, now),
            Err("coupon.applications")
        );
        assert_eq!(
            coupon_check(&download, weekend, None, true, false, now),
            Ok(())
        );
        let code = coupon("code_entry", None);
        assert_eq!(coupon_check(&code, weekend, None, false, true, now), Ok(()));
    }

    #[test]
    fn input_schema_is_closed() {
        let s = closed_schema_for::<CalculateBenefitsInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(s["required"], json!(["cart_id", "customer_id"]));
    }

    /// Integration: on the demo clock (Fri Nov 6) no sale is running yet, so Jamie pays the list
    /// price. The VIP weekend price ($71.20 on Sat Nov 7) is checked by the scene test in
    /// `services/commerce/tests/scenes.rs` (`DATABASE_URL`, demo seed).
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn prices_jamies_windbreaker_from_the_database() {
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let db = acom_store::Db::connect(&url, 2).await.expect("connect");
        let tenant = acom_core::TenantId::parse("store_northwind").unwrap();
        let runtime = acom_store::Runtime::new(db, defaults(), true);
        let cx = runtime
            .context(
                acom_auth::Principal::system(tenant, "sys_test"),
                "req_test".into(),
            )
            .await
            .expect("context");
        let mut tx = cx.read().await.expect("tx");
        let b = calculate(
            &mut tx,
            &Request {
                customer_id: Some("cus_jamie".into()),
                tier_id: Some("vip".into()),
                lines: vec![Line {
                    cart_line_id: "cln_1".into(),
                    variant_id: "var_windbreaker_m_slate".into(),
                    product_id: "prd_windbreaker".into(),
                    offer_id: "off_windbreaker_m_slate".into(),
                    quantity: 1,
                    list_price: calc::money(Decimal::from(89), Currency::USD),
                    cost: Some(calc::money(Decimal::from(36), Currency::USD)),
                }],
                coupon_ids: vec![],
                spend_loyalty: None,
            },
        )
        .await
        .expect("calculate");
        assert_eq!(b.total.value.to_string(), "89.00");
    }
}
