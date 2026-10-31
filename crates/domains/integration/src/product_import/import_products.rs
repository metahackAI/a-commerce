//! `import_products` (`integration.product_import.import_products`): an outside feed — a
//! seller's or the store's own — brings products, prices, stock and availability in one batch.
//! A person always approves (W$).
//!
//! - New products go through catalog's own draft path (the supplier-sheet reader of scene B4:
//!   category match, SKU uniqueness, the draft's name and open fields), with the feed's own list
//!   price checked against the price floor. They stay drafts, invisible to shoppers, until
//!   `approve_product`.
//! - Products an earlier import made, or whose variant SKUs the feed names, change through
//!   catalog's price floor, stock and availability rules.
//! - Rows are taken or rejected one by one (`import.partial_success`); every rejection says why
//!   and which tool fixes it, and `get_import_status` shows them after the run.

use std::collections::{BTreeMap, BTreeSet};

use acom_catalog::service::for_integration as catalog;
use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::Money;
use acom_core::enums::{Availability, FeedImportMode};
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{RowError, RowResult};
use crate::repo::{self, NewImport, NewImportItem};
use crate::rules::ImportRules;
use crate::service::for_partner::seller_status;
use crate::{
    BATCH_TOO_LARGE, BELOW_PRICE_FLOOR, CATEGORY_UNMAPPED, DUPLICATE_SKU, MISSING_REQUIRED_FIELD,
    SELLER_NOT_ACTIVE, check_id, money_text,
};

/// One variant of a feed row.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FeedVariant {
    /// The feed's code for the variant, often its SKU. A code an earlier import of the same
    /// feed took, or a SKU the catalog has, finds the variant again.
    #[schemars(length(min = 1, max = 120), example = &"SKU-1182-M")]
    pub external_id: String,
    /// Size label.
    #[schemars(length(min = 1, max = 20), example = &"M")]
    pub size: Option<String>,
    /// Color name; one color per row (a product is one color).
    #[schemars(length(min = 1, max = 40), example = &"Charcoal")]
    pub color: Option<String>,
    /// Units that can be sold now.
    #[schemars(range(min = 0), example = 12)]
    pub inventory_level: Option<i64>,
    /// Whether it can be bought (status_only, and upsert of a product already in the catalog).
    /// in_stock; out_of_stock; pre_order; back_order; discontinued: never sold again.
    #[schemars(example = &"in_stock")]
    pub availability: Option<Availability>,
}

/// One product row of the feed.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct FeedItem {
    /// The feed's code for the product; the same code in a later import updates it.
    #[schemars(length(min = 1, max = 120), example = &"SKU-1182")]
    pub external_id: String,
    /// Style name; a new product is named "<name>, <color>".
    #[schemars(length(min = 1, max = 200), example = &"Merino trail tee")]
    pub name: Option<String>,
    /// The feed's category word, matched to a standard category by `acom:category_mapping`, the
    /// feed's earlier mappings, or a standard category's name.
    #[schemars(length(min = 1, max = 120), example = &"Tees")]
    pub category: Option<String>,
    /// List price of every variant of the row.
    #[schemars(example = json!({"value": 58.00, "currency": "USD"}))]
    pub price: Option<Money>,
    /// Unit cost, for the price floor (a product the catalog has keeps its own cost).
    #[schemars(example = json!({"value": 22.00, "currency": "USD"}))]
    pub cost: Option<Money>,
    /// The row's variants (sizes of one color).
    #[schemars(example = json!([{"external_id": "SKU-1182-M", "size": "M", "color": "Charcoal", "inventory_level": 12}]))]
    pub variants: Option<Vec<FeedVariant>>,
}

/// A feed category word and the standard category it is.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CategoryWord {
    /// The feed's word, as in the rows' `category`.
    #[schemars(length(min = 1, max = 120), example = &"Trail tops")]
    pub external_category: String,
    /// The standard category it is (`list_categories`).
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_tees")]
    pub category_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ImportProductsInput {
    /// The seller whose feed it is; left out for the store's own feed.
    #[schemars(regex(pattern = r"^slr_[a-z0-9_]+$"), example = &"slr_harbor_goods")]
    pub seller_id: Option<String>,
    /// What the feed may change. upsert: create new products as drafts and update the price,
    /// stock and availability of known ones; price_only: list prices of known products;
    /// stock_only: stock of known variants; status_only: availability of known variants.
    #[serde(rename = "acom:import_mode", alias = "import_mode")]
    #[schemars(example = &"upsert")]
    pub mode: FeedImportMode,
    /// The feed's rows, at most `import.max_items_per_batch`.
    #[schemars(length(min = 1), example = json!([{"external_id": "SKU-1182", "name": "Merino trail tee", "category": "Tees", "price": {"value": 58.00, "currency": "USD"}, "cost": {"value": 22.00, "currency": "USD"}, "variants": [{"external_id": "SKU-1182-M", "size": "M", "color": "Charcoal", "inventory_level": 12}]}]))]
    pub items: Vec<FeedItem>,
    /// Feed category words and the standard categories they are; words left out use the
    /// feed's earlier mappings, then the standard categories' names.
    #[serde(rename = "acom:category_mapping", alias = "category_mapping")]
    #[schemars(example = json!([{"external_category": "Trail tops", "category_id": "cat_tees"}]))]
    pub category_mapping: Option<Vec<CategoryWord>>,
}

/// A list price that changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PriceChange {
    pub variant_id: String,
    pub offer_id: String,
    pub from: Money,
    pub to: Money,
}

/// An availability that changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AvailabilityChange {
    pub variant_id: String,
    pub offer_id: String,
    pub from: String,
    pub to: String,
}

/// What `import_products` runs, fixed at preview.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImportProductsPlan {
    pub import_id: String,
    pub seller_id: Option<String>,
    pub mode: FeedImportMode,
    /// The category words this run resolved (lowercase word → `cat_…`); later runs start from it.
    pub category_mapping: BTreeMap<String, String>,
    pub rows: Vec<RowResult>,
    /// New products, as catalog drafts.
    pub drafts: catalog::FeedDrafts,
    pub prices: Vec<PriceChange>,
    pub stock: Vec<catalog::StockPlan>,
    pub availability: Vec<AvailabilityChange>,
}

/// Import an outside feed.
#[derive(Debug)]
pub struct ImportProducts;

impl Operation for ImportProducts {
    const META: OpMeta = OpMeta {
        address: "integration.product_import.import_products",
        tool: "import_products",
        group: Group::Operate,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/integration/product-imports",
        summary: "Import an outside feed (a seller's or the store's own) in one batch: new products become drafts through catalog's draft path; prices, stock and availability of known products change under catalog's rules. Rows are taken or rejected one by one; a person always approves.",
        consumes: &[],
        produces: &[Slot::one("import_id"), Slot::many("product_id")],
        checks: &[
            "price_floor",
            "import.category_mapping",
            "import.required_fields",
        ],
        errors: &[
            SELLER_NOT_ACTIVE,
            CATEGORY_UNMAPPED,
            BELOW_PRICE_FLOOR,
            BATCH_TOO_LARGE,
        ],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = ImportProductsInput;
    type Output = Value;
}

impl RowError {
    /// The row's error as the whole call's error (`all_or_nothing`, or no row taken).
    fn tool_error(&self, position: i32, external_id: &str) -> ToolError {
        let code = match self.code.as_str() {
            "category_unmapped" => CATEGORY_UNMAPPED,
            "below_price_floor" => BELOW_PRICE_FLOOR,
            "missing_required_field" => MISSING_REQUIRED_FIELD,
            "duplicate_sku" => DUPLICATE_SKU,
            "invalid_state" => ErrorCode::INVALID_STATE,
            "not_found" => ErrorCode::NOT_FOUND,
            _ => ErrorCode::INVALID_INPUT,
        };
        let mut e = ToolError::new(
            code,
            format!("Row {position} ({external_id}): {}", self.message),
        )
        .detail(json!({"position": position, "external_id": external_id}));
        e.next_tool = self.next_tool.clone().map(Into::into);
        e
    }
}

/// A row in the making: its result so far and its changes, kept only if the row is taken.
struct Row {
    result: RowResult,
    prices: Vec<PriceChange>,
    stock: Vec<catalog::StockPlan>,
    availability: Vec<AvailabilityChange>,
    /// The offers the row changes, with the version the preview saw.
    basis: BTreeMap<String, i64>,
}

impl Row {
    fn new(position: i32, external_id: &str) -> Self {
        Self {
            result: RowResult {
                position,
                external_id: external_id.to_owned(),
                product_id: None,
                variants: Vec::new(),
                error: None,
            },
            prices: Vec::new(),
            stock: Vec::new(),
            availability: Vec::new(),
            basis: BTreeMap::new(),
        }
    }

    fn reject(&mut self, e: RowError) {
        self.result.error = Some(e);
        self.result.product_id = None;
        self.result.variants.clear();
        self.prices.clear();
        self.stock.clear();
        self.availability.clear();
        self.basis.clear();
    }
}

fn lower(s: &str) -> String {
    s.trim().to_lowercase()
}

/// The SKU stem of a new product: the feed's code, behind the seller's name for a seller feed
/// (so two sellers' codes never meet).
fn product_code(seller_id: Option<&str>, external_id: &str) -> String {
    match seller_id.and_then(|s| s.strip_prefix("slr_")) {
        Some(seller) => format!("{seller}-{}", external_id.trim()),
        None => external_id.trim().to_owned(),
    }
}

/// Catalog's reader numbers the rows it was given (`Row 2 has no cost`); the feed's code says
/// which row it is.
fn reword(message: &str, external_id: &str) -> String {
    let Some(rest) = message.strip_prefix("Row ") else {
        return message.to_owned();
    };
    let rest = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    format!("{external_id}{rest}")
}

/// The row's error for a catalog draft problem.
fn draft_problem(p: &catalog::DraftProblem, external_id: &str) -> RowError {
    let message = reword(&p.message, external_id);
    match (p.rule, p.field) {
        ("product.required_on_create", "category") => {
            RowError::new(CATEGORY_UNMAPPED, message, Some("list_categories"))
        }
        ("product.sku_unique", _) => RowError::new(DUPLICATE_SKU, message, Some("get_product")),
        ("price_floor", _) => RowError::new(BELOW_PRICE_FLOOR, message, Some("set_price")),
        ("pricing_rule", _) => RowError::new(
            MISSING_REQUIRED_FIELD,
            format!("{external_id} has no cost; the price floor needs it"),
            Some("import_products"),
        ),
        _ => RowError::new(MISSING_REQUIRED_FIELD, message, Some("import_products")),
    }
}

/// What a row of a product the catalog has changes: its list price, stock and availability.
#[allow(clippy::too_many_arguments)]
async fn known_product(
    tx: &mut TxCx,
    row: &mut Row,
    item: &FeedItem,
    mode: FeedImportMode,
    floor: &catalog::PriceFloorRule,
    product_id: &str,
    listed: &[(&FeedVariant, Option<catalog::VariantState>)],
) -> Result<(), ToolError> {
    row.result.product_id = Some(product_id.to_owned());
    let unknown: Vec<&str> = listed
        .iter()
        .filter(|(_, s)| s.is_none())
        .map(|(v, _)| v.external_id.as_str())
        .collect();
    if !unknown.is_empty() {
        row.reject(RowError::new(
            ErrorCode::NOT_FOUND,
            format!(
                "{} {} not a variant of {product_id} yet; add new sizes with set_variants",
                unknown.join(", "),
                if unknown.len() == 1 { "is" } else { "are" }
            ),
            Some("set_variants"),
        ));
        return Ok(());
    }
    for (v, s) in listed {
        if let Some(s) = s {
            row.result
                .variants
                .push((v.external_id.trim().to_owned(), s.variant_id.clone()));
        }
    }
    // Prices: the row's price for its listed variants, or for all of the product's variants.
    if matches!(mode, FeedImportMode::Upsert | FeedImportMode::PriceOnly)
        && let Some(price) = item.price
    {
        let targets: Vec<catalog::VariantState> = if listed.is_empty() {
            catalog::product_variants(tx, &[product_id.to_owned()]).await?
        } else {
            listed.iter().filter_map(|(_, s)| s.clone()).collect()
        };
        let price = price.rounded();
        for s in targets {
            if s.price.currency != price.currency {
                row.reject(RowError::new(
                    ErrorCode::INVALID_INPUT,
                    format!(
                        "{} is sold in {}, not {}",
                        s.variant_id, s.price.currency, price.currency
                    ),
                    Some("import_products"),
                ));
                return Ok(());
            }
            if let Some(cost) = s.cost.or(item.cost)
                && !floor.allows(price, cost)
            {
                row.reject(RowError::new(
                    BELOW_PRICE_FLOOR,
                    format!(
                        "{} at {} is under its floor {} ({})",
                        s.variant_id,
                        money_text(&price),
                        money_text(&floor.floor(cost)),
                        floor.markup_text()
                    ),
                    Some("set_price"),
                ));
                return Ok(());
            }
            if s.price != price {
                row.basis.insert(s.offer_id.clone(), s.offer_version);
                row.prices.push(PriceChange {
                    variant_id: s.variant_id.clone(),
                    offer_id: s.offer_id.clone(),
                    from: s.price,
                    to: price,
                });
            }
        }
    }
    for (v, s) in listed {
        let Some(s) = s else { continue };
        if matches!(mode, FeedImportMode::Upsert | FeedImportMode::StockOnly)
            && let Some(level) = v.inventory_level
            && i64::from(s.inventory_level) != level
        {
            match catalog::plan_stock_level(tx, &s.variant_id, level).await {
                Ok(p) => {
                    row.basis.insert(s.offer_id.clone(), s.offer_version);
                    row.stock.push(p);
                }
                Err(e) => {
                    row.reject(RowError::new(
                        if e.code == "invalid_state" {
                            ErrorCode::INVALID_STATE
                        } else {
                            ErrorCode::INVALID_INPUT
                        },
                        e.message,
                        e.next_tool.as_deref(),
                    ));
                    return Ok(());
                }
            }
        }
        if matches!(mode, FeedImportMode::Upsert | FeedImportMode::StatusOnly)
            && let Some(a) = v.availability
        {
            match catalog::availability_moves(s, a.as_str()) {
                Ok(true) => {
                    row.basis.insert(s.offer_id.clone(), s.offer_version);
                    row.availability.push(AvailabilityChange {
                        variant_id: s.variant_id.clone(),
                        offer_id: s.offer_id.clone(),
                        from: s.availability.clone(),
                        to: a.as_str().to_owned(),
                    });
                }
                Ok(false) => {}
                Err(e) => {
                    row.reject(RowError::new(
                        ErrorCode::INVALID_STATE,
                        e.message,
                        e.next_tool.as_deref(),
                    ));
                    return Ok(());
                }
            }
        }
    }
    Ok(())
}

/// The fields the mode needs that the row leaves out.
fn missing_fields(
    rules: &ImportRules,
    mode: FeedImportMode,
    item: &FeedItem,
    new: bool,
) -> Vec<&'static str> {
    let variants = item.variants.as_deref().unwrap_or_default();
    let mut out = Vec::new();
    match mode {
        FeedImportMode::Upsert => {
            let blank = |v: &Option<String>| v.as_deref().is_none_or(|s| s.trim().is_empty());
            if rules.requires("name") && blank(&item.name) && new {
                out.push("name");
            }
            if rules.requires("price") && item.price.is_none() {
                out.push("price");
            }
            if rules.requires("category") && blank(&item.category) && new {
                out.push("category");
            }
            if rules.requires("size")
                && new
                && (variants.is_empty() || variants.iter().any(|v| blank(&v.size)))
            {
                out.push("size");
            }
        }
        FeedImportMode::PriceOnly => {
            if item.price.is_none() {
                out.push("price");
            }
        }
        FeedImportMode::StockOnly => {
            if variants.is_empty() || variants.iter().any(|v| v.inventory_level.is_none()) {
                out.push("inventory_level");
            }
        }
        FeedImportMode::StatusOnly => {
            if variants.is_empty() || variants.iter().any(|v| v.availability.is_none()) {
                out.push("availability");
            }
        }
    }
    out
}

/// A feed has rows, and no more than `import.max_items_per_batch`.
fn batch_size(rules: &ImportRules, rows: usize) -> Result<(), ToolError> {
    if rows == 0 {
        return Err(ToolError::invalid_input("The feed has no rows.")
            .detail(json!({"field": "items", "reason": "missing"})));
    }
    if rows > rules.max_items {
        return Err(ToolError::new(
            BATCH_TOO_LARGE,
            format!(
                "The feed has {rows} rows; one import takes at most {}. Split it and send each part.",
                rules.max_items
            ),
        )
        .detail(json!({"limit": rules.max_items, "actual": rows,
                       "policy_key": "import.max_items_per_batch"}))
        .next("import_products"));
    }
    Ok(())
}

/// The preview's work, inside one transaction.
pub(crate) async fn plan(
    tx: &mut TxCx,
    input: ImportProductsInput,
) -> Result<Planned<ImportProductsPlan>, ToolError> {
    let rules = ImportRules::from_policy(&tx.policy)?;
    let floor = catalog::PriceFloorRule::from_policy(&tx.policy)?;
    batch_size(&rules, input.items.len())?;
    let seller = input.seller_id.as_deref();
    let mut notes = Vec::new();
    if let Some(s) = seller {
        check_id("seller_id", "slr", s)?;
        match seller_status(tx, s).await? {
            None => notes.push(format!(
                "Seller {s}'s status is not checked: the partner domain does not report it to integration yet"
            )),
            Some(Some(status)) if status == "active" => {}
            Some(found) => {
                let what = found.map_or_else(|| "not a seller of this store".to_owned(), |st| st.replace('_', " "));
                return Err(ToolError::new(SELLER_NOT_ACTIVE, format!("{s} is {what}; its feed cannot be imported."))
                    .detail(json!({"entity": "seller", "id": s}))
                    .next("set_seller_status"));
            }
        }
    }
    let remembered = repo::remembered(tx, seller).await?;
    let standard = catalog::standard_categories(tx).await?;
    let mut words = remembered.categories.clone();
    for w in input.category_mapping.iter().flatten() {
        check_id("category_id", "cat", &w.category_id)?;
        let ok = standard.iter().any(|c| c.id == w.category_id && c.active);
        if !ok {
            return Err(ToolError::invalid_input(format!(
                "{} is not an active standard category; map {} to one.",
                w.category_id, w.external_category
            ))
            .detail(json!({"field": "acom:category_mapping", "reason": "enum"}))
            .next("list_categories"));
        }
        words.insert(lower(&w.external_category), w.category_id.clone());
    }
    // Variants the feed names: by an earlier import's codes, or by the catalog's SKUs.
    let codes: Vec<String> = input
        .items
        .iter()
        .flat_map(|i| i.variants.iter().flatten().map(|v| lower(&v.external_id)))
        .collect();
    let by_sku = catalog::variants_by_sku(tx, &codes).await?;
    let variant_of = |code: &str| -> Option<String> {
        let c = lower(code);
        remembered
            .variants
            .get(&c)
            .or_else(|| by_sku.get(&c))
            .cloned()
    };
    let wanted: Vec<String> = codes
        .iter()
        .filter_map(|c| variant_of(c))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    let states: BTreeMap<String, catalog::VariantState> = catalog::variant_states(tx, &wanted)
        .await?
        .into_iter()
        .map(|s| (s.variant_id.clone(), s))
        .collect();
    let remembered_products: BTreeSet<String> = {
        let ids: Vec<String> = remembered.products.values().cloned().collect();
        catalog::product_names(tx, &ids)
            .await?
            .into_keys()
            .collect()
    };

    let mut rows: Vec<Row> = Vec::new();
    let mut new_products: Vec<(usize, catalog::FeedProduct)> = Vec::new();
    let mut seen = BTreeSet::new();
    for (i, item) in input.items.iter().enumerate() {
        let position = i32::try_from(i + 1).unwrap_or(i32::MAX);
        let mut row = Row::new(position, item.external_id.trim());
        let code = lower(&item.external_id);
        if code.is_empty() {
            row.reject(RowError::new(
                MISSING_REQUIRED_FIELD,
                "The row has no external_id",
                Some("import_products"),
            ));
            rows.push(row);
            continue;
        }
        if !seen.insert(code.clone()) {
            row.reject(RowError::new(
                DUPLICATE_SKU,
                format!("{} is repeated in the feed", item.external_id.trim()),
                Some("import_products"),
            ));
            rows.push(row);
            continue;
        }
        let variants = item.variants.as_deref().unwrap_or_default();
        let listed: Vec<(&FeedVariant, Option<catalog::VariantState>)> = variants
            .iter()
            .map(|v| {
                (
                    v,
                    variant_of(&v.external_id).and_then(|id| states.get(&id).cloned()),
                )
            })
            .collect();
        let owners: BTreeSet<&str> = listed
            .iter()
            .filter_map(|(_, s)| s.as_ref().map(|s| s.product_id.as_str()))
            .collect();
        let product = remembered
            .products
            .get(&code)
            .filter(|p| remembered_products.contains(*p))
            .cloned()
            .or_else(|| {
                (owners.len() == 1)
                    .then(|| owners.iter().next().map(|p| (*p).to_owned()))
                    .flatten()
            });
        if owners.len() > 1
            || product
                .as_deref()
                .is_some_and(|p| owners.iter().any(|o| *o != p))
        {
            row.reject(RowError::new(
                ErrorCode::INVALID_INPUT,
                format!(
                    "The variants of {} belong to different products",
                    item.external_id.trim()
                ),
                Some("get_product"),
            ));
            rows.push(row);
            continue;
        }
        let missing = missing_fields(&rules, input.mode, item, product.is_none());
        if !missing.is_empty() {
            row.reject(RowError::new(
                MISSING_REQUIRED_FIELD,
                format!("{} has no {}", item.external_id.trim(), missing.join(", ")),
                Some("import_products"),
            ));
            rows.push(row);
            continue;
        }
        match product {
            Some(p) => known_product(tx, &mut row, item, input.mode, &floor, &p, &listed).await?,
            None if input.mode != FeedImportMode::Upsert => {
                row.reject(RowError::new(
                    ErrorCode::NOT_FOUND,
                    format!(
                        "{} is not in the catalog yet; import it with acom:import_mode upsert first",
                        item.external_id.trim()
                    ),
                    Some("import_products"),
                ));
            }
            None => {
                let colors: BTreeSet<String> = variants
                    .iter()
                    .filter_map(|v| {
                        v.color
                            .as_deref()
                            .map(str::trim)
                            .filter(|c| !c.is_empty())
                            .map(str::to_owned)
                    })
                    .collect();
                if colors.len() > 1 {
                    row.reject(RowError::new(
                        ErrorCode::INVALID_INPUT,
                        format!(
                            "{} has variants in {} colors; send one row per color",
                            item.external_id.trim(),
                            colors.len()
                        ),
                        Some("import_products"),
                    ));
                } else {
                    let word = item.category.as_deref().unwrap_or_default().trim();
                    let category = words
                        .get(&lower(word))
                        .and_then(|id| standard.iter().find(|c| c.id == *id && c.active))
                        .map_or_else(|| word.to_owned(), |c| c.name.clone());
                    new_products.push((
                        rows.len(),
                        catalog::FeedProduct {
                            code: product_code(seller, &item.external_id),
                            name: item.name.as_deref().unwrap_or_default().trim().to_owned(),
                            category,
                            color: colors.into_iter().next(),
                            sizes: variants
                                .iter()
                                .filter_map(|v| v.size.as_deref().map(str::trim).map(str::to_owned))
                                .collect(),
                            cost: item.cost,
                            // Present: upsert requires it (`import.required_fields`).
                            price: item.price.unwrap_or_else(|| Money::zero(tx.currency)),
                        },
                    ));
                }
            }
        }
        rows.push(row);
    }

    // New products, read as catalog drafts.
    let products: Vec<catalog::FeedProduct> = new_products.iter().map(|(_, p)| p.clone()).collect();
    let answers = catalog::read_feed_products(tx, &products).await?;
    let mut drafts = Vec::new();
    let mut stock_units = Vec::new();
    let mut mapping = BTreeMap::new();
    for ((at, _), answer) in new_products.iter().zip(answers) {
        let row = &mut rows[*at];
        let item = &input.items[*at];
        match answer {
            Err(p) => {
                let mut e = draft_problem(&p, item.external_id.trim());
                if e.code == CATEGORY_UNMAPPED.code {
                    e.message = format!(
                        "No category matches '{}'",
                        item.category.as_deref().unwrap_or_default().trim()
                    );
                }
                row.reject(e);
            }
            Ok(d) => {
                row.result.product_id = Some(d.product_id.clone());
                for v in item.variants.iter().flatten() {
                    let size = v.size.as_deref().map(lower);
                    if let Some(dv) = d
                        .variants
                        .iter()
                        .find(|dv| dv.size.as_deref().map(lower) == size)
                    {
                        row.result
                            .variants
                            .push((v.external_id.trim().to_owned(), dv.variant_id.clone()));
                        let units = v.inventory_level.unwrap_or(0).clamp(0, i64::from(i32::MAX));
                        stock_units.push((
                            dv.variant_id.clone(),
                            i32::try_from(units).unwrap_or(i32::MAX),
                        ));
                    }
                }
                if let Some(word) = item.category.as_deref() {
                    mapping.insert(lower(word), d.category_id.clone());
                }
                drafts.push(d);
            }
        }
    }
    for w in input.category_mapping.iter().flatten() {
        mapping
            .entry(lower(&w.external_category))
            .or_insert_with(|| w.category_id.clone());
    }

    let rejected: Vec<&Row> = rows.iter().filter(|r| !r.result.accepted()).collect();
    if let Some(first) = rejected.first()
        && (rules.all_or_nothing || rejected.len() == rows.len())
    {
        let e = first
            .result
            .error
            .as_ref()
            .map(|e| e.tool_error(first.result.position, &first.result.external_id));
        if let Some(mut e) = e {
            if rules.all_or_nothing {
                e.message = format!(
                    "{} The store takes a feed whole or not at all (import.partial_success).",
                    e.message
                );
            } else {
                e.message = format!("{} No row of the feed can be taken.", e.message);
            }
            e.detail = Some(Box::new(json!({
                "position": first.result.position, "external_id": first.result.external_id,
                "rows": rejected.iter().map(|r| json!({"position": r.result.position,
                    "external_id": r.result.external_id,
                    "error": r.result.error.as_ref().map(RowError::json)})).collect::<Vec<_>>()})));
            return Err(e);
        }
    }

    let suffix = acom_core::id::new_suffix();
    let import_id = format!("imp_{suffix}");
    let mut plan = ImportProductsPlan {
        import_id: import_id.clone(),
        seller_id: input.seller_id.clone(),
        mode: input.mode,
        category_mapping: mapping,
        rows: Vec::new(),
        drafts: catalog::FeedDrafts {
            import_id: import_id.clone(),
            upload_id: format!("upl_{suffix}"),
            file_name: format!("{} feed via import_products", seller.unwrap_or("store")),
            provider_name: input.seller_id.clone(),
            drafts,
            stock: stock_units,
        },
        prices: Vec::new(),
        stock: Vec::new(),
        availability: Vec::new(),
    };
    let mut basis = BTreeMap::new();
    for r in rows {
        basis.extend(r.basis);
        plan.prices.extend(r.prices);
        plan.stock.extend(r.stock);
        plan.availability.extend(r.availability);
        plan.rows.push(r.result);
    }
    let basis = basis
        .into_iter()
        .map(|(id, version)| Basis {
            entity: "catalog.offer".into(),
            id,
            version,
        })
        .collect();
    Ok(preview_of(plan, &floor, &rules, notes, basis))
}

/// The preview's changes, checks, basis and approval reason for a plan.
fn preview_of(
    plan: ImportProductsPlan,
    floor: &catalog::PriceFloorRule,
    rules: &ImportRules,
    notes: Vec<String>,
    basis: Vec<Basis>,
) -> Planned<ImportProductsPlan> {
    let errors = |code: &str| -> Vec<&RowResult> {
        plan.rows
            .iter()
            .filter(|r| r.error.as_ref().is_some_and(|e| e.code == code))
            .collect()
    };
    let accepted = plan.rows.iter().filter(|r| r.accepted()).count();
    let rejected = plan.rows.len() - accepted;
    let created = plan.drafts.drafts.len();
    let mut changes = vec![Change {
        op: ChangeOp::Create,
        entity: "product_import".into(),
        id: String::new(),
        from: Value::Null,
        to: json!({"seller_id": plan.seller_id, "acom:import_mode": plan.mode,
                   "rows": plan.rows.len(), "accepted": accepted, "rejected": rejected,
                   "acom:category_mapping": plan.category_mapping}),
        note: Some(
            std::iter::once(
                "New products are drafts, not visible to shoppers; publishing is approve_product."
                    .to_owned(),
            )
            .chain(notes)
            .collect::<Vec<_>>()
            .join(" "),
        ),
    }];
    for d in &plan.drafts.drafts {
        let mut to = catalog::draft_json(d);
        if let Some(r) = plan
            .rows
            .iter()
            .find(|r| r.product_id.as_deref() == Some(d.product_id.as_str()))
        {
            to["external_id"] = json!(r.external_id);
        }
        changes.push(Change {
            op: ChangeOp::Create,
            entity: "product".into(),
            id: String::new(),
            from: Value::Null,
            to,
            note: d.note.clone(),
        });
    }
    for p in &plan.prices {
        changes.push(Change {
            op: ChangeOp::Update,
            entity: "offer".into(),
            id: p.offer_id.clone(),
            from: json!({"variant_id": p.variant_id, "price": p.from.value, "price_currency": p.from.currency.to_string()}),
            to: json!({"variant_id": p.variant_id, "price": p.to.value, "price_currency": p.to.currency.to_string()}),
            note: None,
        });
    }
    for s in &plan.stock {
        changes.push(Change {
            op: ChangeOp::Update,
            entity: "inventory_level".into(),
            id: s.variant_id.clone(),
            from: json!({"inventory_level": s.from.inventory_level, "availability": s.from.availability}),
            to: json!({"inventory_level": s.to.inventory_level, "availability": s.to.availability}),
            note: None,
        });
    }
    for a in &plan.availability {
        changes.push(Change {
            op: ChangeOp::Update,
            entity: "variant".into(),
            id: a.variant_id.clone(),
            from: json!({"availability": a.from}),
            to: json!({"availability": a.to}),
            note: (a.to == "discontinued").then(|| "Discontinuing cannot be undone.".to_owned()),
        });
    }
    for r in plan.rows.iter().filter(|r| !r.accepted()) {
        changes.push(Change {
            op: ChangeOp::Exclude,
            entity: "product".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({"position": r.position, "external_id": r.external_id,
                       "error": r.error.as_ref().map(RowError::json)}),
            note: r.error.as_ref().map(|e| e.message.clone()),
        });
    }

    let sample = |rows: &[&RowResult]| -> String {
        let mut parts: Vec<String> = rows.iter().take(3).map(|r| r.external_id.clone()).collect();
        if rows.len() > 3 {
            parts.push(format!("{} more", rows.len() - 3));
        }
        parts.join(", ")
    };
    let below = errors(BELOW_PRICE_FLOOR.code);
    let priced = plan.prices.len() + created + below.len();
    let price_floor = Check {
        rule: "price_floor".into(),
        passed: below.is_empty(),
        detail: Some(
            if !matches!(
                plan.mode,
                FeedImportMode::Upsert | FeedImportMode::PriceOnly
            ) {
                "No prices change".to_owned()
            } else if below.is_empty() {
                format!("{priced} of {priced} at or above {}", floor.markup_text())
            } else {
                format!(
                    "{} rows under {}: {}",
                    below.len(),
                    floor.markup_text(),
                    sample(&below)
                )
            },
        ),
        fix_tool: (!below.is_empty()).then(|| "set_price".into()),
    };
    let unmapped = errors(CATEGORY_UNMAPPED.code);
    let category = Check {
        rule: "import.category_mapping".into(),
        passed: unmapped.is_empty(),
        detail: Some(if unmapped.is_empty() {
            if created == 0 {
                "No new product needs a category".into()
            } else {
                format!("All {created} new products map to a standard category")
            }
        } else {
            format!(
                "{} row{} {} no matching category: {}",
                unmapped.len(),
                if unmapped.len() == 1 { "" } else { "s" },
                if unmapped.len() == 1 { "has" } else { "have" },
                sample(&unmapped)
            )
        }),
        fix_tool: (!unmapped.is_empty()).then(|| "import_products".into()),
    };
    let missing = errors(MISSING_REQUIRED_FIELD.code);
    let required = Check {
        rule: "import.required_fields".into(),
        passed: missing.is_empty(),
        detail: Some(if missing.is_empty() {
            format!("All rows have {}", rules.required_fields.join(", "))
        } else {
            format!(
                "{} row{} miss{} a required field: {}",
                missing.len(),
                if missing.len() == 1 { "" } else { "s" },
                if missing.len() == 1 { "es" } else { "" },
                sample(&missing)
            )
        }),
        fix_tool: (!missing.is_empty()).then(|| "import_products".into()),
    };

    let mut said = Vec::new();
    if created > 0 {
        said.push(format!(
            "Creates {created} product{}",
            if created == 1 { "" } else { "s" }
        ));
    }
    if !plan.prices.is_empty() {
        said.push(format!(
            "changes {} price{}",
            plan.prices.len(),
            if plan.prices.len() == 1 { "" } else { "s" }
        ));
    }
    if !plan.stock.is_empty() {
        said.push(format!(
            "sets stock of {} variant{}",
            plan.stock.len(),
            if plan.stock.len() == 1 { "" } else { "s" }
        ));
    }
    if !plan.availability.is_empty() {
        said.push(format!(
            "changes availability of {} variant{}",
            plan.availability.len(),
            if plan.availability.len() == 1 {
                ""
            } else {
                "s"
            }
        ));
    }
    if said.is_empty() {
        said.push("Changes nothing in the catalog".into());
    }
    let mut reason = said.join(", ");
    if let Some(first) = reason.get(..1) {
        reason = format!("{}{}", first.to_uppercase(), &reason[1..]);
    }
    if rejected > 0 {
        reason.push_str(&format!(
            "; rejects {rejected} row{}",
            if rejected == 1 { "" } else { "s" }
        ));
    }

    Planned {
        changes,
        checks: vec![price_floor, category, required],
        basis,
        approval: ApprovalNeed::Needed { reason },
        plan,
    }
}

impl WriteOp<Cx> for ImportProducts {
    type Plan = ImportProductsPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ImportProductsInput,
    ) -> Result<Planned<ImportProductsPlan>, ToolError> {
        batch_size(&ImportRules::from_policy(&cx.policy)?, input.items.len())?;
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let planned = plan(&mut tx, input).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(planned)
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &ImportProductsPlan,
    ) -> Result<Committed, ToolError> {
        let product_ids: Vec<String> = plan
            .rows
            .iter()
            .filter_map(|r| r.product_id.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut produced = Map::new();
        produced.insert("import_id".into(), json!(plan.import_id));
        produced.insert("product_id".into(), json!(product_ids));
        if repo::import(tx, &plan.import_id).await?.is_some() {
            return Ok(Committed {
                id: plan.import_id.clone(),
                produced,
            });
        }
        catalog::save_feed_drafts(tx, &plan.drafts).await?;
        for p in &plan.prices {
            catalog::set_list_price(tx, &p.offer_id, p.to).await?;
        }
        for s in &plan.stock {
            catalog::write_stock(tx, s, false).await?;
        }
        for a in &plan.availability {
            catalog::set_availability(tx, &a.offer_id, &a.to).await?;
        }
        repo::insert_import(
            tx,
            &NewImport {
                id: &plan.import_id,
                seller_id: plan.seller_id.as_deref(),
                import_mode: plan.mode.as_str(),
                category_mapping: &plan.category_mapping,
                at: tx.clock.now(),
            },
        )
        .await?;
        for r in &plan.rows {
            repo::insert_import_item(
                tx,
                &plan.import_id,
                &NewImportItem {
                    position: r.position,
                    external_id: &r.external_id,
                    product_group_id: r.product_id.as_deref(),
                    item_status: if r.accepted() { "accepted" } else { "rejected" },
                    variant_mapping: r.variant_mapping(),
                    error: r.error.as_ref().map(RowError::json),
                },
            )
            .await?;
        }
        Ok(Committed {
            id: plan.import_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ImportProductsPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut out = catalog::verify_feed_drafts(cx, &plan.drafts).await?;
        let ids: Vec<String> = plan
            .prices
            .iter()
            .map(|p| p.variant_id.clone())
            .chain(plan.stock.iter().map(|s| s.variant_id.clone()))
            .chain(plan.availability.iter().map(|a| a.variant_id.clone()))
            .collect();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let states: BTreeMap<String, catalog::VariantState> =
            catalog::variant_states(&mut tx, &ids)
                .await?
                .into_iter()
                .map(|s| (s.variant_id.clone(), s))
                .collect();
        let recorded = repo::import(&mut tx, &plan.import_id).await?.is_some();
        tx.commit().await.map_err(ToolError::from)?;
        if !recorded {
            out.mismatches
                .push(format!("{} was not recorded", plan.import_id));
        }
        for p in &plan.prices {
            if states.get(&p.variant_id).is_none_or(|s| s.price != p.to) {
                out.mismatches.push(format!(
                    "{} is not priced {}",
                    p.variant_id,
                    money_text(&p.to)
                ));
            }
        }
        for s in &plan.stock {
            if states
                .get(&s.variant_id)
                .is_none_or(|v| v.inventory_level != s.to.inventory_level)
            {
                out.mismatches.push(format!(
                    "{} does not have {} units",
                    s.variant_id, s.to.inventory_level
                ));
            }
        }
        for a in &plan.availability {
            if states
                .get(&a.variant_id)
                .is_none_or(|v| v.availability != a.to)
            {
                out.mismatches
                    .push(format!("{} is not {}", a.variant_id, a.to));
            }
        }
        out.matched = out.mismatches.is_empty();
        Ok(out)
    }

    /// Undoing puts back the prices, stock and availability and takes the drafts out, while
    /// none of them was submitted or published; a discontinued variant does not come back.
    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &ImportProductsPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        let gone: Vec<&str> = plan
            .availability
            .iter()
            .filter(|a| a.to == "discontinued")
            .map(|a| a.variant_id.as_str())
            .collect();
        if !gone.is_empty() {
            return Err(ToolError::new(
                ErrorCode::UNDO_WINDOW_CLOSED,
                format!(
                    "The import discontinued {}; discontinuing cannot be taken back.",
                    gone.join(", ")
                ),
            )
            .detail(json!({"entity": "variant", "ids": gone})));
        }
        for a in &plan.availability {
            catalog::set_availability(tx, &a.offer_id, &a.from).await?;
        }
        for s in &plan.stock {
            catalog::write_stock(tx, s, true).await?;
        }
        for p in &plan.prices {
            catalog::set_list_price(tx, &p.offer_id, p.from).await?;
        }
        catalog::remove_feed_drafts(tx, &plan.drafts).await?;
        repo::delete_import(tx, &plan.import_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    fn input(v: Value) -> ImportProductsInput {
        serde_json::from_value(v).unwrap()
    }

    fn item(v: Value) -> FeedItem {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn the_input_is_closed_with_word_modes() {
        let s = closed_schema_for::<ImportProductsInput>();
        assert!(violations(&s).is_empty(), "{s:#}");
        assert_eq!(
            s["properties"]["acom:import_mode"]["enum"],
            json!(["upsert", "price_only", "stock_only", "status_only"])
        );
        assert_eq!(s["required"], json!(["acom:import_mode", "items"]));
        assert_eq!(ImportProducts::META.kind, Kind::WriteApproval);
    }

    #[test]
    fn catalogs_row_numbers_become_the_feeds_codes() {
        assert_eq!(reword("Row 2 has no SKU", "SKU-9"), "SKU-9 has no SKU");
        assert_eq!(
            reword("Row 12: no category matches \"x\"", "SKU-9"),
            "SKU-9: no category matches \"x\""
        );
        assert_eq!(
            reword("SKU X is repeated in the sheet", "SKU-9"),
            "SKU X is repeated in the sheet"
        );
    }

    #[test]
    fn seller_codes_never_meet_the_stores_own() {
        assert_eq!(product_code(None, " SKU-1182 "), "SKU-1182");
        assert_eq!(
            product_code(Some("slr_harbor_goods"), "SKU-1182"),
            "harbor_goods-SKU-1182"
        );
    }

    #[test]
    fn each_mode_asks_for_its_own_fields() {
        let rules = ImportRules::from_policy(&crate::testing::policies()).unwrap();
        let bare = item(json!({"external_id": "SKU-1"}));
        assert_eq!(
            missing_fields(&rules, FeedImportMode::Upsert, &bare, true),
            ["name", "price", "category", "size"]
        );
        assert_eq!(
            missing_fields(&rules, FeedImportMode::Upsert, &bare, false),
            ["price"]
        );
        assert_eq!(
            missing_fields(&rules, FeedImportMode::PriceOnly, &bare, false),
            ["price"]
        );
        let stocked = item(
            json!({"external_id": "SKU-1", "variants": [{"external_id": "SKU-1-M", "inventory_level": 3}]}),
        );
        assert!(missing_fields(&rules, FeedImportMode::StockOnly, &stocked, false).is_empty());
        assert_eq!(
            missing_fields(&rules, FeedImportMode::StatusOnly, &stocked, false),
            ["availability"]
        );
    }

    #[tokio::test]
    async fn a_batch_over_the_limit_is_refused_before_any_query() {
        let rows: Vec<Value> = (0..501)
            .map(|i| json!({"external_id": format!("SKU-{i}")}))
            .collect();
        let e = ImportProducts
            .preview(
                &crate::testing::offline("adm_sj"),
                input(json!({"acom:import_mode": "price_only", "items": rows})),
            )
            .await
            .unwrap_err();
        assert_eq!(
            (e.code.as_ref(), e.next_tool.as_deref()),
            ("batch_too_large", Some("import_products"))
        );
        assert_eq!(e.detail.unwrap()["limit"], 500);
    }

    #[tokio::test]
    #[ignore = "integration: needs the demo database (DATABASE_URL)"]
    async fn imports_through_catalogs_draft_path_row_by_row_and_undoes() {
        let cx = crate::testing::db("adm_sj").await;
        let mut tx = cx.begin().await.unwrap();
        let feed = input(json!({
        "acom:import_mode": "upsert",
        "items": [
            {"external_id": "SKU-9001", "name": "Trail running cap", "category": "Hats",
             "price": {"value": 34.00, "currency": "USD"}, "cost": {"value": 12.00, "currency": "USD"},
             "variants": [{"external_id": "SKU-9001-S", "size": "S", "color": "Black", "inventory_level": 10},
                          {"external_id": "SKU-9001-M", "size": "M", "color": "Black", "inventory_level": 0}]},
            {"external_id": "MT-CHARCOAL", "price": {"value": 60.00, "currency": "USD"},
             "variants": [{"external_id": "MT-CHARCOAL-M", "size": "M", "inventory_level": 20}]},
            {"external_id": "SKU-9003", "name": "Trail tee", "category": "Trail tops",
             "price": {"value": 40.00, "currency": "USD"}, "cost": {"value": 15.00, "currency": "USD"},
             "variants": [{"external_id": "SKU-9003-M", "size": "M", "color": "Sage"}]},
            {"external_id": "SKU-9004", "name": "Bargain tee", "category": "Tees",
             "price": {"value": 10.00, "currency": "USD"}, "cost": {"value": 20.00, "currency": "USD"},
             "variants": [{"external_id": "SKU-9004-M", "size": "M", "color": "Red"}]}
        ]}));
        let p = plan(&mut tx, feed).await.unwrap();
        let failed: Vec<&str> = p
            .checks
            .iter()
            .filter(|c| !c.passed)
            .map(|c| c.rule.as_str())
            .collect();
        assert_eq!(
            failed,
            ["price_floor", "import.category_mapping"],
            "{:#?}",
            p.checks
        );
        assert!(
            matches!(&p.approval, ApprovalNeed::Needed { reason } if reason.starts_with("Creates 1 product")),
            "{:?}",
            p.approval
        );
        let codes: Vec<Option<&str>> = p
            .plan
            .rows
            .iter()
            .map(|r| r.error.as_ref().map(|e| e.code.as_str()))
            .collect();
        assert_eq!(
            codes,
            [
                None,
                None,
                Some("category_unmapped"),
                Some("below_price_floor")
            ]
        );
        assert_eq!(p.plan.rows[0].product_id.as_deref(), Some("prd_sku_9001"));
        assert_eq!(p.plan.rows[1].product_id.as_deref(), Some("prd_merino_tee"));
        assert_eq!(p.plan.prices.len(), 1, "only the listed size changes price");
        assert_eq!(p.plan.stock.len(), 1);
        assert_eq!(p.basis.len(), 1);

        let done = ImportProducts.commit(&mut tx, &p.plan).await.unwrap();
        assert_eq!(
            done.produced["product_id"],
            json!(["prd_merino_tee", "prd_sku_9001"])
        );
        let items = repo::import_items(&mut tx, &p.plan.import_id)
            .await
            .unwrap();
        assert_eq!(
            items.iter().filter(|i| i.item_status == "accepted").count(),
            2
        );
        assert_eq!(
            items[2].error.as_ref().unwrap()["next_tool"],
            "list_categories"
        );
        let states = catalog::variant_states(
            &mut tx,
            &[
                "var_merino_tee_m_charcoal".to_owned(),
                "var_sku_9001_s".to_owned(),
            ],
        )
        .await
        .unwrap();
        assert_eq!(states[0].price.value.to_string(), "60.00");
        assert_eq!(states[0].inventory_level, 20);
        assert_eq!(
            (states[1].inventory_level, states[1].availability.as_str()),
            (10, "in_stock")
        );

        // A second run of the same feed knows the cap by its code now.
        let again = plan(&mut tx, input(json!({"acom:import_mode": "stock_only", "items": [
            {"external_id": "SKU-9001", "variants": [{"external_id": "SKU-9001-S", "inventory_level": 7}]}]})))
            .await
            .unwrap();
        assert_eq!(
            again.plan.rows[0].product_id.as_deref(),
            Some("prd_sku_9001")
        );
        assert_eq!(again.plan.stock[0].to.inventory_level, 7);

        ImportProducts.undo(&mut tx, &p.plan, &done).await.unwrap();
        let back = catalog::variant_states(
            &mut tx,
            &[
                "var_merino_tee_m_charcoal".to_owned(),
                "var_sku_9001_s".to_owned(),
            ],
        )
        .await
        .unwrap();
        assert_eq!(back.len(), 1, "the draft is gone");
        assert_eq!(back[0].price.value.to_string(), "58.00");
        assert!(
            repo::import(&mut tx, &p.plan.import_id)
                .await
                .unwrap()
                .is_none()
        );
        drop(tx);
    }
}
