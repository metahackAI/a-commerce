//! Reading a supplier sheet into product drafts (`batch_import`, scene B4), as pure functions.
//!
//! A sheet is a list of rows, each a map from column name to cell text
//! (`catalog.data_feed.data_feed_element`). Each row becomes one draft product with one variant
//! per size. Everything a draft gets is decided here from the row and the store's rules:
//! - the name from the style and the color (`import.name_format: style_color`),
//! - the category by matching the sheet's word to a standard category (`import.category_match`),
//! - the list price from the cost (`pricing_rule`), checked against the floor (`price_floor`),
//! - the open fields: required fields (`import.required_fields`) that are missing, or a photo
//!   that two rows share (`import.one_product_per_image`).
//!
//! The description is not written here. A draft keeps the supplier's text and gets its
//! description later in the store's tone (`content.copy`); publishing requires one
//! (`product.publish_requires: description`).

use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;

use acom_contract::{Check, ToolError};
use acom_core::enums::DraftStatus;
use acom_core::{Currency, Money};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::{money_text, num};
use crate::neighbors::CategoryTerm;
use crate::rules::{ImportRules, PriceFloorRule, PricingRule};
use crate::{INVALID_FILE_FORMAT, slug};

/// Columns every sheet must have (in at least one row).
pub const REQUIRED_COLUMNS: [&str; 5] = ["supplier_code", "sku", "style_name", "category", "cost"];

/// What the reader needs besides the rows.
#[derive(Debug, Clone)]
pub struct Lookup<'a> {
    pub rules: &'a ImportRules,
    pub pricing: &'a PricingRule,
    pub floor: &'a PriceFloorRule,
    pub categories: &'a [CategoryTerm],
    /// `(id, name)` of every brand.
    pub brands: &'a [(String, String)],
    /// Variant SKUs already in the catalog, lowercase, with the product they belong to.
    pub taken_skus: &'a BTreeMap<String, String>,
    /// Products that already exist, with their approval status.
    pub existing: &'a BTreeMap<String, String>,
    /// Products an earlier import made that are still drafts (`upsert` refreshes these).
    pub refreshable: &'a BTreeSet<String>,
    pub upsert: bool,
    pub currency: Currency,
}

/// One variant a draft will have.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftVariant {
    pub variant_id: String,
    pub offer_id: String,
    pub sku: String,
    pub size: Option<String>,
    pub name: String,
}

/// One row read into a draft.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Draft {
    /// Row number in the sheet, from 1.
    pub position: u32,
    pub supplier_code: String,
    pub supplier_text: Option<String>,
    pub product_id: String,
    pub sku: String,
    pub name: String,
    pub category_id: String,
    pub category_name: String,
    pub brand_id: Option<String>,
    pub color: Option<String>,
    pub material: Option<String>,
    pub image: Option<String>,
    pub sizes: Vec<String>,
    pub units_per_size: i32,
    pub cost: Money,
    pub price: Money,
    pub floor: Money,
    pub variants: Vec<DraftVariant>,
    pub status: DraftStatus,
    pub open_fields: Vec<String>,
    /// Why each open field is open, in a few words.
    #[serde(default)]
    pub open_labels: Vec<String>,
    pub note: Option<String>,
    /// `upsert`: the product is a draft an earlier import made; refresh it instead of creating.
    #[serde(default)]
    pub refresh: bool,
}

/// A row that cannot become a draft, and the rule it breaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowProblem {
    pub position: u32,
    pub rule: &'static str,
    pub field: &'static str,
    pub message: String,
}

/// Every row read: the drafts, and the rows that cannot be drafts.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    pub drafts: Vec<Draft>,
    pub problems: Vec<RowProblem>,
    pub rows: usize,
}

impl Reading {
    pub fn count(&self, status: DraftStatus) -> usize {
        self.drafts.iter().filter(|d| d.status == status).count()
    }
}

fn cell<'a>(row: &'a Value, column: &str) -> Option<&'a str> {
    row.get(column)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

fn amount(text: &str) -> Option<Decimal> {
    let t = text.trim().trim_start_matches('$').replace(',', "");
    Decimal::from_str(&t).ok()
}

/// "Wool blend overcoat" + "Charcoal" → "Wool blend overcoat, charcoal".
pub fn draft_name(style: &str, color: Option<&str>) -> String {
    match color {
        Some(c) => format!("{style}, {}", c.to_lowercase()),
        None => style.to_owned(),
    }
}

/// The standard category a sheet's word names, by name or term code (case and spaces ignored).
pub fn match_category<'a>(
    word: &str,
    categories: &'a [CategoryTerm],
    by: &[String],
) -> Option<&'a CategoryTerm> {
    let code = slug(word);
    categories.iter().find(|c| {
        c.term_set == "standard"
            && c.active
            && ((by.iter().any(|b| b == "name") && c.name.eq_ignore_ascii_case(word.trim()))
                || (by.iter().any(|b| b == "term_code") && c.term_code == code))
    })
}

/// Why an open field is open, in a few words (the check's summary counts these).
fn open_label(field: &str, shared: bool) -> String {
    match (field, shared) {
        ("image", true) => "photo shows more than one item".into(),
        ("image", false) => "no photo".into(),
        ("material", _) => "fabric mix missing".into(),
        (other, _) => format!("no {}", other.replace('_', " ")),
    }
}

/// Why an open field is open, as one sentence for the person.
fn open_note(field: &str, shared: bool) -> String {
    match (field, shared) {
        ("image", true) => "The photo shows more than one item; pick one".into(),
        ("image", false) => "No photo in the sheet".into(),
        ("material", _) => "Fabric mix is missing in the sheet".into(),
        (other, _) => format!("No {} in the sheet", other.replace('_', " ")),
    }
}

/// Reads every row. Fails only when the sheet itself cannot be read (no rows, too many, or a
/// required column missing everywhere); row problems come back in [`Reading::problems`].
pub fn read(rows: &[Value], l: &Lookup<'_>) -> Result<Reading, ToolError> {
    if rows.is_empty() {
        return Err(
            ToolError::new(INVALID_FILE_FORMAT, "The sheet has no rows.")
                .detail(json!({"rows": 0}))
                .next("batch_import"),
        );
    }
    if rows.len() > l.rules.max_rows {
        return Err(ToolError::new(
            INVALID_FILE_FORMAT,
            format!(
                "The sheet has {} rows; one import takes at most {}. Split it and import each part.",
                rows.len(),
                l.rules.max_rows
            ),
        )
        .detail(json!({"rule": "import.max_rows", "rows": rows.len(), "max_rows": l.rules.max_rows}))
        .next("batch_import"));
    }
    let missing: Vec<&str> = REQUIRED_COLUMNS
        .iter()
        .copied()
        .filter(|c| rows.iter().all(|r| r.get(*c).is_none()))
        .collect();
    if !missing.is_empty() {
        return Err(ToolError::new(
            INVALID_FILE_FORMAT,
            format!("The sheet has no {} column.", missing.join(", ")),
        )
        .detail(json!({"missing_columns": missing, "required_columns": REQUIRED_COLUMNS}))
        .next("batch_import"));
    }

    // How many rows use each photo, for `one_product_per_image`.
    let mut photo_rows: BTreeMap<&str, usize> = BTreeMap::new();
    for r in rows {
        if let Some(img) = cell(r, "image") {
            *photo_rows.entry(img).or_default() += 1;
        }
    }
    let mut sheet_skus: BTreeSet<String> = BTreeSet::new();
    let mut out = Reading {
        drafts: Vec::new(),
        problems: Vec::new(),
        rows: rows.len(),
    };
    for (i, r) in rows.iter().enumerate() {
        let position = u32::try_from(i + 1).unwrap_or(u32::MAX);
        let mut problem = |rule: &'static str, field: &'static str, message: String| {
            out.problems.push(RowProblem {
                position,
                rule,
                field,
                message,
            });
        };
        let code = cell(r, "supplier_code").unwrap_or_default().to_owned();
        let color = cell(r, "color");
        let Some(style) = cell(r, "style_name") else {
            problem(
                "product.required_on_create",
                "name",
                format!("Row {position} has no style name"),
            );
            continue;
        };
        let name = draft_name(style, color);
        let category = match cell(r, "category") {
            Some(word) => match match_category(word, l.categories, &l.rules.category_match) {
                Some(c) => c,
                None => {
                    problem(
                        "product.required_on_create",
                        "category",
                        format!("Row {position}: no category matches \"{word}\""),
                    );
                    continue;
                }
            },
            None => {
                problem(
                    "product.required_on_create",
                    "category",
                    format!("Row {position} has no category"),
                );
                continue;
            }
        };
        let Some(sku) = cell(r, "sku").map(str::to_lowercase) else {
            problem(
                "product.sku_unique",
                "sku",
                format!("Row {position} has no SKU"),
            );
            continue;
        };
        let Some(cost_value) = cell(r, "cost")
            .and_then(amount)
            .filter(|c| *c > Decimal::ZERO)
        else {
            problem(
                "pricing_rule",
                "cost",
                format!("Row {position} has no cost, so no price can be set"),
            );
            continue;
        };
        let product_id = format!("prd_{}", slug(&sku));
        let refresh = l.upsert && l.refreshable.contains(&product_id);
        if !refresh && let Some(status) = l.existing.get(&product_id) {
            problem(
                "product.sku_unique",
                "sku",
                format!(
                    "{} is already in the catalog as {product_id} ({})",
                    sku.to_uppercase(),
                    status.replace('_', " ")
                ),
            );
            continue;
        }
        let sizes: Vec<String> = cell(r, "sizes")
            .map(|s| {
                s.split(',')
                    .map(str::trim)
                    .filter(|x| !x.is_empty())
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let mut variants = Vec::new();
        let mut duplicate = None;
        let size_list: Vec<Option<&str>> = if sizes.is_empty() {
            vec![None]
        } else {
            sizes.iter().map(|s| Some(s.as_str())).collect()
        };
        for size in size_list {
            let vsku = match size {
                Some(s) => format!("{sku}-{}", s.to_lowercase()),
                None => sku.clone(),
            };
            let taken_by_other = l
                .taken_skus
                .get(&vsku)
                .is_some_and(|owner| !(refresh && *owner == product_id));
            if taken_by_other || !sheet_skus.insert(vsku.clone()) {
                duplicate = Some(vsku);
                break;
            }
            let suffix = slug(&vsku);
            variants.push(DraftVariant {
                variant_id: format!("var_{suffix}"),
                offer_id: format!("off_{suffix}"),
                sku: vsku,
                size: size.map(str::to_owned),
                name: match size {
                    Some(s) => format!("{name}, {s}"),
                    None => name.clone(),
                },
            });
        }
        if let Some(d) = duplicate {
            problem(
                "product.sku_unique",
                "sku",
                format!(
                    "SKU {} is {}",
                    d.to_uppercase(),
                    if l.taken_skus.contains_key(&d) {
                        "already in the catalog"
                    } else {
                        "repeated in the sheet"
                    }
                ),
            );
            continue;
        }
        let cost = Money::new(cost_value, l.currency).rounded();
        let price = l.pricing.price(cost);
        let floor = l.floor.floor(cost);
        let units_per_size = cell(r, "units_per_size")
            .and_then(|u| u.parse::<i32>().ok())
            .filter(|u| *u >= 0)
            .unwrap_or(0);
        let brand_word = cell(r, "brand");
        let brand_id = brand_word.and_then(|b| {
            l.brands
                .iter()
                .find(|(_, n)| n.eq_ignore_ascii_case(b))
                .map(|(id, _)| id.clone())
        });
        let material = cell(r, "material").map(str::to_owned);
        let image = cell(r, "image").map(str::to_owned);
        let mut open_fields = Vec::new();
        let mut open_labels = Vec::new();
        let mut notes = Vec::new();
        for field in &l.rules.required_fields {
            let (present, shared) = match field.as_str() {
                "material" => (material.is_some(), false),
                "image" => {
                    let shared = l.rules.one_product_per_image
                        && image
                            .as_deref()
                            .is_some_and(|i| photo_rows.get(i).copied().unwrap_or(0) > 1);
                    (image.is_some() && !shared, shared)
                }
                "brand" => (brand_id.is_some(), false),
                "sizes" => (!sizes.is_empty(), false),
                "color" => (color.is_some(), false),
                "supplier_text" => (cell(r, "supplier_text").is_some(), false),
                _ => (true, false),
            };
            if !present {
                open_fields.push(field.clone());
                open_labels.push(open_label(field, shared));
                notes.push(open_note(field, shared));
            }
        }
        if let (Some(b), None) = (brand_word, &brand_id) {
            notes.push(format!("Brand {b} is not in the catalog yet"));
        }
        if !open_fields.is_empty() && !l.rules.save_open_rows {
            problem(
                "import.required_fields",
                "material",
                format!("Row {position}: {}", notes.join(". ")),
            );
            continue;
        }
        let status = if open_fields.is_empty() {
            DraftStatus::Ready
        } else {
            DraftStatus::NeedsYou
        };
        out.drafts.push(Draft {
            position,
            supplier_code: code,
            supplier_text: cell(r, "supplier_text").map(str::to_owned),
            product_id,
            sku,
            name,
            category_id: category.id.clone(),
            category_name: category.name.clone(),
            brand_id,
            color: color.map(str::to_owned),
            material,
            image,
            sizes,
            units_per_size,
            cost,
            price,
            floor,
            variants,
            status,
            open_fields,
            open_labels,
            note: (!notes.is_empty()).then(|| notes.join(". ")),
            refresh,
        });
    }
    Ok(out)
}

fn rows_text(problems: &[&RowProblem]) -> String {
    let mut parts: Vec<String> = problems.iter().take(3).map(|p| p.message.clone()).collect();
    if problems.len() > 3 {
        parts.push(format!("{} more", problems.len() - 3));
    }
    parts.join("; ")
}

/// The checks of an import preview, in a fixed order: required fields to create, SKUs, the
/// price floor, and the fields a person still has to fill.
pub fn checks(r: &Reading, l: &Lookup<'_>) -> Vec<Check> {
    let of =
        |rule: &str| -> Vec<&RowProblem> { r.problems.iter().filter(|p| p.rule == rule).collect() };
    let mut out = Vec::new();
    let required = of("product.required_on_create");
    out.push(Check {
        rule: "product.required_on_create".into(),
        passed: required.is_empty(),
        detail: Some(if required.is_empty() {
            format!(
                "Every row has a name and a category; the store is the seller ({} rows)",
                r.rows
            )
        } else {
            rows_text(&required)
        }),
        fix_tool: (!required.is_empty()).then(|| {
            if required.iter().any(|p| p.field == "category") {
                "list_categories".into()
            } else {
                "batch_import".into()
            }
        }),
    });
    let skus = of("product.sku_unique");
    let variant_count: usize = r.drafts.iter().map(|d| d.variants.len()).sum();
    out.push(Check {
        rule: "product.sku_unique".into(),
        passed: skus.is_empty(),
        detail: Some(if skus.is_empty() {
            format!(
                "No duplicate SKU: {} products, {variant_count} variants, none already in the catalog",
                r.drafts.len()
            )
        } else {
            rows_text(&skus)
        }),
        fix_tool: (!skus.is_empty()).then(|| "get_product".into()),
    });
    let costless = of("pricing_rule");
    let below: Vec<&Draft> = r
        .drafts
        .iter()
        .filter(|d| !l.floor.allows(d.price, d.cost))
        .collect();
    let floor_ok = below.is_empty() && costless.is_empty();
    out.push(Check {
        rule: "price_floor".into(),
        passed: floor_ok,
        detail: Some(if floor_ok {
            format!(
                "{} of {} prices at or above the floor ({})",
                r.drafts.len(),
                r.drafts.len(),
                l.floor.markup_text()
            )
        } else if !costless.is_empty() {
            rows_text(&costless)
        } else {
            below
                .iter()
                .take(3)
                .map(|d| {
                    format!(
                        "{} at {} is under the floor {}",
                        d.name,
                        money_text(&d.price),
                        money_text(&d.floor)
                    )
                })
                .collect::<Vec<_>>()
                .join("; ")
        }),
        fix_tool: (!floor_ok).then(|| "set_price".into()),
    });
    let open: Vec<&Draft> = r
        .drafts
        .iter()
        .filter(|d| !d.open_fields.is_empty())
        .collect();
    let refused = of("import.required_fields");
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for d in &open {
        for label in &d.open_labels {
            *counts.entry(label.clone()).or_default() += 1;
        }
    }
    let reasons: Vec<String> = counts.iter().map(|(n, c)| format!("{n} ({c})")).collect();
    let fields = l.rules.required_fields.join(" and ");
    out.push(Check {
        rule: "import.required_fields".into(),
        passed: refused.is_empty(),
        detail: Some(if !refused.is_empty() {
            rows_text(&refused)
        } else if open.is_empty() {
            format!("Every row has its {fields}")
        } else {
            format!(
                "{} draft{} need{} you: {}. They are saved as drafts and cannot be published until filled",
                open.len(),
                if open.len() == 1 { "" } else { "s" },
                if open.len() == 1 { "s" } else { "" },
                reasons.join(", ")
            )
        }),
        fix_tool: (!open.is_empty() || !refused.is_empty())
            .then(|| "update_product_group".into()),
    });
    out
}

/// A draft as a preview row (`changes[].to`).
pub fn draft_json(d: &Draft) -> Value {
    json!({
        "row": d.position, "product_id": d.product_id, "supplier_code": d.supplier_code,
        "sku": d.sku.to_uppercase(), "name": d.name, "category_id": d.category_id,
        "category": d.category_name, "brand_id": d.brand_id, "color": d.color,
        "sizes": d.sizes, "variants": d.variants.len(),
        "cost": num(d.cost.value), "price": num(d.price.value),
        "price_currency": d.price.currency.to_string(), "price_floor": num(d.floor.value),
        "material": d.material, "image": d.image,
        "acom:draft_status": d.status.as_str(), "acom:open_fields": d.open_fields,
        "note": d.note, "acom:supplier_text": d.supplier_text,
        "description": Value::Null,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::policies;

    pub(crate) fn categories() -> Vec<CategoryTerm> {
        [
            ("cat_outerwear", "Outerwear", "outerwear"),
            ("cat_knitwear", "Knitwear", "knitwear"),
            ("cat_shirts", "Shirts", "shirts"),
            ("cat_pants", "Pants", "pants"),
        ]
        .iter()
        .map(|(id, name, code)| CategoryTerm {
            id: (*id).into(),
            name: (*name).into(),
            term_code: (*code).into(),
            term_set: "standard".into(),
            active: true,
        })
        .collect()
    }

    fn row(code: &str, style: &str, color: &str, cat: &str, cost: &str) -> Value {
        json!({"supplier_code": code, "sku": format!("{code}-{}", &color[..3].to_uppercase()),
               "style_name": style, "color": color, "category": cat, "brand": "Northwind",
               "sizes": "S, M, L", "cost": cost, "units_per_size": "10",
               "material": "100% wool", "image": format!("https://cdn.example/{code}.jpg"),
               "supplier_text": "Text."})
    }

    struct Fixture {
        rules: ImportRules,
        pricing: PricingRule,
        floor: PriceFloorRule,
        categories: Vec<CategoryTerm>,
        brands: Vec<(String, String)>,
        taken: BTreeMap<String, String>,
        existing: BTreeMap<String, String>,
        refreshable: BTreeSet<String>,
    }

    impl Fixture {
        fn new() -> Self {
            let p = policies();
            Self {
                rules: ImportRules::from_policy(&p).unwrap(),
                pricing: PricingRule::from_policy(&p).unwrap(),
                floor: PriceFloorRule::from_policy(&p).unwrap(),
                categories: categories(),
                brands: vec![("brn_northwind".into(), "Northwind".into())],
                taken: BTreeMap::new(),
                existing: BTreeMap::new(),
                refreshable: BTreeSet::new(),
            }
        }

        fn lookup(&self) -> Lookup<'_> {
            Lookup {
                rules: &self.rules,
                pricing: &self.pricing,
                floor: &self.floor,
                categories: &self.categories,
                brands: &self.brands,
                taken_skus: &self.taken,
                existing: &self.existing,
                refreshable: &self.refreshable,
                upsert: false,
                currency: Currency::USD,
            }
        }
    }

    #[test]
    fn named_rows_become_priced_drafts() {
        let f = Fixture::new();
        let rows = vec![
            row(
                "FW26-OT-014",
                "Wool blend overcoat",
                "Charcoal",
                "Outerwear",
                "96.00",
            ),
            row(
                "FW26-KN-031",
                "Merino crew-neck sweater",
                "Navy",
                "knitwear",
                "$38",
            ),
        ];
        let r = read(&rows, &f.lookup()).unwrap();
        assert!(r.problems.is_empty(), "{:?}", r.problems);
        let coat = &r.drafts[0];
        assert_eq!(coat.name, "Wool blend overcoat, charcoal");
        assert_eq!(coat.product_id, "prd_fw26_ot_014_cha");
        assert_eq!(coat.price.value.to_string(), "239.00");
        assert_eq!(coat.category_id, "cat_outerwear");
        assert_eq!(coat.brand_id.as_deref(), Some("brn_northwind"));
        assert_eq!(coat.variants.len(), 3);
        assert_eq!(coat.variants[1].sku, "fw26-ot-014-cha-m");
        assert_eq!(coat.variants[1].variant_id, "var_fw26_ot_014_cha_m");
        assert_eq!(coat.status, DraftStatus::Ready);
        assert_eq!(r.drafts[1].price.value.to_string(), "99.00");
        let c = checks(&r, &f.lookup());
        assert!(c.iter().all(|c| c.passed), "{c:#?}");
        assert_eq!(
            c[2].detail.as_deref(),
            Some("2 of 2 prices at or above the floor (cost + 15%)")
        );
    }

    #[test]
    fn missing_fabric_and_a_shared_photo_flag_the_rows_but_keep_them() {
        let f = Fixture::new();
        let mut liner = row(
            "FW26-OT-022",
            "Quilted liner jacket",
            "Olive",
            "Outerwear",
            "52.00",
        );
        liner["material"] = json!("");
        let mut sand = row("FW26-PT-011", "Corduroy trousers", "Sand", "Pants", "34.00");
        let mut olive = row(
            "FW26-PT-012",
            "Corduroy trousers",
            "Olive",
            "Pants",
            "34.00",
        );
        sand["image"] = json!("https://cdn.example/cords.jpg");
        olive["image"] = json!("https://cdn.example/cords.jpg");
        let r = read(&[liner, sand, olive], &f.lookup()).unwrap();
        assert_eq!(r.count(DraftStatus::NeedsYou), 3);
        assert_eq!(r.drafts[0].open_fields, ["material"]);
        assert_eq!(r.drafts[0].price.value.to_string(), "129.00");
        assert_eq!(
            r.drafts[1].note.as_deref(),
            Some("The photo shows more than one item; pick one")
        );
        let c = checks(&r, &f.lookup());
        let open = &c[3];
        assert_eq!(open.rule, "import.required_fields");
        assert!(
            open.passed,
            "open fields keep the drafts: a warning, not a block"
        );
        assert_eq!(
            open.detail.as_deref(),
            Some(
                "3 drafts need you: fabric mix missing (1), photo shows more than one item (2). They are saved as drafts and cannot be published until filled"
            )
        );
        assert_eq!(open.fix_tool.as_deref(), Some("update_product_group"));
    }

    #[test]
    fn duplicates_unknown_categories_and_missing_costs_fail_their_checks() {
        let mut f = Fixture::new();
        f.taken
            .insert("fw26-ot-014-cha-s".into(), "prd_other".into());
        let mut nocat = row("FW26-XX-001", "Mystery", "Red", "Gadgets", "10.00");
        nocat["category"] = json!("Gadgets");
        let mut nocost = row("FW26-SH-007", "Flannel overshirt", "Check", "Shirts", "");
        nocost["cost"] = json!("");
        let rows = vec![
            row(
                "FW26-OT-014",
                "Wool blend overcoat",
                "Charcoal",
                "Outerwear",
                "96.00",
            ),
            nocat,
            nocost,
        ];
        let r = read(&rows, &f.lookup()).unwrap();
        assert!(r.drafts.is_empty());
        let c = checks(&r, &f.lookup());
        let failed: Vec<&str> = c
            .iter()
            .filter(|c| !c.passed)
            .map(|c| c.rule.as_str())
            .collect();
        assert_eq!(
            failed,
            [
                "product.required_on_create",
                "product.sku_unique",
                "price_floor"
            ]
        );
        assert_eq!(c[0].fix_tool.as_deref(), Some("list_categories"));
        assert_eq!(
            c[1].detail.as_deref(),
            Some("SKU FW26-OT-014-CHA-S is already in the catalog")
        );
    }

    #[test]
    fn a_sheet_without_rows_or_columns_is_refused() {
        let f = Fixture::new();
        assert_eq!(
            read(&[], &f.lookup()).unwrap_err().code,
            "invalid_file_format"
        );
        let e = read(&[json!({"name": "x"})], &f.lookup()).unwrap_err();
        assert_eq!(e.code, "invalid_file_format");
        assert!(e.message.contains("supplier_code"));
    }
}
