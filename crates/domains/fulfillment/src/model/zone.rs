//! Shipping zones: which zone an address falls in, what a parcel leg to it costs, and whether
//! two zones claim the same region (`delivery.zone_overlap`). Until a store saves zones
//! (`manage_shipping_zone`), every address uses `delivery.default_zone` from the policy.

use acom_core::Money;
use serde::Serialize;
use serde_json::{Value, json};

use super::Rules;

/// One region a zone covers: a country, or one region (state) of it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize)]
pub struct Region {
    /// ISO 3166-1 alpha-2.
    pub address_country: String,
    /// A state or region code inside the country; `None` covers the whole country.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub address_region: Option<String>,
}

impl Region {
    /// `US`, or `US-AK` for one region.
    pub fn label(&self) -> String {
        match &self.address_region {
            Some(r) => format!("{}-{r}", self.address_country),
            None => self.address_country.clone(),
        }
    }
}

/// Reads `[{address_country, address_region}]` as stored in `shipping_destination`.
pub fn regions(v: &Value) -> Vec<Region> {
    v.as_array()
        .map(|l| {
            l.iter()
                .filter_map(|r| {
                    Some(Region {
                        address_country: r.get("address_country")?.as_str()?.to_uppercase(),
                        address_region: r
                            .get("address_region")
                            .and_then(Value::as_str)
                            .map(str::to_uppercase),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Writes regions back as `[{address_country, address_region}]`.
pub fn regions_json(regions: &[Region]) -> Value {
    json!(regions)
}

/// A shipping zone, stored or the policy default.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Zone {
    /// `zone_…`; `zone_default` for the policy's default zone, which is not stored.
    pub id: String,
    pub name: String,
    /// `standard` or `remote`.
    pub zone_type: String,
    pub shipping_destination: Vec<Region>,
    /// `paid`, `free` or `free_over_min_subtotal`.
    pub fee_type: String,
    pub shipping_rate: Money,
    pub free_over: Option<Money>,
    pub surcharge: Money,
    pub transit_business_days: i64,
}

/// The id the policy's default zone goes by.
pub const DEFAULT_ZONE_ID: &str = "zone_default";

/// `delivery.default_zone`, used while the store has no saved zones.
pub fn default_zone(rules: &Rules) -> Zone {
    Zone {
        id: DEFAULT_ZONE_ID.to_owned(),
        name: rules.zone_name.clone(),
        zone_type: "standard".to_owned(),
        shipping_destination: rules
            .zone_countries
            .iter()
            .map(|c| Region {
                address_country: c.clone(),
                address_region: None,
            })
            .collect(),
        fee_type: "free_over_min_subtotal".to_owned(),
        shipping_rate: rules.shipping_rate,
        free_over: Some(rules.free_shipping_min),
        surcharge: rules.zone_surcharge,
        transit_business_days: rules.transit_business_days,
    }
}

/// The zone an address is in: a zone naming its region wins over one naming only its country.
/// `None` when no zone covers it (the store does not deliver there).
pub fn zone_for<'a>(zones: &'a [Zone], country: &str, region: Option<&str>) -> Option<&'a Zone> {
    let country = country.to_uppercase();
    let region = region.map(str::to_uppercase);
    let exact = zones.iter().find(|z| {
        z.shipping_destination.iter().any(|r| {
            r.address_country == country && r.address_region.is_some() && r.address_region == region
        })
    });
    exact.or_else(|| {
        zones.iter().find(|z| {
            z.shipping_destination
                .iter()
                .any(|r| r.address_country == country && r.address_region.is_none())
        })
    })
}

/// The regions `wanted` shares with other zones (`delivery.zone_overlap: not_allowed`): the
/// same country, or the same region of a country, named twice. A country entry and a region
/// entry of that country do not overlap: the region entry is the exception.
pub fn overlaps(
    zones: &[Zone],
    except_id: Option<&str>,
    wanted: &[Region],
) -> Vec<(String, Region)> {
    let mut out = Vec::new();
    for z in zones.iter().filter(|z| Some(z.id.as_str()) != except_id) {
        for r in &z.shipping_destination {
            if wanted.contains(r) {
                out.push((z.name.clone(), r.clone()));
            }
        }
    }
    out
}

/// What one parcel leg to this zone costs the shopper when the shopper pays shipping: the
/// zone's rate plus its surcharge.
pub fn leg_cost(zone: &Zone) -> Money {
    Money::new(
        zone.shipping_rate.value + zone.surcharge.value,
        zone.shipping_rate.currency,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::tests::{rules, usd};

    fn zone(id: &str, regions: &[(&str, Option<&str>)], surcharge: &str, days: i64) -> Zone {
        Zone {
            id: id.into(),
            name: id.into(),
            zone_type: if surcharge == "0.00" {
                "standard"
            } else {
                "remote"
            }
            .into(),
            shipping_destination: regions
                .iter()
                .map(|(c, r)| Region {
                    address_country: (*c).into(),
                    address_region: r.map(str::to_owned),
                })
                .collect(),
            fee_type: "paid".into(),
            shipping_rate: usd("6.00"),
            free_over: None,
            surcharge: usd(surcharge),
            transit_business_days: days,
        }
    }

    #[test]
    fn a_region_entry_wins_over_its_country() {
        let zones = vec![
            zone("zone_us_contiguous", &[("US", None)], "0.00", 2),
            zone(
                "zone_us_remote",
                &[("US", Some("AK")), ("US", Some("HI"))],
                "12.00",
                5,
            ),
        ];
        assert_eq!(
            zone_for(&zones, "US", Some("NY")).map(|z| z.id.as_str()),
            Some("zone_us_contiguous")
        );
        assert_eq!(
            zone_for(&zones, "us", Some("hi")).map(|z| z.id.as_str()),
            Some("zone_us_remote")
        );
        assert_eq!(zone_for(&zones, "CA", None), None);
        assert_eq!(leg_cost(&zones[1]), usd("18.00"));
    }

    #[test]
    fn the_same_region_in_two_zones_overlaps() {
        let zones = vec![zone("zone_us_remote", &[("US", Some("AK"))], "12.00", 5)];
        let ak = Region {
            address_country: "US".into(),
            address_region: Some("AK".into()),
        };
        let us = Region {
            address_country: "US".into(),
            address_region: None,
        };
        assert_eq!(overlaps(&zones, None, std::slice::from_ref(&ak)).len(), 1);
        assert!(overlaps(&zones, Some("zone_us_remote"), &[ak]).is_empty());
        assert!(overlaps(&zones, None, &[us]).is_empty());
    }

    #[test]
    fn the_default_zone_comes_from_the_policy() {
        let z = default_zone(&rules());
        assert_eq!(z.name, "Contiguous US");
        assert_eq!(z.shipping_rate, usd("6.00"));
        assert_eq!(z.free_over, Some(usd("80.00")));
        assert_eq!(z.transit_business_days, 2);
        assert_eq!(z.shipping_destination[0].label(), "US");
    }
}
