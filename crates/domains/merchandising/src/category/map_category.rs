//! `map_category` (docs/api/merchandising.md#category).

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Check, Committed, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::*;
use crate::check_id;
use crate::repo;

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MapCategoryInput {
    /// The standard category whose products are shown.
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_summer_outerwear")]
    pub standard_category_id: String,
    /// Display categories that show them.
    #[schemars(length(min = 1, max = 20), example = ["cat_new_in", "cat_travel"])]
    pub display_category_ids: Vec<String>,
    /// Make the first display category the standard category's primary one (the path product
    /// pages show). A standard category has one.
    #[schemars(example = true)]
    pub is_primary: Option<bool>,
}

/// What `map_category` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MapPlan {
    pub created: Vec<Mapping>,
}

/// Show a standard category's products in display categories.
pub struct MapCategory;

impl Operation for MapCategory {
    const META: OpMeta = OpMeta {
        address: "merchandising.category.map",
        tool: "map_category",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/merchandising/category-mappings",
        summary: "Show a standard category's products in one or more display categories of the shopper menu. Then call invalidate_display_cache.",
        consumes: &[Slot::one("category_id"), Slot::many("category_id")],
        produces: &[Slot::many("category_mapping_id")],
        checks: &["category.mapping", "category.primary_display_per_standard"],
        errors: &[
            CROSS_SET_VIOLATION,
            PRIMARY_CONFLICT,
            DISPLAY_CATEGORY_INACTIVE,
            CATEGORY_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = MapCategoryInput;
    type Output = Value;
}

pub(super) fn mapping_id(standard: &str, display: &str) -> String {
    format!(
        "map_{}_{}",
        standard.trim_start_matches("cat_"),
        display.trim_start_matches("cat_")
    )
}

impl WriteOp<Cx> for MapCategory {
    type Plan = MapPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: MapCategoryInput,
    ) -> Result<Planned<MapPlan>, ToolError> {
        let rules = CategoryRules::from_policy(&cx.policy)?;
        check_id("standard_category_id", "cat", &input.standard_category_id)?;
        if input.display_category_ids.is_empty() {
            return Err(
                ToolError::invalid_input("Name at least one display category.")
                    .detail(json!({"field": "display_category_ids", "reason": "missing"})),
            );
        }
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let std = repo::category(&mut tx, &input.standard_category_id)
            .await?
            .ok_or_else(|| not_found(&input.standard_category_id))?;
        if std.term_set != "standard" {
            return Err(ToolError::new(
                CROSS_SET_VIOLATION,
                format!("{} is a display category; mappings go from a standard category to display ones.", std.id),
            )
            .detail(json!({"field": "standard_category_id", "term_set": std.term_set, "rule": "category.mapping"}))
            .next("map_category"));
        }
        let existing = mappings_of(&mut tx, &std.id).await?;
        let mut displays = Vec::new();
        for d in &input.display_category_ids {
            check_id("display_category_ids", "cat", d)?;
            let c = repo::category(&mut tx, d)
                .await?
                .ok_or_else(|| not_found(d))?;
            if c.term_set != "display" {
                return Err(ToolError::new(
                    CROSS_SET_VIOLATION,
                    format!("{d} is a standard category; a standard category maps to display ones."),
                )
                .detail(json!({"field": "display_category_ids", "term_set": c.term_set, "rule": "category.mapping"}))
                .next("map_category"));
            }
            if !c.active {
                return Err(ToolError::new(
                    DISPLAY_CATEGORY_INACTIVE,
                    format!("{d} is switched off; switch it on first."),
                )
                .detail(json!({"entity": "category", "id": d, "from": "inactive"}))
                .next("update_category"));
            }
            displays.push(c);
        }
        let products = repo::product_count(&mut tx, &std.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let want_primary = input.is_primary == Some(true);
        let mut created = Vec::new();
        let mut changes = Vec::new();
        for (i, d) in displays.iter().enumerate() {
            if existing.iter().any(|m| m.display_category_id == d.id) {
                continue;
            }
            let m = Mapping {
                id: mapping_id(&std.id, &d.id),
                standard_category_id: std.id.clone(),
                display_category_id: d.id.clone(),
                is_primary: want_primary && i == 0,
            };
            changes.push(Change {
                op: ChangeOp::Create,
                entity: "category_mapping".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"category_mapping_id": m.id, "standard_category_id": m.standard_category_id,
                           "display_category_id": m.display_category_id, "is_primary": m.is_primary,
                           "products_shown": products}),
                note: Some(format!("{products} products of {} show under {}", std.name, d.name)),
            });
            created.push(m);
        }
        if created.is_empty() {
            return Err(ToolError::invalid_input(
                "Those display categories already show this standard category.",
            )
            .detail(
                json!({"field": "display_category_ids", "reason": "missing", "existing": existing}),
            ));
        }
        let primaries = existing.iter().filter(|m| m.is_primary).count() as u64
            + created.iter().filter(|m| m.is_primary).count() as u64;
        let conflict = primaries > rules.primary_per_standard;
        Ok(Planned {
            changes,
            checks: vec![
                Check {
                    rule: "category.mapping".into(),
                    passed: true,
                    detail: Some(format!(
                        "{} (standard) to display categories only",
                        std.name
                    )),
                    fix_tool: None,
                },
                Check {
                    rule: "category.primary_display_per_standard".into(),
                    passed: !conflict,
                    detail: Some(if conflict {
                        format!(
                            "{} already has a primary display category ({}); unmap it or keep this one not primary",
                            std.name,
                            existing
                                .iter()
                                .find(|m| m.is_primary)
                                .map_or("", |m| m.display_category_id.as_str())
                        )
                    } else {
                        format!(
                            "{} has at most {} primary display category",
                            std.name, rules.primary_per_standard
                        )
                    }),
                    fix_tool: conflict.then(|| "unmap_category".into()),
                },
            ],
            basis: Vec::new(),
            approval: crate::policy_approval(cx, Self::META.tool, &json!({})),
            plan: MapPlan { created },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &MapPlan) -> Result<Committed, ToolError> {
        for m in &plan.created {
            insert_mapping(tx, m).await?;
        }
        let ids: Vec<&str> = plan.created.iter().map(|m| m.id.as_str()).collect();
        let mut produced = Map::new();
        produced.insert("category_mapping_id".into(), json!(ids));
        Ok(Committed {
            id: ids.first().map(|s| (*s).to_owned()).unwrap_or_default(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &MapPlan,
        _d: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        for m in &plan.created {
            if mapping(&mut tx, &m.id).await?.as_ref() != Some(m) {
                mismatches.push(format!("{} is not there", m.id));
            }
        }
        tx.commit().await.map_err(ToolError::from)?;
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(&self, tx: &mut TxCx, plan: &MapPlan, _d: &Committed) -> Result<(), ToolError> {
        for m in &plan.created {
            delete_mapping(tx, &m.id).await?;
        }
        Ok(())
    }
}
