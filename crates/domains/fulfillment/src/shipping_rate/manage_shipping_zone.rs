//! `manage_shipping_zone`: define a delivery zone or change one (its regions, remote-area
//! surcharge and transit days). Rates are set with `set_shipping_rate`.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::Money;
use acom_core::enums::ZoneType;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{zone_not_found, zone_of};
use crate::errors::{OVERLAPPING_ZONE, ZONE_NOT_FOUND};
use crate::model::money_text;
use crate::model::zone::{DEFAULT_ZONE_ID, Region, default_zone, overlaps, regions_json};
use crate::repo::{self, ZoneValues};
use crate::rules;

/// One region of a zone: a whole country, or one state or region of it.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RegionInput {
    /// ISO 3166-1 alpha-2 country code.
    #[schemars(regex(pattern = r"^[A-Z]{2}$"), example = &"US")]
    pub address_country: String,
    /// A state or region code of the country; leave it out for the whole country (a zone
    /// naming one region of it wins over the zone naming the country).
    #[schemars(regex(pattern = r"^[A-Z0-9]{1,3}$"), example = &"AK")]
    pub address_region: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ManageShippingZoneInput {
    /// The zone to change, from `list_shipping_zones`; leave it out to define a new zone.
    #[schemars(regex(pattern = r"^zone_[a-z0-9_]+$"), example = &"zone_us_remote")]
    pub zone_id: Option<String>,
    /// The zone's name, as people say it.
    #[schemars(length(min = 1, max = 120), example = &"Alaska and Hawaii")]
    pub name: String,
    /// The regions the zone covers (schema.org `DefinedRegion`). A region may be in only one
    /// zone.
    #[schemars(length(min = 1, max = 100), example = json!([{"address_country": "US", "address_region": "AK"}, {"address_country": "US", "address_region": "HI"}]))]
    pub shipping_destination: Vec<RegionInput>,
    /// Amount added to every parcel leg to this zone, in the store's currency. Unchanged (or
    /// 0.00 for a new zone) when left out.
    #[schemars(example = json!({"value": 12.00, "currency": "USD"}))]
    pub surcharge: Option<Money>,
    /// Business days a parcel travels to this zone after it ships. Unchanged (or the default
    /// zone's) when left out.
    #[schemars(range(min = 0, max = 60), example = 5)]
    pub transit_business_days: Option<u32>,
    /// standard, or remote for areas that cost more to reach. Unchanged (or standard) when
    /// left out.
    #[schemars(example = &"remote")]
    pub zone_type: Option<ZoneType>,
}

/// The zone before and after, fixed at preview time.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZonePlan {
    pub zone_id: String,
    /// `None` when the zone is new.
    pub before: Option<ZoneValues>,
    pub after: ZoneValues,
}

pub struct ManageShippingZone;

impl Operation for ManageShippingZone {
    const META: OpMeta = OpMeta {
        address: "fulfillment.shipping_rate.manage_zone",
        tool: "manage_shipping_zone",
        group: Group::Transact,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/fulfillment/shipping-zones",
        summary: "Define a delivery zone or change one: its regions, remote-area surcharge and transit days. No region may be in two zones. Set its rate with set_shipping_rate.",
        consumes: &[],
        produces: &[Slot::one("zone_id")],
        checks: &["delivery.zone_overlap", "delivery.remote_area_surcharge"],
        errors: &[OVERLAPPING_ZONE, ZONE_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = ManageShippingZoneInput;
    type Output = Value;
}

/// `Alaska and Hawaii` → `alaska_and_hawaii`.
fn slug(text: &str) -> String {
    let mut out = String::new();
    for ch in text.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if !out.ends_with('_') && !out.is_empty() {
            out.push('_');
        }
    }
    out.trim_end_matches('_').to_owned()
}

fn zone_wire(z: &ZoneValues) -> Value {
    json!({"name": z.name, "zone_type": z.zone_type,
           "shipping_destination": z.shipping_destination, "surcharge": z.surcharge,
           "transit_business_days": z.transit_business_days})
}

impl WriteOp<Cx> for ManageShippingZone {
    type Plan = ZonePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ManageShippingZoneInput,
    ) -> Result<Planned<ZonePlan>, ToolError> {
        let rules = rules(&cx.policy)?;
        let name = input.name.trim().to_owned();
        if name.is_empty() {
            return Err(ToolError::invalid_input("Give the zone a name.")
                .detail(json!({"field": "name", "reason": "missing"})));
        }
        if let Some(s) = &input.surcharge
            && (s.currency != cx.currency || s.value.is_sign_negative())
        {
            return Err(ToolError::invalid_input(format!(
                "The surcharge must be 0 or more, in {}.",
                cx.currency
            ))
            .detail(json!({"field": "surcharge", "reason": "range"})));
        }
        let mut wanted: Vec<Region> = input
            .shipping_destination
            .iter()
            .map(|r| Region {
                address_country: r.address_country.to_uppercase(),
                address_region: r.address_region.as_ref().map(|x| x.to_uppercase()),
            })
            .collect();
        wanted.sort();
        wanted.dedup();

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let rows = repo::zones(&mut tx).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let stored = rows.iter().map(zone_of).collect::<Result<Vec<_>, _>>()?;
        let existing = match &input.zone_id {
            Some(id) => Some(
                rows.iter()
                    .find(|r| &r.id == id)
                    .ok_or_else(|| zone_not_found(id, "list_shipping_zones"))?,
            ),
            None => None,
        };
        let clash = overlaps(&stored, input.zone_id.as_deref(), &wanted);
        if !clash.is_empty() {
            let items: Vec<String> = clash
                .iter()
                .map(|(zone, r)| format!("{} is already in {zone}", r.label()))
                .collect();
            return Err(ToolError::new(
                OVERLAPPING_ZONE,
                format!("{}; a region can be in only one zone.", items.join("; ")),
            )
            .detail(json!({"rule": "delivery.zone_overlap", "items": items}))
            .next("list_shipping_zones"));
        }

        let base = match existing {
            Some(row) => row.values()?,
            None => {
                let d = default_zone(&rules);
                ZoneValues {
                    name: name.clone(),
                    zone_type: "standard".into(),
                    shipping_destination: Value::Array(Vec::new()),
                    fee_type: d.fee_type,
                    shipping_rate: d.shipping_rate,
                    free_over: d.free_over,
                    surcharge: Money::zero(cx.currency),
                    transit_business_days: i32::try_from(d.transit_business_days).unwrap_or(0),
                }
            }
        };
        let after = ZoneValues {
            name: name.clone(),
            zone_type: input
                .zone_type
                .map_or(base.zone_type.clone(), |t| t.as_str().to_owned()),
            shipping_destination: regions_json(&wanted),
            surcharge: input.surcharge.map_or(base.surcharge, Money::rounded),
            transit_business_days: input
                .transit_business_days
                .map_or(base.transit_business_days, |d| {
                    i32::try_from(d).unwrap_or(i32::MAX)
                }),
            ..base.clone()
        };
        let zone_id = match existing {
            Some(row) => row.id.clone(),
            None => {
                let wish = format!("zone_{}", slug(&name));
                if wish == "zone_" || wish == DEFAULT_ZONE_ID || rows.iter().any(|r| r.id == wish) {
                    format!("zone_{}", acom_core::id::new_suffix())
                } else {
                    wish
                }
            }
        };

        let labels: Vec<String> = wanted.iter().map(Region::label).collect();
        let surcharge_detail = if after.surcharge.value.is_zero() {
            format!("No surcharge for {}", labels.join(", "))
        } else {
            format!(
                "Every parcel leg to {} adds {}",
                labels.join(", "),
                money_text(after.surcharge)
            )
        };
        let checks = vec![
            Check {
                rule: "delivery.zone_overlap".into(),
                passed: true,
                detail: Some("No region is in two zones".into()),
                fix_tool: Some("list_shipping_zones".into()),
            },
            Check {
                rule: "delivery.remote_area_surcharge".into(),
                passed: true,
                detail: Some(surcharge_detail),
                fix_tool: None,
            },
        ];
        let change = match existing {
            Some(row) => Change {
                op: ChangeOp::Update,
                entity: "shipping_zone".into(),
                id: row.id.clone(),
                from: zone_wire(&base),
                to: zone_wire(&after),
                note: None,
            },
            None => Change {
                op: ChangeOp::Create,
                entity: "shipping_zone".into(),
                id: String::new(),
                from: Value::Null,
                to: zone_wire(&after),
                note: Some(format!(
                    "Rate {} until set_shipping_rate changes it",
                    money_text(after.shipping_rate)
                )),
            },
        };
        let basis = existing
            .map(|row| Basis {
                entity: "fulfillment.shipping_zone".into(),
                id: row.id.clone(),
                version: i64::from(row.version),
            })
            .into_iter()
            .collect();
        let approval = cx.policy.approval_for(Self::META.tool, &json!({}));
        Ok(Planned {
            plan: ZonePlan {
                zone_id,
                before: existing.map(|_| base),
                after,
            },
            changes: vec![change],
            checks,
            basis,
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: approval
                        .reason
                        .unwrap_or_else(|| "Changes a shipping zone".into()),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ZonePlan) -> Result<Committed, ToolError> {
        let done = Committed {
            id: plan.zone_id.clone(),
            produced: Map::from_iter([("zone_id".to_owned(), json!(plan.zone_id))]),
        };
        let now = repo::zone(tx, &plan.zone_id).await?;
        match (&plan.before, now) {
            (None, None) => repo::insert_zone(tx, &plan.zone_id, &plan.after).await?,
            (None, Some(row)) if row.values()? == plan.after => {}
            (None, Some(_)) => {
                return Err(ToolError::new(
                    ErrorCode::INVALID_STATE,
                    format!("Zone {} already exists.", plan.zone_id),
                )
                .next("list_shipping_zones"));
            }
            (Some(_), None) => return Err(zone_not_found(&plan.zone_id, "list_shipping_zones")),
            (Some(_), Some(_)) => {
                repo::update_zone(tx, &plan.zone_id, &plan.after).await?;
            }
        }
        Ok(done)
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ZonePlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row = repo::zone(&mut tx, &plan.zone_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mismatches = match row {
            None => vec![format!("zone {} is missing", plan.zone_id)],
            Some(r) if r.values()? == plan.after => Vec::new(),
            Some(r) => vec![format!("zone {} differs from the plan", r.id)],
        };
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(&self, tx: &mut TxCx, plan: &ZonePlan, _: &Committed) -> Result<(), ToolError> {
        match &plan.before {
            None => repo::delete_zone(tx, &plan.zone_id).await,
            Some(before) => repo::update_zone(tx, &plan.zone_id, before)
                .await
                .map(|_| ()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{input, offline};

    #[test]
    fn names_become_readable_ids() {
        assert_eq!(slug("Alaska and Hawaii"), "alaska_and_hawaii");
        assert_eq!(slug("  US / territories "), "us_territories");
    }

    #[tokio::test]
    async fn refuses_a_surcharge_in_another_currency() {
        let e = ManageShippingZone
            .preview(
                &offline("adm_sj"),
                input(json!({"name": "Alaska", "shipping_destination": [{"address_country": "US", "address_region": "AK"}],
                             "surcharge": {"value": 12.00, "currency": "EUR"}})),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
    }
}
