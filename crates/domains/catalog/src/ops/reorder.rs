//! `create_reorder` (docs/api/catalog.md#reorder): stock the store orders from its supplier, and
//! what `get_inventory` shows of it until it arrives.
//!
//! A reorder is an order the store places as the buyer. Each item is a variant and a quantity at
//! the variant's cost; the expected arrival is the store's date plus `reorder.lead_time_days`.
//! Ordered stock is not sellable, so nothing here touches `catalog.offer.inventory_level`.

use std::collections::BTreeMap;

use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::Money;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Days, Utc};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::product::visible_group;
use super::product_not_found;
use crate::model::{money_text, num};
use crate::rules::ReorderRules;
use crate::{PRODUCT_NOT_FOUND, db, repo};

/// Reorders not received yet: their units are on order.
const OPEN: [&str; 2] = ["processing", "in_transit"];

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReorderLineInput {
    /// A variant of the product (`get_inventory`).
    #[schemars(regex(pattern = r"^var_[a-z0-9_]+$"), example = &"var_linen_shirt_s_white")]
    pub variant_id: String,
    /// Units to order.
    #[schemars(range(min = 1), example = 70)]
    pub order_quantity: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateReorderInput {
    /// The product to reorder.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_linen_shirt")]
    pub product_id: String,
    /// How many of each variant; each variant once.
    #[schemars(length(min = 1, max = 50))]
    pub lines: Vec<ReorderLineInput>,
    /// A note for the supplier or why it is ordered.
    #[schemars(length(max = 500), example = &"Sells out in about 4 days at the current rate.")]
    pub description: Option<String>,
}

/// One item of a planned reorder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReorderLine {
    pub variant_id: String,
    pub size: Option<String>,
    pub color: Option<String>,
    pub order_quantity: i32,
    /// What one unit costs the store.
    pub unit_cost: Money,
}

/// What `create_reorder` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReorderPlan {
    pub product_id: String,
    pub name: String,
    pub lines: Vec<ReorderLine>,
    pub total: Money,
    pub expected_arrival_until: DateTime<Utc>,
    pub description: Option<String>,
}

impl ReorderPlan {
    fn units(&self) -> i64 {
        self.lines.iter().map(|l| i64::from(l.order_quantity)).sum()
    }
}

/// Order more stock of a product from the supplier.
#[derive(Debug)]
pub struct CreateReorder;

impl Operation for CreateReorder {
    const META: OpMeta = OpMeta {
        address: "catalog.reorder.create",
        tool: "create_reorder",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/reorders",
        summary: "Order more stock of a product from the supplier: units per variant at each variant's cost, arriving after the store's lead time. The stock is sellable once received; until then it shows as on order.",
        consumes: &[Slot::one("product_id"), Slot::many("variant_id")],
        produces: &[Slot::one("reorder_id")],
        checks: &["reorder.max_units_per_line", "reorder.variant_orderable"],
        errors: &[PRODUCT_NOT_FOUND, ErrorCode::INVALID_STATE],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreateReorderInput;
    type Output = Value;
}

fn line_error(message: String, variant_id: &str, reason: &str) -> ToolError {
    ToolError::invalid_input(message)
        .detail(json!({"field": "lines", "reason": reason, "variant_id": variant_id}))
        .next("get_inventory")
}

impl WriteOp<Cx> for CreateReorder {
    type Plan = ReorderPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreateReorderInput,
    ) -> Result<Planned<ReorderPlan>, ToolError> {
        let rules = ReorderRules::from_policy(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let group = visible_group(cx, &mut tx, &input.product_id)
            .await?
            .ok_or_else(|| product_not_found("product", &input.product_id, "search_products"))?;
        let variants = repo::variants(&mut tx, std::slice::from_ref(&group.id)).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let mut lines = Vec::new();
        let mut over = Vec::new();
        let mut not_orderable = Vec::new();
        for l in &input.lines {
            if lines
                .iter()
                .any(|x: &ReorderLine| x.variant_id == l.variant_id)
            {
                return Err(line_error(
                    format!("{} is in the reorder twice.", l.variant_id),
                    &l.variant_id,
                    "duplicate",
                ));
            }
            let v = variants
                .iter()
                .find(|v| v.id == l.variant_id)
                .ok_or_else(|| {
                    line_error(
                        format!("{} is not a variant of {}.", l.variant_id, group.name),
                        &l.variant_id,
                        "other_product",
                    )
                })?;
            let quantity = i32::try_from(l.order_quantity).map_err(|_| {
                line_error(
                    format!("{} units is too many.", l.order_quantity),
                    &l.variant_id,
                    "range",
                )
            })?;
            let unit_cost = v.cost().ok_or_else(|| {
                ToolError::new(
                    ErrorCode::INVALID_STATE,
                    format!(
                        "{} has no cost, so its reorder price is unknown; set it first.",
                        v.name
                    ),
                )
                .detail(json!({"entity": "variant", "id": v.id, "field": "acom:cost"}))
                .next("set_price")
            })?;
            if l.order_quantity > rules.max_units_per_line {
                over.push(format!(
                    "{} {}",
                    v.size.as_deref().unwrap_or(&v.name),
                    l.order_quantity
                ));
            }
            let availability = v.availability.as_deref().unwrap_or_default();
            if !rules
                .orderable_availability
                .iter()
                .any(|a| a == availability)
            {
                not_orderable.push(format!("{} is {}", v.name, availability.replace('_', " ")));
            }
            lines.push(ReorderLine {
                variant_id: v.id.clone(),
                size: v.size.clone(),
                color: v.color.clone(),
                order_quantity: quantity,
                unit_cost,
            });
        }
        let currency = lines[0].unit_cost.currency;
        if lines.iter().any(|l| l.unit_cost.currency != currency) {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                "The variants' costs are in different currencies; one reorder takes one.",
            )
            .detail(json!({"entity": "product", "id": group.id, "field": "acom:cost"}))
            .next("get_inventory"));
        }
        let total: Decimal = lines
            .iter()
            .map(|l| l.unit_cost.value * Decimal::from(l.order_quantity))
            .sum();
        let total = Money::new(total, currency);

        let today = cx.clock.now().with_timezone(&cx.time.zone).date_naive();
        let arrives = today
            .checked_add_days(Days::new(u64::from(rules.lead_time_days)))
            .ok_or_else(|| ToolError::new(ErrorCode::INTERNAL, "arrival past the calendar"))?;
        let expected_arrival_until = cx
            .time
            .parse(&format!("{arrives}T23:59"))
            .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))?;

        let plan = ReorderPlan {
            product_id: group.id.clone(),
            name: group.name.clone(),
            lines,
            total,
            expected_arrival_until,
            description: input
                .description
                .map(|d| d.trim().to_owned())
                .filter(|d| !d.is_empty()),
        };
        let checks = vec![
            Check {
                rule: "reorder.max_units_per_line".into(),
                passed: over.is_empty(),
                detail: Some(if over.is_empty() {
                    format!("Every line is at most {} units", rules.max_units_per_line)
                } else {
                    format!(
                        "Over {} units: {}",
                        rules.max_units_per_line,
                        over.join(", ")
                    )
                }),
                fix_tool: (!over.is_empty()).then(|| "create_reorder".into()),
            },
            Check {
                rule: "reorder.variant_orderable".into(),
                passed: not_orderable.is_empty(),
                detail: Some(if not_orderable.is_empty() {
                    format!("{} sizes can be ordered", plan.lines.len())
                } else {
                    not_orderable.join("; ")
                }),
                fix_tool: None,
            },
        ];
        let mut changes = vec![Change {
            op: ChangeOp::Create,
            entity: "reorder".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({"product_id": plan.product_id, "name": plan.name,
                       "total_units": plan.units(), "total_payment_due": plan.total,
                       "expected_arrival_until": cx.time.format(plan.expected_arrival_until)}),
            note: plan.description.clone(),
        }];
        changes.extend(plan.lines.iter().map(|l| Change {
            op: ChangeOp::Create,
            entity: "reorder_item".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({"variant_id": l.variant_id, "size": l.size, "color": l.color,
                       "order_quantity": l.order_quantity, "price": num(l.unit_cost.value),
                       "price_currency": l.unit_cost.currency.as_str()}),
            note: None,
        }));
        Ok(Planned {
            changes,
            checks,
            basis: vec![Basis {
                entity: "catalog.product_group".into(),
                id: group.id.clone(),
                version: i64::from(group.version),
            }],
            approval: crate::policy_approval(
                cx,
                Self::META.tool,
                &json!({"reorder_cost": plan.total}),
            ),
            plan,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ReorderPlan) -> Result<Committed, ToolError> {
        let id = format!("ro_{}", acom_core::id::new_suffix());
        sqlx::query(
            "INSERT INTO catalog.reorder (tenant_id, id, product_group_id, order_date,
               expected_arrival_until, total_payment_due_value, total_payment_due_currency,
               description)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(tx.tenant.as_str())
        .bind(&id)
        .bind(&plan.product_id)
        .bind(tx.clock.now())
        .bind(plan.expected_arrival_until)
        .bind(plan.total.value)
        .bind(plan.total.currency.as_str())
        .bind(plan.description.as_deref())
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        for (i, l) in plan.lines.iter().enumerate() {
            sqlx::query(
                "INSERT INTO catalog.reorder_item (tenant_id, reorder_id, position, product_id,
                   order_quantity, price, price_currency)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(tx.tenant.as_str())
            .bind(&id)
            .bind(i32::try_from(i + 1).unwrap_or(i32::MAX))
            .bind(&l.variant_id)
            .bind(l.order_quantity)
            .bind(l.unit_cost.value)
            .bind(l.unit_cost.currency.as_str())
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
        }
        let mut produced = Map::new();
        produced.insert("reorder_id".into(), json!(id));
        Ok(Committed { id, produced })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ReorderPlan,
        done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let row: Option<(String, Decimal, i64)> = sqlx::query_as(
            "SELECT r.order_status, r.total_payment_due_value,
                    coalesce((SELECT sum(i.order_quantity) FROM catalog.reorder_item i
                              WHERE i.tenant_id = r.tenant_id AND i.reorder_id = r.id), 0)::bigint
             FROM catalog.reorder r WHERE r.tenant_id = $1 AND r.id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&done.id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        match row {
            None => mismatches.push(format!("{} was not recorded", done.id)),
            Some((status, total, units)) => {
                if status != "processing" {
                    mismatches.push(format!("{} is {status}, not processing", done.id));
                }
                if total != plan.total.value || units != plan.units() {
                    mismatches.push(format!(
                        "{} is {units} units for {total} instead of {} units for {}",
                        done.id,
                        plan.units(),
                        money_text(&plan.total)
                    ));
                }
            }
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        _plan: &ReorderPlan,
        done: &Committed,
    ) -> Result<(), ToolError> {
        let n = sqlx::query(
            "UPDATE catalog.reorder SET order_status = 'cancelled', updated_at = $3,
               version = version + 1
             WHERE tenant_id = $1 AND id = $2 AND order_status = 'processing'",
        )
        .bind(tx.tenant.as_str())
        .bind(&done.id)
        .bind(tx.clock.now())
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?
        .rows_affected();
        if n == 1 {
            return Ok(());
        }
        Err(ToolError::new(
            ErrorCode::INVALID_STATE,
            format!(
                "{} has left the supplier or arrived; it can no longer be called off.",
                done.id
            ),
        )
        .detail(json!({"entity": "reorder", "id": done.id}))
        .next("get_inventory"))
    }
}

/// Reorders of a product that have not arrived, soonest first, and the units each variant has
/// on order in them.
pub(crate) struct OnOrder {
    pub reorders: Vec<Value>,
    pub by_variant: BTreeMap<String, i64>,
}

pub(crate) async fn on_order(
    cx: &Cx,
    tx: &mut TxCx,
    product_id: &str,
) -> Result<OnOrder, ToolError> {
    let open: Vec<String> = OPEN.iter().map(|s| (*s).to_owned()).collect();
    let rows: Vec<(String, String, DateTime<Utc>, String, i32)> = sqlx::query_as(
        "SELECT r.id, r.order_status, r.expected_arrival_until, i.product_id, i.order_quantity
         FROM catalog.reorder r
         JOIN catalog.reorder_item i ON i.tenant_id = r.tenant_id AND i.reorder_id = r.id
         WHERE r.tenant_id = $1 AND r.product_group_id = $2 AND r.order_status = ANY($3)
         ORDER BY r.expected_arrival_until, r.id, i.position",
    )
    .bind(tx.tenant.as_str())
    .bind(product_id)
    .bind(&open)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    let mut reorders: Vec<Value> = Vec::new();
    let mut by_variant: BTreeMap<String, i64> = BTreeMap::new();
    for (id, status, arrives, variant, quantity) in rows {
        *by_variant.entry(variant).or_default() += i64::from(quantity);
        match reorders.last_mut() {
            Some(r) if r["id"] == id.as_str() => {
                let units = r["total_units"].as_i64().unwrap_or(0) + i64::from(quantity);
                r["total_units"] = json!(units);
            }
            _ => reorders.push(json!({"id": id, "order_status": status,
                                     "total_units": quantity,
                                     "expected_arrival_until": cx.time.format(arrives)})),
        }
    }
    Ok(OnOrder {
        reorders,
        by_variant,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::testing;
    use acom_contract::ApprovalNeed;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn input_schema_is_closed() {
        let s = closed_schema_for::<CreateReorderInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(s["required"], json!(["product_id", "lines"]));
    }

    #[test]
    fn reads_the_reorder_policy() {
        let r = ReorderRules::from_policy(&crate::testing::policies()).unwrap();
        assert_eq!((r.lead_time_days, r.max_units_per_line), (7, 500));
        assert!(!r.orderable_availability.iter().any(|a| a == "discontinued"));
    }

    fn linen(lines: Value) -> CreateReorderInput {
        serde_json::from_value(json!({"product_id": "prd_linen_shirt", "lines": lines,
            "description": "Sells out in about 4 days at the current rate."}))
        .unwrap()
    }

    /// Integration: the Linen shirt reorder of task #1041 (`DATABASE_URL`, demo seed), run and
    /// called off inside a transaction that is rolled back.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn reorders_the_linen_shirt_and_calls_it_off() {
        let cx = testing::operator_db().await;
        let planned = CreateReorder
            .preview(
                &cx,
                linen(json!([
                    {"variant_id": "var_linen_shirt_s_white", "order_quantity": 70},
                    {"variant_id": "var_linen_shirt_m_white", "order_quantity": 90},
                    {"variant_id": "var_linen_shirt_l_white", "order_quantity": 80}])),
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let head = &planned.changes[0].to;
        assert_eq!(head["total_units"], 240);
        assert_eq!(head["total_payment_due"]["value"].to_string(), "6240.00");
        assert_eq!(head["expected_arrival_until"], "2026-11-13T23:59:00-05:00");
        assert_eq!(planned.changes.len(), 4);
        assert_eq!(planned.changes[2].to["size"], "M");
        assert!(
            planned.checks.iter().all(|c| c.passed),
            "{:?}",
            planned.checks
        );
        match &planned.approval {
            ApprovalNeed::Needed { reason } => assert_eq!(reason, "Reorder cost over $1000.00"),
            other => panic!("needs approval: {other:?}"),
        }

        let small = CreateReorder
            .preview(
                &cx,
                linen(json!([{"variant_id": "var_linen_shirt_s_white", "order_quantity": 10}])),
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(small.approval, ApprovalNeed::NotNeeded));
        let too_many = CreateReorder
            .preview(
                &cx,
                linen(json!([{"variant_id": "var_linen_shirt_s_white", "order_quantity": 900}])),
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(!too_many.checks[0].passed);
        let other = CreateReorder
            .preview(
                &cx,
                linen(json!([{"variant_id": "var_windbreaker_m_slate", "order_quantity": 5}])),
            )
            .await
            .unwrap_err();
        assert_eq!(
            other.detail.as_deref().map(|d| d["reason"].clone()),
            Some(json!("other_product"))
        );

        let mut tx = cx.begin().await.unwrap_or_else(|e| panic!("{e}"));
        let done = CreateReorder
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let ordered = on_order(&cx, &mut tx, "prd_linen_shirt")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(ordered.by_variant.get("var_linen_shirt_m_white"), Some(&90));
        assert_eq!(ordered.reorders.len(), 1);
        assert_eq!(ordered.reorders[0]["total_units"], 240);
        CreateReorder
            .undo(&mut tx, &planned.plan, &done)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        let after = on_order(&cx, &mut tx, "prd_linen_shirt")
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(
            after.reorders.is_empty(),
            "a cancelled reorder is not on order"
        );
        let again = CreateReorder.undo(&mut tx, &planned.plan, &done).await;
        assert!(
            again.is_err(),
            "a cancelled reorder cannot be called off twice"
        );
        drop(tx);
    }
}
