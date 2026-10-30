//! Generated members: people, home addresses, memberships and the Gold tier coupons
//! (`generate.yml` `members`).

use chrono::{DateTime, Datelike, NaiveDate, TimeDelta};
use chrono_tz::Tz;
use serde_json::json;

use super::named::Named;
use super::spec::{Spec, prose};
use super::util::{Dice, at, local, rfc3339};

#[derive(Debug)]
pub(crate) struct Member {
    pub id: String,
    pub number: usize,
    pub given: String,
    pub family: String,
    pub card: String,
    pub registered: DateTime<Tz>,
    pub tier: String,
    pub tier_since: DateTime<Tz>,
    pub address: Address,
}

#[derive(Debug)]
pub(crate) struct Address {
    pub id: String,
    pub street: String,
    pub locality: &'static str,
    pub region: &'static str,
    pub postal_code: &'static str,
}

impl Member {
    pub(crate) fn name(&self) -> String {
        format!("{} {}", self.given, self.family)
    }
}

/// Towns and ZIP codes per north-east state.
const TOWNS: &[(&str, &[(&str, &str)])] = &[
    (
        "NY",
        &[
            ("Brooklyn", "11215"),
            ("Albany", "12207"),
            ("Buffalo", "14202"),
            ("Rochester", "14604"),
            ("Ithaca", "14850"),
            ("Astoria", "11102"),
        ],
    ),
    (
        "NJ",
        &[
            ("Montclair", "07042"),
            ("Trenton", "08608"),
            ("Newark", "07102"),
            ("Morristown", "07960"),
        ],
    ),
    (
        "CT",
        &[
            ("Hartford", "06103"),
            ("New Haven", "06511"),
            ("Norwalk", "06851"),
        ],
    ),
    (
        "MA",
        &[
            ("Boston", "02116"),
            ("Somerville", "02143"),
            ("Worcester", "01608"),
            ("Salem", "01970"),
        ],
    ),
    ("RI", &[("Providence", "02906"), ("Newport", "02840")]),
    ("VT", &[("Burlington", "05401"), ("Montpelier", "05602")]),
    ("ME", &[("Portland", "04101"), ("Bangor", "04401")]),
    (
        "PA",
        &[
            ("Philadelphia", "19103"),
            ("Pittsburgh", "15213"),
            ("Lancaster", "17602"),
        ],
    ),
];

const STREETS: &[&str] = &[
    "Maple St",
    "Oak Ave",
    "Main St",
    "Elm St",
    "Cedar Ln",
    "Park Ave",
    "Church St",
    "High St",
    "Washington St",
    "Spring St",
    "Pine St",
    "River Rd",
    "Hill St",
    "Lake St",
    "Union St",
    "Water St",
    "Chestnut St",
    "Walnut St",
    "Franklin St",
    "School St",
];

pub(crate) fn generate(spec: &Spec, named: &Named, dice: &mut Dice) -> Result<Vec<Member>, String> {
    let m = &spec.members;
    let tz = named.tz;
    let mut tiers: Vec<&str> = Vec::new();
    for (tier, total, named_count, generate) in &m.tiers {
        if named_count + generate != *total {
            return Err(format!(
                "members.{tier}: {named_count} + {generate} is not {total}"
            ));
        }
        tiers.extend(std::iter::repeat_n(tier.as_str(), *generate as usize));
    }
    dice.shuffle(&mut tiers);

    let from = local(tz, m.registered_from);
    let through = local(tz, m.registered_through);
    let minutes = (through - from).num_minutes();
    let last_tier_day = NaiveDate::from_ymd_opt(2026, 11, 1).ok_or("bad date")?;
    let towns: Vec<_> = TOWNS
        .iter()
        .filter(|(state, _)| prose::STATES.contains(state))
        .collect();

    let mut members = Vec::with_capacity(tiers.len());
    for (i, tier) in tiers.into_iter().enumerate() {
        let number = i + 1;
        let given = dice.pick(&m.given_names).clone();
        let family = dice.pick(&m.family_names).clone();
        let card = format!("{:04}", dice.below(10_000));
        let registered = from + TimeDelta::minutes(dice.range(0, minutes));
        let tier_since = if tier == "member" {
            registered
        } else {
            // The first day of a month after registration, up to 2026-11-01, at 03:00.
            let mut months = Vec::new();
            let mut d = NaiveDate::from_ymd_opt(registered.year(), registered.month(), 1)
                .ok_or("bad date")?;
            while d <= last_tier_day {
                let t = at(tz, d, 3, 0);
                if t > registered {
                    months.push(t);
                }
                d = d
                    .checked_add_months(chrono::Months::new(1))
                    .ok_or("bad date")?;
            }
            *dice.pick(&months)
        };
        let (region, list) = **dice.pick(&towns);
        let (locality, postal_code) = *dice.pick(list);
        let mut street = format!("{} {}", dice.range(1, 998), dice.pick(STREETS));
        if dice.below(4) == 0 {
            street.push_str(&format!(", Apt {}", dice.range(1, 24)));
        }
        members.push(Member {
            id: format!("cus_m{number:06}"),
            number,
            given,
            family,
            card,
            registered,
            tier: tier.to_owned(),
            tier_since,
            address: Address {
                id: format!("adr_m{number:06}"),
                street,
                locality,
                region,
                postal_code,
            },
        });
    }
    Ok(members)
}

/// Rows in load order: people, addresses, memberships, then Gold tier coupons.
pub(crate) fn rows(spec: &Spec, named: &Named, members: &[Member], out: &mut crate::SeedRows) {
    let tenant = &spec.store;
    for m in members {
        out.push((
            "customer.person".to_owned(),
            json!({
                "tenant_id": tenant,
                "id": m.id,
                "name": m.name(),
                "given_name": m.given,
                "family_name": m.family,
                "email": format!("m{:06}@example.com", m.number),
                "member_state": spec.members.member_state,
                "member_type": "individual",
                "card_last4": m.card,
                "registered_at": rfc3339(&m.registered),
            }),
        ));
    }
    for m in members {
        out.push((
            "customer.postal_address".to_owned(),
            json!({
                "tenant_id": tenant,
                "id": m.address.id,
                "customer_id": m.id,
                "label": "Home",
                "name": m.name(),
                "street_address": m.address.street,
                "address_locality": m.address.locality,
                "address_region": m.address.region,
                "postal_code": m.address.postal_code,
                "address_country": "US",
                "address_usage": ["shipping", "billing"],
                "is_primary": true,
            }),
        ));
    }
    for m in members {
        out.push((
            "customer.program_membership".to_owned(),
            json!({
                "tenant_id": tenant,
                "customer_id": m.id,
                "member_program_id": named.program_id,
                "tier_id": m.tier,
                "membership_number": format!("NW-1{:05}", m.number),
                "tier_since": rfc3339(&m.tier_since),
            }),
        ));
    }
    let (year, month) = prose::GOLD_COUPON_MONTH;
    let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap_or_default();
    let last = first
        .checked_add_months(chrono::Months::new(1))
        .and_then(|d| d.pred_opt())
        .unwrap_or(first);
    let coupon_suffix = prose::GOLD_COUPON.trim_start_matches("cpn_");
    for m in members.iter().filter(|m| m.tier == prose::GOLD_TIER) {
        out.push((
            "promotion.member_coupon".to_owned(),
            json!({
                "tenant_id": tenant,
                "id": format!("mcp_m{:06}_{coupon_suffix}_{year}{month:02}", m.number),
                "coupon_id": prose::GOLD_COUPON,
                "customer_id": m.id,
                "coupon_status": prose::GOLD_COUPON_STATUS,
                "owned_from": rfc3339(&at(named.tz, first, 0, 0)),
                "owned_through": rfc3339(&at(named.tz, last, 23, 59)),
            }),
        ));
    }
}
