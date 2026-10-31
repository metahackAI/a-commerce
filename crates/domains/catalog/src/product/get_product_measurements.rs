//! `get_product_measurements` (docs/api/catalog.md#product).

use acom_contract::{
    ActionType, Audience, ErrorCode, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot,
    ToolError,
};
use acom_core::enums::MeasurementName;
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::model::{plain, size_key};
use crate::repo::{self, Measurement};
use crate::visible_group;
use crate::{SIZE_NOT_FOUND, not_found};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetProductMeasurementsInput {
    /// The product.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_wool_overcoat")]
    pub product_id: String,
    /// Only these sizes, values of the product's size axis. Default all.
    #[schemars(example = ["M", "L"])]
    pub sizes: Option<Vec<String>>,
}

/// Garment measurements per size, for fit questions.
#[derive(Debug)]
pub struct GetProductMeasurements;

impl Operation for GetProductMeasurements {
    const META: OpMeta = OpMeta {
        address: "catalog.product.get_measurements",
        tool: "get_product_measurements",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/products/{product_id}/measurements",
        summary: "Garment measurements per size in cm (chest, shoulder, sleeve, length, …), with the same names on every product so sizes can be compared with what the shopper already owns.",
        consumes: &[Slot::one("product_id")],
        produces: &[],
        checks: &[],
        errors: &[ErrorCode::NOT_FOUND, SIZE_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetProductMeasurementsInput;
    type Output = Value;
}

/// The size chart: measurement names in their usual order and one row per size.
fn measurement_chart(rows: &[Measurement], only: &[String]) -> Result<Value, ToolError> {
    let mut sizes: Vec<&str> = rows.iter().map(|r| r.size.as_str()).collect();
    sizes.sort_by_key(|s| size_key(Some(s)));
    sizes.dedup();
    for s in only {
        if !sizes.iter().any(|x| x.eq_ignore_ascii_case(s)) {
            return Err(
                ToolError::new(SIZE_NOT_FOUND, format!("The size chart has no size {s}."))
                    .detail(json!({"field": "sizes", "size": s, "allowed": sizes}))
                    .next("get_product"),
            );
        }
    }
    let names: Vec<&str> = MeasurementName::WORDS
        .iter()
        .copied()
        .filter(|n| rows.iter().any(|r| r.name == *n))
        .collect();
    let table: Vec<Value> = sizes
        .iter()
        .filter(|s| only.is_empty() || only.iter().any(|o| o.eq_ignore_ascii_case(s)))
        .map(|s| {
            let mut row = Map::new();
            row.insert("size".into(), json!(s));
            for n in &names {
                if let Some(m) = rows.iter().find(|r| r.size == *s && r.name == *n) {
                    row.insert((*n).into(), plain(m.value));
                }
            }
            Value::Object(row)
        })
        .collect();
    Ok(json!({
        "unit": rows.first().map_or("cm", |r| r.unit_text.as_str()),
        "measurements": names,
        "sizes": table,
        "how_measured": rows.iter().find_map(|r| r.measurement_technique.clone()),
    }))
}

impl ReadOp<Cx> for GetProductMeasurements {
    async fn read(&self, cx: &Cx, input: GetProductMeasurementsInput) -> Result<Value, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let group = visible_group(cx, &mut tx, &input.product_id).await?;
        let rows = match group {
            Some(_) => repo::measurements(&mut tx, &input.product_id).await?,
            None => Vec::new(),
        };
        tx.commit().await.map_err(ToolError::from)?;
        if group.is_none() {
            return Err(not_found("product", &input.product_id, "search_products"));
        }
        if rows.is_empty() {
            return Err(not_found(
                "measurement_chart",
                &input.product_id,
                "get_review_summary",
            ));
        }
        let mut out = json!({"product_id": input.product_id});
        let chart = measurement_chart(&rows, &input.sizes.unwrap_or_default())?;
        if let (Value::Object(o), Value::Object(c)) = (&mut out, chart) {
            o.extend(c);
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::ProductIdInput;
    use crate::product::get_delivery_policy::GetDeliveryPolicy;
    use crate::product::get_return_policy::GetReturnPolicy;
    use crate::testing;
    use rust_decimal::Decimal;
    use std::str::FromStr;

    fn m(size: &str, name: &str, value: &str) -> Measurement {
        Measurement {
            size: size.into(),
            name: name.into(),
            value: Decimal::from_str(value).unwrap_or_default(),
            unit_text: "cm".into(),
            measurement_technique: Some("Garment laid flat.".into()),
        }
    }

    /// F3: the wool overcoat M 56/46/64/98 and L 59/48/65.5/100.
    fn overcoat() -> Vec<Measurement> {
        vec![
            m("L", "length", "100.0"),
            m("M", "chest", "56.0"),
            m("L", "chest", "59.0"),
            m("M", "shoulder", "46.0"),
            m("L", "shoulder", "48.0"),
            m("M", "sleeve", "64.0"),
            m("L", "sleeve", "65.5"),
            m("M", "length", "98.0"),
        ]
    }

    #[test]
    fn size_chart_matches_the_fit_check() {
        let chart = measurement_chart(&overcoat(), &[]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            chart["measurements"],
            json!(["chest", "shoulder", "sleeve", "length"])
        );
        assert_eq!(
            chart["sizes"],
            json!([
                {"size": "M", "chest": 56, "shoulder": 46, "sleeve": 64, "length": 98},
                {"size": "L", "chest": 59, "shoulder": 48, "sleeve": 65.5, "length": 100}])
        );
        let only_l =
            measurement_chart(&overcoat(), &["l".into()]).unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(only_l["sizes"].as_array().map(Vec::len), Some(1));
        let e = measurement_chart(&overcoat(), &["XS".into()]).unwrap_err();
        assert_eq!(e.code, "size_not_found");
        assert_eq!(
            e.detail.map(|d| d["allowed"].clone()),
            Some(json!(["M", "L"]))
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn overcoat_chart_and_policies_read_back() {
        let cx = testing::shopper_db().await;
        let chart = GetProductMeasurements
            .read(
                &cx,
                GetProductMeasurementsInput {
                    product_id: "prd_wool_overcoat".into(),
                    sizes: Some(vec!["M".into(), "L".into()]),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(chart["sizes"][1]["sleeve"].to_string(), "65.5");
        let returns = GetReturnPolicy
            .read(
                &cx,
                ProductIdInput {
                    product_id: "prd_wool_overcoat".into(),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(returns["return_fees"], "free_return");
        assert_eq!(returns["acom:size_exchange_days"], json!(30));
        let delivery = GetDeliveryPolicy
            .read(
                &cx,
                ProductIdInput {
                    product_id: "prd_windbreaker".into(),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(delivery["shipping_fee_type"], "free_over_min_subtotal");
    }
}
