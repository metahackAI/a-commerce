//! Watcher `price_below_cost` (`policies/platform.yml watchers`): products whose list price is
//! under their cost. Offers and costs are this domain's data. The finding carries a fix
//! (`fix: raise_to_price_floor`): one `set_price` per variant under cost, to the price floor of
//! `catalog.yml price_floor`. The job runner drafts that change set and submits it for approval;
//! only a standing approval in the store's policy runs it without a person.

use std::collections::BTreeMap;

use acom_contract::{
    EntityRef, ErrorCode, FixCall, FixDraft, Job, JobMeta, JobOutcome, TaskDraft, ToolError,
};
use acom_store::{Cx, TxCx};
use rust_decimal::Decimal;
use serde_json::{Value, json};

use crate::db;
use crate::model::{money_of, money_text, num};
use crate::rules::PriceFloorRule;

/// One variant whose list price is under its cost.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub(crate) struct BelowCost {
    pub product_id: String,
    pub name: String,
    pub variant_id: String,
    pub price: Decimal,
    pub price_currency: String,
    pub cost_value: Decimal,
}

#[derive(Debug)]
pub struct PriceBelowCost;

impl Job<Cx> for PriceBelowCost {
    const META: JobMeta = JobMeta {
        kind: "watcher.price_below_cost",
        summary: "Products whose list price is under their cost, with a fix up to the price floor.",
        watcher: Some("price_below_cost"),
    };

    async fn run(&self, tx: &mut TxCx, _: &Value) -> Result<JobOutcome, ToolError> {
        let w = tx
            .policy
            .watchers()
            .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))?
            .price_below_cost;
        if w.compare != "list_price_below_cost" {
            return Err(ToolError::new(
                ErrorCode::INTERNAL,
                format!(
                    "watchers.price_below_cost.compare `{}` is not something this watcher compares.",
                    w.compare
                ),
            ));
        }
        let floor = PriceFloorRule::from_policy(&tx.policy)?;
        let rows = sqlx::query_as::<_, BelowCost>(
            "SELECT g.id AS product_id, g.name, v.id AS variant_id, o.price,
                    o.price_currency::text AS price_currency, o.cost_value
             FROM catalog.offer o
             JOIN catalog.product v ON v.tenant_id = o.tenant_id AND v.id = o.product_id
             JOIN catalog.product_group g ON g.tenant_id = v.tenant_id AND g.id = v.product_group_id
             WHERE o.tenant_id = $1 AND o.cost_value IS NOT NULL
               AND o.cost_currency = o.price_currency AND o.price < o.cost_value
             ORDER BY g.id, v.id",
        )
        .bind(tx.tenant.as_str())
        .fetch_all(&mut *tx.tx)
        .await
        .map_err(db)?;
        let fix = w.fix == "raise_to_price_floor";
        Ok(JobOutcome::Watched(
            finding(&rows, &floor, fix).into_iter().collect(),
        ))
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// What the watcher reports for the variants under cost: `None` when there are none. With
/// `fix`, the change set that raises each variant to its price floor.
pub(crate) fn finding(rows: &[BelowCost], floor: &PriceFloorRule, fix: bool) -> Option<TaskDraft> {
    // Per product, the variant priced lowest stands for it in the observation.
    let mut products: BTreeMap<&str, &BelowCost> = BTreeMap::new();
    for r in rows {
        let e = products.entry(r.product_id.as_str()).or_insert(r);
        if r.price < e.price {
            *e = r;
        }
    }
    if products.is_empty() {
        return None;
    }
    let n = products.len();
    let names: Vec<&str> = products.values().map(|r| r.name.as_str()).collect();
    let ids: Vec<&str> = products.keys().copied().collect();
    let observed: Vec<Value> = products
        .values()
        .map(|r| {
            json!({"product_id": r.product_id, "list_price": num(r.price), "cost": num(r.cost_value)})
        })
        .collect();
    let markup = floor.markup_text();
    let calls: Vec<FixCall> = rows
        .iter()
        .filter_map(|r| {
            let cost = money_of(r.cost_value, &r.price_currency)?;
            let to = floor.floor(cost);
            Some(FixCall {
                tool: "set_price",
                input: json!({"variant_id": r.variant_id, "price": num(to.value),
                              "price_currency": to.currency.as_str()}),
            })
        })
        .collect();
    let lowest = products
        .values()
        .filter_map(|r| money_of(r.price, &r.price_currency).map(|m| (r, m)))
        .map(|(r, m)| format!("{} {}", r.name, money_text(&m)))
        .collect::<Vec<_>>()
        .join(", ");
    let fix = (fix && !calls.is_empty()).then(|| FixDraft {
        title: format!("Raise {} to the price floor", plural(n, "product", "products")),
        request_text: format!(
            "{} priced below cost. Raise each list price to the price floor ({markup}).",
            if n == 1 {
                "1 product is".to_owned()
            } else {
                format!("{n} products are")
            }
        ),
        interpretation: vec![
            json!({"slot": "products", "text": format!("{} under cost", plural(n, "product", "products")),
                   "value": {"product_ids": ids}}),
            json!({"slot": "benefit", "text": format!("List price to {markup}"),
                   "value": {"rule": "price_floor"}}),
        ],
        calls,
        done_text: "Raised to the price floor".into(),
    });
    Some(TaskDraft {
        cause: None,
        title: if n == 1 {
            "1 product is priced below cost".into()
        } else {
            format!("{n} products are priced below cost")
        },
        summary: format!("Under cost: {lowest}"),
        tag: "pricing".into(),
        request_text: format!(
            "{} below cost: {}. Raise them to the price floor ({markup}).",
            plural(n, "product is", "products are"),
            names.join(", ")
        ),
        observation: json!({
            "threshold": {"policy_key": "watchers.price_below_cost.compare",
                          "value": "list_price_below_cost"},
            "observed": {"products": observed},
        }),
        related: ids
            .iter()
            .map(|id| EntityRef::new("product", *id))
            .collect(),
        fix,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(product: &str, name: &str, variant: &str, price: i64, cost: i64) -> BelowCost {
        BelowCost {
            product_id: product.into(),
            name: name.into(),
            variant_id: variant.into(),
            price: Decimal::new(price, 2),
            price_currency: "USD".into(),
            cost_value: Decimal::new(cost, 2),
        }
    }

    fn floor() -> PriceFloorRule {
        PriceFloorRule {
            multiplier: Decimal::new(115, 2),
            strict: false,
        }
    }

    #[test]
    fn raises_every_variant_under_cost_to_its_floor() {
        let rows = [
            row(
                "prd_breton_tee",
                "Breton tee",
                "var_breton_tee_s",
                1900,
                2000,
            ),
            row(
                "prd_breton_tee",
                "Breton tee",
                "var_breton_tee_m",
                1900,
                2000,
            ),
            row(
                "prd_canvas_tote",
                "Canvas tote",
                "var_canvas_tote",
                2400,
                2500,
            ),
        ];
        let d = finding(&rows, &floor(), true).unwrap();
        assert_eq!(d.title, "2 products are priced below cost");
        assert_eq!(
            d.observation["observed"].to_string(),
            r#"{"products":[{"product_id":"prd_breton_tee","list_price":19.00,"cost":20.00},{"product_id":"prd_canvas_tote","list_price":24.00,"cost":25.00}]}"#
        );
        let fix = d.fix.unwrap();
        assert_eq!(fix.calls.len(), 3);
        assert_eq!(
            fix.calls[2].input.to_string(),
            r#"{"variant_id":"var_canvas_tote","price":28.75,"price_currency":"USD"}"#
        );
        assert_eq!(
            fix.request_text,
            "2 products are priced below cost. Raise each list price to the price floor (cost + 15%)."
        );
        assert_eq!(d.related.len(), 2);
    }

    #[test]
    fn nothing_under_cost_is_no_finding_and_no_fix_when_off() {
        assert!(finding(&[], &floor(), true).is_none());
        let rows = [row("prd_socks", "Hiking socks", "var_socks", 1200, 1300)];
        let d = finding(&rows, &floor(), false).unwrap();
        assert_eq!(d.title, "1 product is priced below cost");
        assert!(d.fix.is_none());
    }
}
