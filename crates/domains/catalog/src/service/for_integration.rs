//! What `integration` uses from catalog when it imports an outside feed (`import_products`).
//!
//! New products go through catalog's own draft path, the one the supplier sheet of scene B4
//! takes: the feed's products are read by the sheet reader ([`crate::import`]: category match,
//! SKU uniqueness, open fields, the draft's name), saved as drafts by `batch_import`'s run, read
//! back by its verify, and taken out by its undo. Products already in the catalog change through
//! catalog's price floor, stock and availability rules. Nothing here adds a rule of its own: the
//! only difference from a supplier sheet is that an outside feed sets its own list price, which
//! the price floor then checks.

use std::collections::{BTreeMap, BTreeSet};

use acom_contract::{Committed, ErrorCode, ToolError, Verification, WriteOp};
use acom_core::Money;
use acom_core::enums::ImportMode;
use acom_store::{Cx, StoreError, TxCx};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub use crate::import::{Draft, DraftVariant, draft_json};
pub use crate::inventory::{StockPlan, StockSnapshot};
pub use crate::rules::PriceFloorRule;

use crate::import::{self, Lookup};
use crate::model::money_text;
use crate::product::batch_import::{BatchImport, ImportPlan};
use crate::rules::{ImportRules, PricingRule, StockRules};
use crate::{inventory, neighbors, repo, slug, write};

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

/// One new product of an outside feed, as integration read it.
#[derive(Debug, Clone, PartialEq)]
pub struct FeedProduct {
    /// The feed's code for the product: the stem of its SKUs (`<code>-<size>`) and the draft's
    /// supplier code. The product id follows from it (`prd_<code>`).
    pub code: String,
    /// The style name; the draft is named `<name>, <color>` (`catalog.import.name_format`).
    pub name: String,
    /// A standard category's name, or the feed's own word for it.
    pub category: String,
    pub color: Option<String>,
    pub sizes: Vec<String>,
    /// The unit cost; the price floor is checked against it.
    pub cost: Option<Money>,
    /// The feed's list price. It takes the place of the price the pricing rule would suggest
    /// for a supplier sheet.
    pub price: Money,
}

/// Why a feed product cannot become a draft: the rule it breaks (a catalog policy key), the
/// field, and a sentence.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DraftProblem {
    pub rule: &'static str,
    pub field: &'static str,
    pub message: String,
}

fn sheet_row(p: &FeedProduct) -> Value {
    json!({
        "supplier_code": p.code,
        "sku": p.code,
        "style_name": p.name,
        "color": p.color.clone().unwrap_or_default(),
        "category": p.category,
        "sizes": p.sizes.join(", "),
        "cost": p.cost.map(|c| c.value.to_string()).unwrap_or_default(),
        "units_per_size": "0",
    })
}

/// The product id a feed code becomes (`SKU-1182` → `prd_sku_1182`).
pub fn product_id_for(code: &str) -> String {
    format!("prd_{}", slug(&code.trim().to_lowercase()))
}

/// Reads an outside feed's new products as catalog drafts, with the rules of a supplier sheet.
/// One answer per product, in the order given: the draft, or the rule it breaks.
pub async fn read_feed_products(
    tx: &mut TxCx,
    products: &[FeedProduct],
) -> Result<Vec<Result<Draft, DraftProblem>>, ToolError> {
    if products.is_empty() {
        return Ok(Vec::new());
    }
    let rules = ImportRules::from_policy(&tx.policy)?;
    let pricing = PricingRule::from_policy(&tx.policy)?;
    let floor = PriceFloorRule::from_policy(&tx.policy)?;
    let categories = neighbors::category_terms(tx).await?;
    let rows: Vec<Value> = products.iter().map(sheet_row).collect();
    let mut skus = Vec::new();
    let mut product_ids = Vec::new();
    for p in products {
        let sku = p.code.trim().to_lowercase();
        product_ids.push(product_id_for(&sku));
        for s in &p.sizes {
            skus.push(format!("{sku}-{}", s.trim().to_lowercase()));
        }
        skus.push(sku);
    }
    let taken = write::taken_skus(tx, &skus).await?;
    let existing = write::existing_groups(tx, &product_ids).await?;
    let refreshable = BTreeSet::new();
    let lookup = Lookup {
        rules: &rules,
        pricing: &pricing,
        floor: &floor,
        categories: &categories,
        brands: &[],
        taken_skus: &taken,
        existing: &existing,
        refreshable: &refreshable,
        upsert: false,
        currency: tx.currency,
    };
    let reading = import::read(&rows, &lookup)?;
    let mut out: Vec<Option<Result<Draft, DraftProblem>>> = vec![None; products.len()];
    for p in reading.problems {
        let i = usize::try_from(p.position).unwrap_or(1).saturating_sub(1);
        if let Some(slot) = out.get_mut(i)
            && slot.is_none()
        {
            *slot = Some(Err(DraftProblem {
                rule: p.rule,
                field: p.field,
                message: p.message,
            }));
        }
    }
    for mut d in reading.drafts {
        let i = usize::try_from(d.position).unwrap_or(1).saturating_sub(1);
        let Some(p) = products.get(i) else { continue };
        let answer = if p.price.currency != d.cost.currency {
            Err(DraftProblem {
                rule: "price_floor",
                field: "price",
                message: format!(
                    "{} is priced in {} but the store sells in {}",
                    d.name, p.price.currency, d.cost.currency
                ),
            })
        } else {
            d.price = p.price.rounded();
            if floor.allows(d.price, d.cost) {
                Ok(d)
            } else {
                Err(DraftProblem {
                    rule: "price_floor",
                    field: "price",
                    message: format!(
                        "{} at {} is under its floor {} ({})",
                        d.name,
                        money_text(&d.price),
                        money_text(&d.floor),
                        floor.markup_text()
                    ),
                })
            }
        };
        if let Some(slot) = out.get_mut(i) {
            *slot = Some(answer);
        }
    }
    Ok(out
        .into_iter()
        .map(|a| {
            a.unwrap_or_else(|| {
                Err(DraftProblem {
                    rule: "product.required_on_create",
                    field: "name",
                    message: "The row could not be read".into(),
                })
            })
        })
        .collect())
}

/// The new products of one import, as drafts, and the feed rows they came from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FeedDrafts {
    /// The import (`imp_…`); catalog records its drafts under the same id.
    pub import_id: String,
    /// The data feed that keeps the rows (`upl_…`).
    pub upload_id: String,
    /// A name for the feed, as the uploaded sheets have one.
    pub file_name: String,
    /// Who sent the feed: the seller or the channel; `None` for the store's own feed.
    pub provider_name: Option<String>,
    pub drafts: Vec<Draft>,
    /// Units each new variant starts with: `(variant_id, units)`.
    pub stock: Vec<(String, i32)>,
}

fn import_plan(f: &FeedDrafts) -> ImportPlan {
    ImportPlan {
        import_id: f.import_id.clone(),
        upload_id: f.upload_id.clone(),
        mode: ImportMode::Create,
        drafts: f.drafts.clone(),
        before: Vec::new(),
    }
}

/// Saves the drafts the way `batch_import` runs: the feed's rows as a data feed, the import, the
/// draft products with their variants, offers and category, then each variant's own stock.
pub async fn save_feed_drafts(tx: &mut TxCx, f: &FeedDrafts) -> Result<(), ToolError> {
    if f.drafts.is_empty() {
        return Ok(());
    }
    let elements: Vec<Value> = f
        .drafts
        .iter()
        .map(|d| {
            json!({"supplier_code": d.supplier_code, "sku": d.sku.to_uppercase(),
                   "style_name": d.name, "color": d.color, "category": d.category_name,
                   "sizes": d.sizes.join(", "), "cost": d.cost.value.to_string(),
                   "price": d.price.value.to_string()})
        })
        .collect();
    sqlx::query(
        "INSERT INTO catalog.data_feed (tenant_id, id, name, encoding_format, provider_name,
           data_feed_element)
         VALUES ($1, $2, $3, 'application/json', $4, $5)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(&f.upload_id)
    .bind(&f.file_name)
    .bind(f.provider_name.as_deref())
    .bind(Value::Array(elements))
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    // A variant with stock starts in stock; the exact units per variant follow below.
    let mut plan = import_plan(f);
    for d in &mut plan.drafts {
        d.units_per_size = d
            .variants
            .iter()
            .filter_map(|v| f.stock.iter().find(|(id, _)| *id == v.variant_id))
            .map(|(_, units)| *units)
            .max()
            .unwrap_or(0);
    }
    BatchImport.commit(tx, &plan).await?;
    let rules = StockRules::from_policy(&tx.policy)?;
    let now = tx.clock.now();
    for (variant_id, units) in &f.stock {
        let Some(s) = repo::offer_state(tx, variant_id).await? else {
            continue;
        };
        if s.inventory_level != *units {
            let (p, _) = inventory::plan_stock(&rules, &s, i64::from(*units), None, now)?;
            inventory::write(tx, &p, &p.to).await?;
        }
    }
    Ok(())
}

/// Reads the drafts back as `batch_import` does.
pub async fn verify_feed_drafts(cx: &Cx, f: &FeedDrafts) -> Result<Verification, ToolError> {
    if f.drafts.is_empty() {
        return Ok(Verification {
            matched: true,
            mismatches: Vec::new(),
        });
    }
    let done = Committed {
        id: f.import_id.clone(),
        produced: Map::new(),
    };
    BatchImport.verify(cx, &import_plan(f), &done).await
}

/// Takes the drafts out again, as undoing `batch_import` does: refused once a draft was
/// submitted or published. The data feed stays, like an uploaded sheet.
pub async fn remove_feed_drafts(tx: &mut TxCx, f: &FeedDrafts) -> Result<(), ToolError> {
    if f.drafts.is_empty() {
        return Ok(());
    }
    let done = Committed {
        id: f.import_id.clone(),
        produced: Map::new(),
    };
    BatchImport.undo(tx, &import_plan(f), &done).await
}

/// One variant as an import sees it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VariantState {
    pub variant_id: String,
    /// `prd_…`
    pub product_id: String,
    pub offer_id: String,
    /// List price.
    pub price: Money,
    pub cost: Option<Money>,
    /// `schema:availability` word.
    pub availability: String,
    pub inventory_level: i32,
    /// The offer row's version, for a preview's basis.
    pub offer_version: i64,
}

/// The variants that exist among `variant_ids`, with their offers.
pub async fn variant_states(
    tx: &mut TxCx,
    variant_ids: &[String],
) -> Result<Vec<VariantState>, ToolError> {
    let mut out = Vec::new();
    for id in variant_ids {
        let Some(s) = repo::offer_state(tx, id).await? else {
            continue;
        };
        let money = |value, currency: &str| {
            acom_core::Currency::parse(currency.trim())
                .map(|c| Money::new(value, c))
                .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))
        };
        let cost = match (s.cost_value, s.cost_currency.as_deref()) {
            (Some(v), Some(c)) => Some(money(v, c)?),
            _ => None,
        };
        out.push(VariantState {
            price: money(s.price, &s.price_currency)?,
            variant_id: s.variant_id,
            product_id: s.product_group_id,
            offer_id: s.offer_id,
            cost,
            availability: s.availability,
            inventory_level: s.inventory_level,
            offer_version: i64::from(s.version),
        });
    }
    Ok(out)
}

/// Every variant of these products, with their offers.
pub async fn product_variants(
    tx: &mut TxCx,
    product_ids: &[String],
) -> Result<Vec<VariantState>, ToolError> {
    let ids: Vec<String> = repo::variants(tx, product_ids)
        .await?
        .into_iter()
        .map(|v| v.id)
        .collect();
    variant_states(tx, &ids).await
}

/// A standard category a feed's word may map to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StandardCategory {
    pub id: String,
    pub name: String,
    /// `false` when it or a category above it is inactive: nothing new is placed in it.
    pub active: bool,
}

/// The store's standard categories (the ones products are placed in).
pub async fn standard_categories(tx: &mut TxCx) -> Result<Vec<StandardCategory>, ToolError> {
    Ok(neighbors::category_terms(tx)
        .await?
        .into_iter()
        .filter(|c| c.term_set == "standard")
        .map(|c| StandardCategory {
            id: c.id,
            name: c.name,
            active: c.active,
        })
        .collect())
}

/// The variants whose SKU is one of `skus` (case ignored): lowercase SKU → variant id.
pub async fn variants_by_sku(
    tx: &mut TxCx,
    skus: &[String],
) -> Result<BTreeMap<String, String>, ToolError> {
    let lower: Vec<String> = skus.iter().map(|s| s.trim().to_lowercase()).collect();
    let rows: Vec<(String, String)> = sqlx::query_as(
        "SELECT lower(sku), id FROM catalog.product WHERE tenant_id = $1 AND lower(sku) = ANY($2)",
    )
    .bind(tx.tenant.as_str())
    .bind(&lower)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(rows.into_iter().collect())
}

/// Product names by id, for what a preview shows.
pub async fn product_names(
    tx: &mut TxCx,
    product_ids: &[String],
) -> Result<BTreeMap<String, String>, ToolError> {
    Ok(repo::groups(tx, product_ids)
        .await?
        .into_iter()
        .map(|g| (g.id, g.name))
        .collect())
}

/// Sets an offer's list price (the cost stays). The caller checked the price floor.
pub async fn set_list_price(tx: &mut TxCx, offer_id: &str, price: Money) -> Result<(), ToolError> {
    let n = repo::set_offer_price(tx, offer_id, price.value, price.currency.as_str(), None).await?;
    if n == 0 {
        return Err(ToolError::new(
            ErrorCode::NOT_FOUND,
            format!("There is no offer {offer_id}."),
        )
        .detail(json!({"entity": "offer", "id": offer_id})));
    }
    Ok(())
}

/// What setting a variant's stock to `level` changes, with catalog's stock rules (out of stock at
/// zero, back in stock only after running out by itself). Refused for a discontinued variant.
pub async fn plan_stock_level(
    tx: &mut TxCx,
    variant_id: &str,
    level: i64,
) -> Result<StockPlan, ToolError> {
    let rules = StockRules::from_policy(&tx.policy)?;
    let now = tx.clock.now();
    let s = repo::offer_state(tx, variant_id).await?.ok_or_else(|| {
        ToolError::new(
            ErrorCode::NOT_FOUND,
            format!("There is no variant {variant_id}."),
        )
        .detail(json!({"entity": "variant", "id": variant_id}))
    })?;
    Ok(inventory::plan_stock(&rules, &s, level, None, now)?.0)
}

/// Writes a stock plan (`back` writes what it was). A variant that comes back in stock tells the
/// shoppers waiting for it, as `set_variant_stock` does.
pub async fn write_stock(tx: &mut TxCx, plan: &StockPlan, back: bool) -> Result<(), ToolError> {
    if back {
        return inventory::write(tx, plan, &plan.from).await;
    }
    inventory::write(tx, plan, &plan.to).await?;
    if plan.restores() {
        crate::restock_alert::notify_back_in_stock(tx, &plan.variant_id).await?;
    }
    Ok(())
}

/// Whether a variant may move to `to` availability: a discontinued variant is not sold again
/// (`discontinue_product` cannot be undone). `Ok(false)` when it is there already.
pub fn availability_moves(v: &VariantState, to: &str) -> Result<bool, ToolError> {
    if v.availability == to {
        return Ok(false);
    }
    if v.availability == "discontinued" {
        return Err(ToolError::new(
            ErrorCode::INVALID_STATE,
            format!("{} is discontinued; it is not sold again.", v.variant_id),
        )
        .detail(json!({"entity": "variant", "from": "discontinued", "allowed_to": []}))
        .next("get_product"));
    }
    Ok(true)
}

/// Sets a variant's availability. Discontinuing goes the way `discontinue_product` does.
pub async fn set_availability(
    tx: &mut TxCx,
    offer_id: &str,
    availability: &str,
) -> Result<(), ToolError> {
    if availability == "discontinued" {
        repo::discontinue_offer(tx, offer_id).await?;
    } else {
        write::set_availability(tx, offer_id, availability).await?;
    }
    Ok(())
}
