//! `batch_import` (docs/api/catalog.md#batch_import): a supplier sheet becomes product drafts in
//! one change set (scene B4). The rows are read by [`crate::import`]; this file loads what the
//! reading needs, turns it into a preview, writes the drafts, reads them back, and takes them
//! back out on undo.

use std::collections::BTreeSet;

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::Money;
use acom_core::enums::{DraftStatus, ImportMode};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::import::{self, Draft, Lookup};
use crate::rules::{CopyRules, ImportRules, PriceFloorRule, PricingRule};
use crate::write::{self, DraftItem, GroupFields, NewGroup, NewImport, NewOffer, NewVariant};
use crate::{BELOW_PRICE_FLOOR, DUPLICATE_SKU, INVALID_FILE_FORMAT, neighbors, repo, slug};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchImportInput {
    /// The uploaded supplier sheet (`upl_…`), as attached to the chat or uploaded on the page.
    #[serde(rename = "acom:file", alias = "file", alias = "upload_id")]
    #[schemars(regex(pattern = r"^upl_[a-z0-9_]+$"), example = &"upl_fw26_supplier", extend("x-slot" = "upload_id"))]
    pub file: String,
    /// create: every row becomes a new draft; upsert: rows that are already drafts from an
    /// earlier import of the same products are refreshed from the sheet instead.
    #[serde(rename = "acom:mode", alias = "mode")]
    #[schemars(example = &"create")]
    pub mode: ImportMode,
    /// Must stay false: drafts are published one by one (`approve_product`), each in its own
    /// change set.
    #[serde(rename = "acom:publish_on_success", alias = "publish_on_success")]
    #[schemars(example = false)]
    pub publish_on_success: Option<bool>,
}

/// What `batch_import` runs: the drafts to create or refresh, and what the refreshed ones were.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportPlan {
    pub import_id: String,
    pub upload_id: String,
    pub mode: ImportMode,
    pub drafts: Vec<Draft>,
    /// For `upsert`: the draft rows as they were, to put back on undo.
    #[serde(default)]
    pub before: Vec<Before>,
}

/// A refreshed draft as it was before the import.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Before {
    pub product_id: String,
    pub fields: GroupFields,
    pub item: DraftItem,
    /// `(offer_id, price, cost)` of its variants.
    pub offers: Vec<(String, Money, Option<Money>)>,
}

/// Turn a supplier sheet into product drafts.
#[derive(Debug)]
pub struct BatchImport;

impl Operation for BatchImport {
    const META: OpMeta = OpMeta {
        address: "catalog.product.batch_import",
        tool: "batch_import",
        group: Group::Offer,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/catalog/products/batch-import",
        summary: "Turn an uploaded supplier sheet into product drafts: names, categories, variants and prices from the store's pricing rule, with the fields a person still has to fill flagged. Drafts are invisible to shoppers; publishing is approve_product.",
        consumes: &[Slot::one("upload_id")],
        produces: &[Slot::one("import_id"), Slot::many("product_id")],
        checks: &[
            "product.required_on_create",
            "product.sku_unique",
            "price_floor",
            "import.required_fields",
        ],
        errors: &[
            INVALID_FILE_FORMAT,
            BELOW_PRICE_FLOOR,
            DUPLICATE_SKU,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = BatchImportInput;
    type Output = Value;
}

fn upload_not_found(id: &str) -> ToolError {
    ToolError::new(
        ErrorCode::NOT_FOUND,
        format!("There is no uploaded sheet {id} in this store."),
    )
    .detail(json!({"entity": "upload", "id": id}))
    .next("batch_import")
}

/// `imp_` + the sheet's name: the first import of `upl_fw26_supplier` is `imp_fw26_supplier`,
/// the next one `imp_fw26_supplier_2`.
fn import_id(upload_id: &str, earlier: usize) -> String {
    let base = upload_id.strip_prefix("upl_").unwrap_or(upload_id);
    if earlier == 0 {
        format!("imp_{base}")
    } else {
        format!("imp_{base}_{}", earlier + 1)
    }
}

fn rules_used(
    pricing: &PricingRule,
    floor: &PriceFloorRule,
    copy: &CopyRules,
    categories: usize,
) -> Value {
    json!([
        {"rule": "pricing_rule", "label": "Price", "text": pricing.text()},
        {"rule": "price_floor", "label": "Floor", "text": format!("Never below {}", floor.markup_text())},
        {"rule": "content.copy", "label": "Tone",
         "text": format!("Your style guide, {}; the supplier's text is kept for each draft", copy.shape_text())},
        {"rule": "import.category_match", "label": "Category",
         "text": format!("Matched to your {categories} standard categories")},
    ])
}

impl WriteOp<Cx> for BatchImport {
    type Plan = ImportPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: BatchImportInput,
    ) -> Result<Planned<ImportPlan>, ToolError> {
        if input.publish_on_success == Some(true) {
            return Err(ToolError::invalid_input(
                "Drafts are published one by one with approve_product, each in its own change set; leave `acom:publish_on_success` out.",
            )
            .detail(json!({"field": "acom:publish_on_success", "reason": "enum", "allowed": [false]})));
        }
        let rules = ImportRules::from_policy(&cx.policy)?;
        let pricing = PricingRule::from_policy(&cx.policy)?;
        let floor = PriceFloorRule::from_policy(&cx.policy)?;
        let copy = CopyRules::from_policy(&cx.policy)?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let feed = write::feed(&mut tx, &input.file)
            .await?
            .ok_or_else(|| upload_not_found(&input.file))?;
        let rows: Vec<Value> = feed
            .data_feed_element
            .as_array()
            .cloned()
            .ok_or_else(|| ToolError::new(INVALID_FILE_FORMAT, "The sheet has no rows."))?;
        let earlier = write::imports_of(&mut tx, &feed.id).await?;
        let categories = neighbors::category_terms(&mut tx).await?;
        let (brands, _) = repo::brands(&mut tx, None, i64::from(i32::MAX), 0).await?;
        let brands: Vec<(String, String)> = brands.into_iter().map(|b| (b.id, b.name)).collect();
        // Every SKU and product id the sheet could make, to find the ones already taken.
        let mut skus = Vec::new();
        let mut product_ids = Vec::new();
        for r in &rows {
            if let Some(sku) = r.get("sku").and_then(Value::as_str) {
                let sku = sku.trim().to_lowercase();
                product_ids.push(format!("prd_{}", slug(&sku)));
                let sizes = r.get("sizes").and_then(Value::as_str).unwrap_or("");
                skus.push(sku.clone());
                for s in sizes.split(',').map(str::trim).filter(|s| !s.is_empty()) {
                    skus.push(format!("{sku}-{}", s.to_lowercase()));
                }
            }
        }
        let taken = write::taken_skus(&mut tx, &skus).await?;
        let existing = write::existing_groups(&mut tx, &product_ids).await?;
        let refreshable = if input.mode == ImportMode::Upsert {
            write::imported_drafts(&mut tx, &product_ids).await?
        } else {
            BTreeSet::new()
        };
        let lookup = Lookup {
            rules: &rules,
            pricing: &pricing,
            floor: &floor,
            categories: &categories,
            brands: &brands,
            taken_skus: &taken,
            existing: &existing,
            refreshable: &refreshable,
            upsert: input.mode == ImportMode::Upsert,
            currency: cx.currency,
        };
        let reading = import::read(&rows, &lookup)?;
        let checks = import::checks(&reading, &lookup);

        // What the refreshed drafts are now, for the preview's `from` and for undo.
        let refreshed: Vec<String> = reading
            .drafts
            .iter()
            .filter(|d| d.refresh)
            .map(|d| d.product_id.clone())
            .collect();
        let mut before = Vec::new();
        if !refreshed.is_empty() {
            let groups = repo::groups(&mut tx, &refreshed).await?;
            let items = write::items_of(&mut tx, &refreshed).await?;
            let variants = repo::variants(&mut tx, &refreshed).await?;
            for g in groups {
                let Some(item) = items.iter().find(|i| i.product_group_id == g.id).cloned() else {
                    continue;
                };
                let offers = variants
                    .iter()
                    .filter(|v| v.product_group_id == g.id)
                    .filter_map(|v| Some((v.offer_id.clone()?, v.list_price()?, v.cost())))
                    .collect();
                before.push(Before {
                    product_id: g.id.clone(),
                    fields: GroupFields {
                        name: Some(g.name.clone()),
                        description: None,
                        brand_id: Some(g.brand_id.clone()),
                        varies_by: None,
                        material: Some(g.material.clone()),
                        image: Some(g.image.clone()),
                    },
                    item,
                    offers,
                });
            }
        }
        tx.commit().await.map_err(ToolError::from)?;

        let id = import_id(&feed.id, earlier.len());
        let count = |s: DraftStatus| reading.count(s);
        let standard = categories
            .iter()
            .filter(|c| c.term_set == "standard")
            .count();
        let mut changes = vec![Change {
            op: ChangeOp::Create,
            entity: "product_import".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({
                "import_id": id, "upload_id": feed.id, "file_name": feed.name,
                "provider_name": feed.provider_name, "mode": input.mode.as_str(),
                "rows": reading.rows, "drafts": reading.drafts.len(),
                "ready": count(DraftStatus::Ready), "needs_you": count(DraftStatus::NeedsYou),
                "working": count(DraftStatus::Working),
                "rules": rules_used(&pricing, &floor, &copy, standard),
            }),
            note: Some(
                "Drafts are not visible to shoppers. Publishing is a separate change set.".into(),
            ),
        }];
        for d in &reading.drafts {
            let to = import::draft_json(d);
            if d.refresh {
                let from = before
                    .iter()
                    .find(|b| b.product_id == d.product_id)
                    .map(|b| {
                        json!({"name": b.fields.name, "material": b.fields.material.clone().flatten(),
                               "image": b.fields.image.clone().flatten(),
                               "acom:draft_status": b.item.draft_status,
                               "acom:open_fields": b.item.open_fields})
                    })
                    .unwrap_or(Value::Null);
                changes.push(Change {
                    op: ChangeOp::Update,
                    entity: "product".into(),
                    id: d.product_id.clone(),
                    from,
                    to,
                    note: d.note.clone(),
                });
            } else {
                changes.push(Change {
                    op: ChangeOp::Create,
                    entity: "product".into(),
                    id: String::new(),
                    from: Value::Null,
                    to,
                    note: d.note.clone(),
                });
            }
        }
        let created = reading.drafts.iter().filter(|d| !d.refresh).count();
        let reason = match (created, refreshed.len()) {
            (n, 0) => format!("Creates {n} product draft{}", if n == 1 { "" } else { "s" }),
            (0, m) => format!(
                "Refreshes {m} product draft{}",
                if m == 1 { "" } else { "s" }
            ),
            (n, m) => format!("Creates {n} and refreshes {m} product drafts"),
        };
        // A W$: always a person's approval (approval.yml thresholds.bulk_catalog_change).
        Ok(Planned {
            plan: ImportPlan {
                import_id: id,
                upload_id: feed.id.clone(),
                mode: input.mode,
                drafts: reading.drafts,
                before,
            },
            changes,
            checks,
            basis: vec![Basis {
                entity: "catalog.data_feed".into(),
                id: feed.id,
                version: i64::from(feed.version),
            }],
            approval: ApprovalNeed::Needed { reason },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ImportPlan) -> Result<Committed, ToolError> {
        let now = tx.clock.now();
        write::insert_import(
            tx,
            &NewImport {
                id: &plan.import_id,
                data_feed_id: &plan.upload_id,
                import_mode: plan.mode.as_str(),
                at: now,
            },
        )
        .await?;
        for d in &plan.drafts {
            let availability = if d.units_per_size > 0 {
                "in_stock"
            } else {
                "out_of_stock"
            };
            if d.refresh {
                write::update_group(
                    tx,
                    &d.product_id,
                    &GroupFields {
                        name: Some(d.name.clone()),
                        description: None,
                        brand_id: Some(d.brand_id.clone()),
                        varies_by: None,
                        material: Some(d.material.clone()),
                        image: Some(d.image.clone()),
                    },
                )
                .await?;
                for v in &d.variants {
                    repo::set_offer_price(
                        tx,
                        &v.offer_id,
                        d.price.value,
                        d.price.currency.as_str(),
                        Some((Some(d.cost.value), Some(d.cost.currency.to_string()))),
                    )
                    .await?;
                }
                let mut items = write::items_of(tx, std::slice::from_ref(&d.product_id)).await?;
                if let Some(mut item) = items.pop() {
                    item.supplier_text.clone_from(&d.supplier_text);
                    item.draft_status = d.status.as_str().to_owned();
                    item.open_fields.clone_from(&d.open_fields);
                    item.draft_note.clone_from(&d.note);
                    write::set_item_state(tx, &item).await?;
                }
                continue;
            }
            let varies_by: Vec<String> = if d.sizes.is_empty() {
                Vec::new()
            } else {
                vec!["size".into()]
            };
            write::insert_group(
                tx,
                &NewGroup {
                    id: &d.product_id,
                    name: &d.name,
                    description: None,
                    brand_id: d.brand_id.as_deref(),
                    varies_by: &varies_by,
                    material: d.material.as_deref(),
                    image: d.image.as_deref(),
                },
            )
            .await?;
            for v in &d.variants {
                write::insert_variant(
                    tx,
                    &NewVariant {
                        id: &v.variant_id,
                        product_group_id: &d.product_id,
                        sku: &v.sku,
                        name: &v.name,
                        color: d.color.as_deref(),
                        size: v.size.as_deref(),
                        image: None,
                        sale_type: "always",
                        is_managed: true,
                    },
                )
                .await?;
                write::insert_offer(
                    tx,
                    &NewOffer {
                        id: &v.offer_id,
                        variant_id: &v.variant_id,
                        price: d.price.value,
                        price_currency: d.price.currency.as_str(),
                        availability,
                        inventory_level: d.units_per_size,
                        cost: Some((d.cost.value, d.cost.currency.as_str())),
                    },
                )
                .await?;
            }
            neighbors::place_product(tx, &d.product_id, &d.category_id).await?;
            write::insert_item(
                tx,
                &DraftItem {
                    product_import_id: plan.import_id.clone(),
                    position: i32::try_from(d.position).unwrap_or(i32::MAX),
                    product_group_id: d.product_id.clone(),
                    supplier_code: d.supplier_code.clone(),
                    supplier_text: d.supplier_text.clone(),
                    draft_status: d.status.as_str().to_owned(),
                    open_fields: d.open_fields.clone(),
                    draft_note: d.note.clone(),
                },
            )
            .await?;
        }
        let ids: Vec<&str> = plan.drafts.iter().map(|d| d.product_id.as_str()).collect();
        let mut produced = Map::new();
        produced.insert("import_id".into(), json!(plan.import_id));
        produced.insert("product_id".into(), json!(ids));
        Ok(Committed {
            id: plan.import_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ImportPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let ids: Vec<String> = plan.drafts.iter().map(|d| d.product_id.clone()).collect();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let groups = repo::groups(&mut tx, &ids).await?;
        let variants = repo::variants(&mut tx, &ids).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        for d in &plan.drafts {
            match groups.iter().find(|g| g.id == d.product_id) {
                None => mismatches.push(format!("{} was not created", d.product_id)),
                Some(g) if g.approval_status != "draft" && !d.refresh => mismatches.push(format!(
                    "{} is {} instead of draft",
                    g.id, g.approval_status
                )),
                Some(_) => {}
            }
            for v in &d.variants {
                match variants.iter().find(|x| x.id == v.variant_id) {
                    Some(x) if x.price == Some(d.price.value) => {}
                    Some(x) => mismatches.push(format!(
                        "{} is priced {:?} instead of {}",
                        v.variant_id, x.price, d.price.value
                    )),
                    None => mismatches.push(format!("{} was not created", v.variant_id)),
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
        plan: &ImportPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        let mut published = Vec::new();
        for d in plan.drafts.iter().filter(|d| !d.refresh) {
            if !write::delete_draft(tx, &d.product_id).await? {
                published.push(d.product_id.clone());
            }
        }
        if !published.is_empty() {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "{} already left draft (submitted or published); take those back first.",
                    published.join(", ")
                ),
            )
            .detail(json!({"entity": "product", "ids": published}))
            .next("get_change_history"));
        }
        for b in &plan.before {
            write::update_group(tx, &b.product_id, &b.fields).await?;
            for (offer, price, cost) in &b.offers {
                repo::set_offer_price(
                    tx,
                    offer,
                    price.value,
                    price.currency.as_str(),
                    Some((cost.map(|c| c.value), cost.map(|c| c.currency.to_string()))),
                )
                .await?;
            }
            write::set_item_state(tx, &b.item).await?;
        }
        write::delete_import(tx, &plan.import_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::testing;

    #[test]
    fn import_ids_follow_the_sheet() {
        assert_eq!(import_id("upl_fw26_supplier", 0), "imp_fw26_supplier");
        assert_eq!(import_id("upl_fw26_supplier", 1), "imp_fw26_supplier_2");
    }

    #[test]
    fn reads_the_spec_input_and_refuses_unknown_fields() {
        let i: BatchImportInput = serde_json::from_value(
            json!({"acom:file": "upl_fw26_supplier", "acom:mode": "create"}),
        )
        .unwrap();
        assert_eq!(i.mode, ImportMode::Create);
        let i: BatchImportInput =
            serde_json::from_value(json!({"file": "upl_fw26_supplier", "mode": "upsert"})).unwrap();
        assert_eq!(i.mode, ImportMode::Upsert);
        assert!(
            serde_json::from_value::<BatchImportInput>(
                json!({"acom:file": "upl_x", "acom:mode": "replace"})
            )
            .is_err()
        );
        let schema = acom_contract::schema::closed_schema_for::<BatchImportInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert_eq!(
            schema["properties"]["acom:mode"]["enum"],
            json!(["create", "upsert"])
        );
    }

    #[tokio::test]
    async fn publishing_with_the_import_is_refused() {
        let e = BatchImport
            .preview(
                &testing::cx(acom_core::enums::ActorKind::Operator, "adm_sj"),
                serde_json::from_value(json!({"acom:file": "upl_fw26_supplier",
                    "acom:mode": "create", "acom:publish_on_success": true}))
                .unwrap(),
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "invalid_input");
    }

    #[test]
    fn the_import_always_waits_for_a_person() {
        assert_eq!(BatchImport::META.kind, Kind::WriteApproval);
        assert!(
            crate::testing::policies()
                .approval_for("batch_import", &json!({"rows": 30}))
                .needed
        );
    }
}
