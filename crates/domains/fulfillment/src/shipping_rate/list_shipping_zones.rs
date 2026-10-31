//! `list_shipping_zones`: the delivery zones with their rates, free-shipping minimum,
//! surcharge and transit days.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError, fields,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{zone_json, zones};
use crate::errors::INVALID_FILTER;
use crate::rules;

/// The fields a zone has, for `fields`.
const FIELDS: &[&str] = &[
    "id",
    "name",
    "zone_type",
    "shipping_destination",
    "fee_type",
    "shipping_rate",
    "free_over",
    "surcharge",
    "transit_business_days",
    "note",
];

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListShippingZonesInput {
    /// Top-level fields to return for each zone, for example only the name and the rate.
    #[schemars(example = ["name", "shipping_rate", "surcharge"])]
    pub fields: Option<Vec<String>>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListShippingZones;

impl Operation for ListShippingZones {
    const META: OpMeta = OpMeta {
        address: "fulfillment.shipping_rate.list_zones",
        tool: "list_shipping_zones",
        group: Group::Transact,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/fulfillment/shipping-zones",
        summary: "The delivery zones and what shipping costs in each: rate, free-shipping minimum, remote-area surcharge and transit days.",
        consumes: &[],
        produces: &[Slot::many("zone_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListShippingZonesInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListShippingZones {
    async fn read(&self, cx: &Cx, input: ListShippingZonesInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let fields = input.fields.unwrap_or_default();
        if let Some(bad) = fields.iter().find(|f| !FIELDS.contains(&f.as_str())) {
            return Err(
                ToolError::new(INVALID_FILTER, format!("A zone has no field `{bad}`."))
                    .detail(json!({"field": "fields", "reason": "enum", "allowed": FIELDS}))
                    .next("list_shipping_zones"),
            );
        }
        let rules = rules(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let all = zones(&mut tx, &rules).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let total = all.len() as i64;
        let items: Vec<Value> = all
            .iter()
            .skip(usize::try_from(page.offset).unwrap_or(0))
            .take(usize::try_from(page.limit).unwrap_or(0))
            .map(zone_json)
            .collect();
        let items = fields::keep_each(items, &fields)?;
        let mut out = json!({"items": items, "total": total});
        if let Some(next) = page.next_cursor(items.len(), total) {
            out["next_cursor"] = json!(next);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests::{input, offline};

    #[tokio::test]
    async fn refuses_a_field_zones_do_not_have() {
        let e = ListShippingZones
            .read(&offline("adm_sj"), input(json!({"fields": ["price"]})))
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_filter");
        assert_eq!(e.next_tool.as_deref(), Some("list_shipping_zones"));
    }
}
