//! `arrange_display_slot` (docs/api/merchandising.md#display_slot).
//!
//! Adds, removes and reorders a slot's items (`item_list_element`). The period and the audience
//! are `schedule_display`'s; this only changes what the slot holds, so it needs no approval.

use std::collections::{BTreeMap, BTreeSet};

use acom_catalog::service::for_merchandising::{ProductCard, brand_names, product_cards};
use acom_contract::{
    ActionType, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::SlotType;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    DUPLICATE_POSITION, ITEM_DISCONTINUED, SLOT_NOT_FOUND, SLOT_TYPE_MISMATCH, SlotItem,
    discontinued, discontinued_check, hidden_reason, product_ids, slot_items,
};
use crate::model::{DisplayRules, item_kind, slot_words};
use crate::repo;
use crate::{check_id, policy_approval};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ArrangeDisplaySlotInput {
    /// The slot (`list_display_slots`).
    #[schemars(regex(pattern = r"^slot_[a-z0-9_]+$"), example = &"slot_home_new_in")]
    pub slot_id: String,
    /// Every item the slot should hold, each with its place; items left out are taken out of
    /// the slot. An empty list empties it.
    #[schemars(length(max = 1000), example = json!([{"position": 1, "item_id": "prd_wool_overcoat"}, {"position": 2, "item_id": "prd_cashmere_sweater"}]))]
    pub item_list_element: Vec<SlotItem>,
    /// The kind of slot you expect: hero, banner_board, product_grid, brand_strip or
    /// collection_feed. When the slot is another kind, nothing changes (`slot_type_mismatch`).
    #[schemars(example = &"product_grid")]
    pub slot_type: Option<SlotType>,
}

/// What `arrange_display_slot` runs, with what it replaces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArrangePlan {
    pub slot_id: String,
    /// The new items, in place order.
    pub items: Vec<SlotItem>,
    /// The slot's items as they were (as stored), for undo.
    pub before: Value,
}

/// Add, remove and reorder the items of a slot.
#[derive(Debug)]
pub struct ArrangeDisplaySlot;

impl Operation for ArrangeDisplaySlot {
    const META: OpMeta = OpMeta {
        address: "merchandising.display_slot.arrange",
        tool: "arrange_display_slot",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Put,
        path: "/v1/merchandising/display-slots/{slot_id}/items",
        summary: "Set which items a display slot holds and in what order (products, banners, curated pages, or brands in a brand strip). Period and audience are schedule_display's; booking the arrangement for shoppers is a separate, approved step.",
        consumes: &[Slot::one("slot_id"), Slot::many("product_id")],
        produces: &[Slot::one("slot_id")],
        checks: &["display.max_items_per_slot", "display.exclude_discontinued"],
        errors: &[
            SLOT_NOT_FOUND,
            DUPLICATE_POSITION,
            SLOT_TYPE_MISMATCH,
            ITEM_DISCONTINUED,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = ArrangeDisplaySlotInput;
    type Output = Value;
}

/// The items in place order; two items in one place, or one item twice, is refused.
fn ordered(mut items: Vec<SlotItem>) -> Result<Vec<SlotItem>, ToolError> {
    items.sort_by_key(|i| i.position);
    for pair in items.windows(2) {
        if pair[0].position == pair[1].position {
            return Err(ToolError::new(
                DUPLICATE_POSITION,
                format!(
                    "{} and {} are both in place {}.",
                    pair[0].item_id, pair[1].item_id, pair[0].position
                ),
            )
            .detail(json!({"field": "item_list_element", "position": pair[0].position}))
            .next("arrange_display_slot"));
        }
    }
    let mut seen = BTreeSet::new();
    for i in &items {
        if !seen.insert(i.item_id.as_str()) {
            return Err(ToolError::invalid_input(format!(
                "{} is in the list twice; give each item one place.",
                i.item_id
            ))
            .detail(json!({"field": "item_list_element", "reason": "format", "id": i.item_id})));
        }
    }
    Ok(items)
}

/// Each item's kind fits the slot (`display.slot_items`).
fn check_kinds(rules: &DisplayRules, slot_type: &str, items: &[SlotItem]) -> Result<(), ToolError> {
    let allowed = rules.slot_holds(slot_type);
    for i in items {
        let kind = item_kind(&i.item_id).unwrap_or("unknown");
        if !allowed.iter().any(|k| k == kind) {
            return Err(ToolError::new(
                SLOT_TYPE_MISMATCH,
                format!(
                    "A {} holds {}; {} is a {}.",
                    slot_words(slot_type).to_lowercase(),
                    if allowed.is_empty() {
                        "nothing".to_owned()
                    } else {
                        allowed.join(" or ").replace('_', " ")
                    },
                    i.item_id,
                    kind.replace('_', " ")
                ),
            )
            .detail(
                json!({"slot_type": slot_type, "item_id": i.item_id, "kind": kind,
                           "allowed": allowed}),
            )
            .next("get_display_slot"));
        }
    }
    Ok(())
}

fn missing(entity: &str, id: &str, next: &'static str) -> ToolError {
    ToolError::new(
        ErrorCode::NOT_FOUND,
        format!("There is no {} {id}.", entity.replace('_', " ")),
    )
    .detail(json!({"entity": entity, "id": id}))
    .next(next)
}

/// Every item exists; returns the product cards (for discontinued and sold-out products).
async fn known(tx: &mut TxCx, items: &[SlotItem]) -> Result<Vec<ProductCard>, ToolError> {
    let of = |kind: &str| -> Vec<String> {
        items
            .iter()
            .filter(|i| item_kind(&i.item_id) == Some(kind))
            .map(|i| i.item_id.clone())
            .collect()
    };
    let products = of("product");
    let cards = product_cards(tx, &products).await?;
    if let Some(p) = products
        .iter()
        .find(|p| !cards.iter().any(|c| &c.product_id == *p))
    {
        return Err(missing("product", p, "search_products"));
    }
    let banners = of("banner");
    let found = repo::banner_ids_known(tx, &banners).await?;
    if let Some(b) = banners.iter().find(|b| !found.contains(b)) {
        return Err(missing("banner", b, "list_banners"));
    }
    let pages = of("collection_page");
    let found = repo::collection_page_ids_known(tx, &pages).await?;
    if let Some(c) = pages.iter().find(|c| !found.contains(c)) {
        return Err(missing("collection_page", c, "list_curations"));
    }
    let brands = of("brand");
    let found = brand_names(tx, &brands).await?;
    if let Some(b) = brands
        .iter()
        .find(|b| !found.iter().any(|(id, _)| id == *b))
    {
        return Err(missing("brand", b, "list_brands"));
    }
    Ok(cards)
}

impl WriteOp<Cx> for ArrangeDisplaySlot {
    type Plan = ArrangePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ArrangeDisplaySlotInput,
    ) -> Result<Planned<ArrangePlan>, ToolError> {
        check_id("slot_id", "slot", &input.slot_id)?;
        let rules = DisplayRules::from_policy(&cx.policy)?;
        let items = ordered(input.item_list_element)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let slot = repo::slot(&mut tx, &input.slot_id).await?.ok_or_else(|| {
            ToolError::new(
                SLOT_NOT_FOUND,
                format!("There is no slot {}.", input.slot_id),
            )
            .detail(json!({"entity": "display_slot", "id": input.slot_id}))
            .next("list_display_slots")
        })?;
        if let Some(t) = input.slot_type
            && t.as_str() != slot.slot_type
        {
            return Err(ToolError::new(
                SLOT_TYPE_MISMATCH,
                format!(
                    "{} is a {} slot, not a {} slot.",
                    slot.id,
                    slot.slot_type.replace('_', " "),
                    t.as_str().replace('_', " ")
                ),
            )
            .detail(json!({"slot_type": slot.slot_type, "expected": t.as_str()}))
            .next("get_display_slot"));
        }
        check_kinds(&rules, &slot.slot_type, &items)?;
        let cards = known(&mut tx, &items).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let by_id: BTreeMap<&str, &ProductCard> =
            cards.iter().map(|c| (c.product_id.as_str(), c)).collect();
        let hidden: Vec<Value> = product_ids(&items)
            .iter()
            .filter_map(|p| {
                hidden_reason(&rules, by_id.get(p.as_str()).copied())
                    .map(|r| json!({"product_id": p, "reason": r}))
            })
            .collect();
        let count = items.len();
        let max_check = Check {
            rule: "display.max_items_per_slot".into(),
            passed: count <= rules.max_items_per_slot,
            detail: Some(format!(
                "{count} item{}; a slot holds at most {}",
                if count == 1 { "" } else { "s" },
                rules.max_items_per_slot
            )),
            fix_tool: (count > rules.max_items_per_slot).then(|| "arrange_display_slot".into()),
        };
        let unchanged = slot_items(&slot.item_list_element) == items;
        let change = Change {
            op: ChangeOp::Update,
            entity: "display_slot".into(),
            id: slot.id.clone(),
            from: json!({"item_list_element": slot.item_list_element}),
            to: json!({"item_list_element": items, "hidden_items": hidden}),
            note: unchanged.then(|| "The slot already holds these items in this order".into()),
        };
        Ok(Planned {
            plan: ArrangePlan {
                slot_id: slot.id.clone(),
                items,
                before: slot.item_list_element.clone(),
            },
            changes: vec![change],
            checks: vec![max_check, discontinued_check(&rules, &cards)],
            basis: vec![Basis {
                entity: "merchandising.display_slot".into(),
                id: slot.id,
                version: i64::from(slot.version),
            }],
            approval: policy_approval(
                cx,
                Self::META.tool,
                &json!({"reaches_customers": rules.reaches(Self::META.tool)}),
            ),
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ArrangePlan) -> Result<Committed, ToolError> {
        let rules = DisplayRules::from_policy(&tx.policy)?;
        if repo::slot(tx, &plan.slot_id).await?.is_none() {
            return Err(ToolError::new(
                SLOT_NOT_FOUND,
                format!("There is no slot {}.", plan.slot_id),
            )
            .detail(json!({"entity": "display_slot", "id": plan.slot_id}))
            .next("list_display_slots"));
        }
        // A product may have been discontinued since the preview.
        let cards = product_cards(tx, &product_ids(&plan.items)).await?;
        let gone = discontinued(&cards);
        if rules.exclude_discontinued && !gone.is_empty() {
            return Err(ToolError::new(
                ITEM_DISCONTINUED,
                format!(
                    "{} can no longer be displayed: discontinued.",
                    gone.join(", ")
                ),
            )
            .detail(json!({"rule": "display.exclude_discontinued", "items": gone}))
            .next("search_products"));
        }
        repo::set_slot_items(tx, &plan.slot_id, &json!(plan.items)).await?;
        let mut produced = Map::new();
        produced.insert("slot_id".into(), json!(plan.slot_id));
        Ok(Committed {
            id: plan.slot_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ArrangePlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let slot = repo::slot(&mut tx, &plan.slot_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = slot.is_some_and(|s| slot_items(&s.item_list_element) == plan.items);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!(
                    "{} does not hold the planned items in order",
                    plan.slot_id
                )]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &ArrangePlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        repo::set_slot_items(tx, &plan.slot_id, &plan.before)
            .await
            .map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::policies;

    fn item(position: i32, id: &str) -> SlotItem {
        SlotItem {
            position,
            item_id: id.into(),
        }
    }

    #[test]
    fn takes_the_spec_input_with_a_closed_schema() {
        let i: ArrangeDisplaySlotInput = serde_json::from_value(json!({
            "slot_id": "slot_home_new_in", "slot_type": "product_grid",
            "item_list_element": [{"position": 2, "item_id": "prd_cashmere_sweater"},
                                  {"position": 1, "item_id": "prd_wool_overcoat"}]}))
        .unwrap();
        assert_eq!(
            ordered(i.item_list_element).unwrap()[0].item_id,
            "prd_wool_overcoat"
        );
        assert!(
            serde_json::from_value::<ArrangeDisplaySlotInput>(json!({
                "slot_id": "slot_home_new_in",
                "item_list_element": [{"position": 1, "item_id": "prd_x", "note": "x"}]}))
            .is_err()
        );
        let schema = acom_contract::schema::closed_schema_for::<ArrangeDisplaySlotInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert_eq!(ArrangeDisplaySlot::META.kind, Kind::Write);
    }

    #[test]
    fn places_are_unique_and_kinds_fit_the_slot() {
        let e = ordered(vec![item(1, "prd_a"), item(1, "prd_b")]).unwrap_err();
        assert_eq!(e.code, "duplicate_position");
        let e = ordered(vec![item(1, "prd_a"), item(2, "prd_a")]).unwrap_err();
        assert_eq!(e.code, "invalid_input");

        let rules = DisplayRules::from_policy(&policies(false)).unwrap();
        assert!(check_kinds(&rules, "product_grid", &[item(1, "prd_a")]).is_ok());
        assert!(check_kinds(&rules, "brand_strip", &[item(1, "brn_northwind")]).is_ok());
        let e = check_kinds(&rules, "product_grid", &[item(1, "bnr_fall_layers")]).unwrap_err();
        assert_eq!(e.code, "slot_type_mismatch");
        assert_eq!(e.next_tool.as_deref(), Some("get_display_slot"));
    }

    #[test]
    fn discontinued_products_fail_the_check() {
        let rules = DisplayRules::from_policy(&policies(false)).unwrap();
        let card = |id: &str, status: &str| ProductCard {
            product_id: id.into(),
            name: "Linen shirt".into(),
            approval_status: status.into(),
            image: None,
            colors: vec![],
            inventory_level: 3,
        };
        assert!(discontinued_check(&rules, &[card("prd_a", "active")]).passed);
        let c = discontinued_check(&rules, &[card("prd_linen_shirt", "discontinued")]);
        assert!(!c.passed);
        assert_eq!(
            c.detail.as_deref(),
            Some("Discontinued: Linen shirt (prd_linen_shirt)")
        );
        assert_eq!(c.fix_tool.as_deref(), Some("search_products"));
    }
}
