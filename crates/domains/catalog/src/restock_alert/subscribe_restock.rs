//! `subscribe_restock` (docs/api/catalog.md#restock_alert): a signed-in shopper asks to hear when
//! a sold-out variant is back. The variant is found by size and color, or given by id. The alert
//! is transactional (not marketing) and lapses after `restock_alert.expires_after_days`.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::enums::MessageChannel;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Days, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::model::Variant;
use crate::rules::RestockRules;
use crate::{
    ALREADY_IN_STOCK, ALREADY_SUBSCRIBED, PRODUCT_NOT_FOUND, db, product_not_found, repo,
    visible_group,
};

/// How the variant is named.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RestockMode {
    /// By `size` and `color` (either can be left out when the product has one value).
    ProductOptions,
    /// By `variant_id`.
    Variant,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SubscribeRestockInput {
    /// The product (`search_products`).
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_chore_coat")]
    pub product_id: String,
    /// product_options: find the variant by `size` and `color`; variant: give `variant_id`.
    #[schemars(example = &"product_options")]
    pub mode: RestockMode,
    /// Size wanted (mode product_options).
    #[schemars(length(min = 1, max = 20), example = &"M")]
    pub size: Option<String>,
    /// Color wanted (mode product_options). Leave out for a one-color product.
    #[schemars(length(min = 1, max = 60), example = &"Olive")]
    pub color: Option<String>,
    /// The variant (mode variant), from `get_product`.
    #[schemars(regex(pattern = r"^var_[a-z0-9_]+$"), example = &"var_chore_coat_m_olive")]
    pub variant_id: Option<String>,
    /// How to tell the shopper: email, sms, push (app notification) or in_app.
    #[schemars(example = &"email")]
    pub channel: MessageChannel,
}

/// What `subscribe_restock` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubscribePlan {
    pub restock_alert_id: String,
    pub customer_id: String,
    pub product_id: String,
    pub variant_id: String,
    pub channel: String,
    pub start_time: DateTime<Utc>,
    pub expires: DateTime<Utc>,
    /// A channel the store does not send restock alerts on.
    pub channel_refused: bool,
}

/// A shopper asks to hear when a variant is back.
#[derive(Debug)]
pub struct SubscribeRestock;

impl Operation for SubscribeRestock {
    const META: OpMeta = OpMeta {
        address: "catalog.restock_alert.subscribe",
        tool: "subscribe_restock",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/products/{product_id}/restock-alerts",
        summary: "The signed-in shopper asks to be told when a sold-out size or color is back in stock, by email, text, app notification or in the app. A transactional alert, not marketing.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::one("restock_alert_id")],
        checks: &["restock_alert.channels", "inventory.auto_restore"],
        errors: &[
            ALREADY_IN_STOCK,
            ALREADY_SUBSCRIBED,
            PRODUCT_NOT_FOUND,
            ErrorCode::INVALID_STATE,
        ],
        audience: Audience::Shopper,
        action_type: ActionType("SubscribeAction"),
        person_scope: "",
    };
    type Input = SubscribeRestockInput;
    type Output = Value;
}

fn same(a: Option<&str>, b: Option<&str>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.trim().eq_ignore_ascii_case(b.trim()),
        (None, _) => true,
        (Some(_), None) => false,
    }
}

/// The one variant the shopper means, or an `invalid_input` that lists what there is.
pub(crate) fn pick_variant<'a>(
    variants: &'a [Variant],
    mode: RestockMode,
    size: Option<&str>,
    color: Option<&str>,
    variant_id: Option<&str>,
) -> Result<&'a Variant, ToolError> {
    let offered: Vec<&Variant> = variants.iter().filter(|v| v.offer_id.is_some()).collect();
    let choices = || -> Value {
        json!(
            offered
                .iter()
                .map(|v| json!({"variant_id": v.id, "size": v.size, "color": v.color}))
                .collect::<Vec<_>>()
        )
    };
    match mode {
        RestockMode::Variant => {
            let Some(id) = variant_id else {
                return Err(
                    ToolError::invalid_input("Mode `variant` needs `variant_id`.")
                        .detail(json!({"field": "variant_id", "reason": "missing"})),
                );
            };
            offered.iter().copied().find(|v| v.id == id).ok_or_else(|| {
                ToolError::invalid_input(format!("{id} is not a variant of this product."))
                    .detail(json!({"field": "variant_id", "reason": "other_product",
                                   "allowed": choices()}))
                    .next("get_product")
            })
        }
        RestockMode::ProductOptions => {
            let found: Vec<&Variant> = offered
                .iter()
                .copied()
                .filter(|v| same(size, v.size.as_deref()) && same(color, v.color.as_deref()))
                .collect();
            match found.as_slice() {
                [one] => Ok(one),
                [] => Err(ToolError::invalid_input(
                    "No variant of this product has that size and color.",
                )
                .detail(json!({"field": "size", "reason": "enum", "allowed": choices()}))
                .next("get_product")),
                _ => Err(ToolError::invalid_input(
                    "More than one variant matches; give both `size` and `color`.",
                )
                .detail(json!({"field": "color", "reason": "missing", "allowed": choices()}))),
            }
        }
    }
}

impl WriteOp<Cx> for SubscribeRestock {
    type Plan = SubscribePlan;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SubscribeRestockInput,
    ) -> Result<Planned<SubscribePlan>, ToolError> {
        let customer = crate::signed_in_shopper(cx, "get restock alerts")?;
        let rules = RestockRules::from_policy(&cx.policy)?;
        let channel = input.channel.as_str().to_owned();
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let Some(group) = visible_group(cx, &mut tx, &input.product_id).await? else {
            return Err(product_not_found(
                "product",
                &input.product_id,
                "search_products",
            ));
        };
        let variants = repo::variants(&mut tx, std::slice::from_ref(&group.id)).await?;
        let v = pick_variant(
            &variants,
            input.mode,
            input.size.as_deref(),
            input.color.as_deref(),
            input.variant_id.as_deref(),
        )?
        .clone();
        let existing = super::open_alert(&mut tx, &customer, &v.id, &channel, now).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let availability = v.availability.clone().unwrap_or_default();
        if availability == "in_stock" && v.stock() > 0 {
            return Err(ToolError::new(
                ALREADY_IN_STOCK,
                format!("{} is in stock now ({} left).", v.name, v.stock()),
            )
            .detail(json!({"variant_id": v.id, "inventory_level": v.stock()}))
            .next("add_to_cart"));
        }
        if !rules.subscribable_availability.contains(&availability) {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "{} is {}; it is not coming back.",
                    v.name,
                    availability.replace('_', " ")
                ),
            )
            .detail(json!({"entity": "variant", "from": availability}))
            .next("search_products"));
        }
        if let Some(id) = existing {
            return Err(ToolError::new(
                ALREADY_SUBSCRIBED,
                format!(
                    "You already asked to hear about {} by {channel} ({id}).",
                    v.name
                ),
            )
            .detail(json!({"entity": "restock_alert", "existing_id": id}))
            .next("list_restock_subscriptions"));
        }
        let channel_ok = rules.channels.contains(&channel);
        let expires = now
            .checked_add_days(Days::new(u64::from(rules.expires_after_days)))
            .unwrap_or(now);
        let checks = vec![
            Check {
                rule: "restock_alert.channels".into(),
                passed: channel_ok,
                detail: Some(if channel_ok {
                    format!("Sent by {channel}, as a transactional message")
                } else {
                    format!(
                        "This store sends restock alerts by {} only",
                        rules.channels.join(", ")
                    )
                }),
                fix_tool: (!channel_ok).then(|| "subscribe_restock".into()),
            },
            Check {
                rule: "inventory.auto_restore".into(),
                passed: rules.notify,
                detail: Some(if rules.notify {
                    "Sent when the size comes back in stock".into()
                } else {
                    "This store does not send restock alerts".into()
                }),
                fix_tool: None,
            },
        ];
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "restock_alert".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"product_id": group.id, "variant_id": v.id, "size": v.size,
                           "color": v.color, "channel": channel, "availability": availability,
                           "expires": cx.time.format(expires)}),
                note: Some(format!(
                    "Lapses after {} days if it is not back by then",
                    rules.expires_after_days
                )),
            }],
            checks,
            basis: vec![Basis {
                entity: "catalog.product_group".into(),
                id: group.id.clone(),
                version: i64::from(group.version),
            }],
            approval: ApprovalNeed::NotNeeded,
            plan: SubscribePlan {
                restock_alert_id: format!("rsa_{}", acom_core::id::new_suffix()),
                customer_id: customer,
                product_id: group.id,
                variant_id: v.id,
                channel,
                start_time: now,
                expires,
                channel_refused: !channel_ok,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &SubscribePlan) -> Result<Committed, ToolError> {
        if plan.channel_refused {
            return Err(ToolError::new(
                ErrorCode::POLICY_VIOLATION,
                format!(
                    "This store does not send restock alerts by {}.",
                    plan.channel
                ),
            )
            .detail(json!({"rule": "restock_alert.channels"}))
            .next("subscribe_restock"));
        }
        if let Some(id) = super::open_alert(
            tx,
            &plan.customer_id,
            &plan.variant_id,
            &plan.channel,
            plan.start_time,
        )
        .await?
            && id != plan.restock_alert_id
        {
            return Err(ToolError::new(
                ALREADY_SUBSCRIBED,
                format!(
                    "You already asked to hear about this by {} ({id}).",
                    plan.channel
                ),
            )
            .detail(json!({"entity": "restock_alert", "existing_id": id}))
            .next("list_restock_subscriptions"));
        }
        sqlx::query(
            "INSERT INTO catalog.restock_alert (tenant_id, id, customer_id, product_id,
               message_channel, start_time, expires)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (tenant_id, id) DO NOTHING",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.restock_alert_id)
        .bind(&plan.customer_id)
        .bind(&plan.variant_id)
        .bind(&plan.channel)
        .bind(plan.start_time)
        .bind(plan.expires)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        let mut produced = Map::new();
        produced.insert("restock_alert_id".into(), json!(plan.restock_alert_id));
        produced.insert("variant_id".into(), json!(plan.variant_id));
        Ok(Committed {
            id: plan.restock_alert_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &SubscribePlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let open = super::open_alert(
            &mut tx,
            &plan.customer_id,
            &plan.variant_id,
            &plan.channel,
            plan.start_time,
        )
        .await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = open.as_deref() == Some(plan.restock_alert_id.as_str());
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} is not waiting", plan.restock_alert_id)]
            },
        })
    }

    async fn undo(
        &self,
        _tx: &mut TxCx,
        plan: &SubscribePlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        Err(ToolError::new(
            ErrorCode::UNDO_WINDOW_CLOSED,
            format!(
                "{} is the shopper's own request; it lapses by itself.",
                plan.restock_alert_id
            ),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    fn v(id: &str, size: &str, color: &str) -> Variant {
        Variant {
            id: id.into(),
            product_group_id: "prd_chore_coat".into(),
            sku: id.into(),
            name: format!("Cotton chore coat, {color}, {size}"),
            color: Some(color.into()),
            size: Some(size.into()),
            sale_type: "always".into(),
            is_managed: true,
            offer_id: Some(format!("off_{id}")),
            price: None,
            price_currency: None,
            availability: Some("out_of_stock".into()),
            inventory_level: Some(0),
            safety_stock: None,
            cost_value: None,
            cost_currency: None,
        }
    }

    #[test]
    fn finds_the_variant_by_size_and_color_or_by_id() {
        let all = [
            v("var_m_olive", "M", "Olive"),
            v("var_l_olive", "L", "Olive"),
        ];
        let m = pick_variant(&all, RestockMode::ProductOptions, Some("m"), None, None)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(m.id, "var_m_olive");
        let both = pick_variant(&all, RestockMode::ProductOptions, None, Some("Olive"), None);
        assert_eq!(both.unwrap_err().code, "invalid_input");
        let by_id = pick_variant(&all, RestockMode::Variant, None, None, Some("var_l_olive"))
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(by_id.size.as_deref(), Some("L"));
        let e = pick_variant(&all, RestockMode::Variant, None, None, Some("var_x")).unwrap_err();
        assert_eq!(
            e.detail.map(|d| d["reason"].clone()),
            Some(json!("other_product"))
        );
        let schema = acom_contract::schema::closed_schema_for::<SubscribeRestockInput>();
        assert!(acom_contract::schema::violations(&schema).is_empty());
    }

    #[tokio::test]
    async fn guests_must_sign_in() {
        let e = SubscribeRestock
            .preview(
                &testing::guest(),
                SubscribeRestockInput {
                    product_id: "prd_chore_coat".into(),
                    mode: RestockMode::ProductOptions,
                    size: Some("M".into()),
                    color: None,
                    variant_id: None,
                    channel: MessageChannel::Email,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn jamie_waits_for_the_denim_jacket_but_not_twice_or_for_stock() {
        let cx = testing::shopper_db().await;
        let input = |product: &str, size: &str, channel| SubscribeRestockInput {
            product_id: product.into(),
            mode: RestockMode::ProductOptions,
            size: Some(size.into()),
            color: None,
            variant_id: None,
            channel,
        };
        let planned = SubscribeRestock
            .preview(&cx, input("prd_denim_jacket", "L", MessageChannel::Push))
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(
            planned.checks.iter().all(|c| c.passed),
            "{:?}",
            planned.checks
        );
        assert_eq!(planned.changes[0].to["availability"], "out_of_stock");
        // Jamie already waits for the chore coat M by email (rsa_204).
        let e = SubscribeRestock
            .preview(&cx, input("prd_chore_coat", "M", MessageChannel::Email))
            .await
            .unwrap_err();
        assert_eq!(e.code, "already_subscribed");
        // The chore coat S is in stock.
        let e = SubscribeRestock
            .preview(&cx, input("prd_chore_coat", "S", MessageChannel::Email))
            .await
            .unwrap_err();
        assert_eq!(e.code, "already_in_stock");
        let mut tx = cx.begin().await.unwrap_or_else(|e| panic!("{e}"));
        let done = SubscribeRestock
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(
            done.produced["variant_id"],
            planned.plan.variant_id.as_str()
        );
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));
    }
}
