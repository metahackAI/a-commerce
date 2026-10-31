//! The `collection_page` resource (docs/api/merchandising.md#collection_page): curated pages
//! (`CollectionPage`, `col_`) of products in sections (`ItemList`). What its operations share:
//! the error codes, the row and its JSON, the content checks, and the SQL.
//!
//! A page is a `draft` until `publish_curation` fixes its period; after that its status
//! (`scheduled` → `active` → `ended`) is computed from the period when asked.

pub mod create_curation;
pub mod get_curation;
pub mod list_curations;
pub mod publish_curation;
pub mod update_curation;

use std::collections::BTreeSet;

use acom_catalog::service::for_merchandising::{ProductCard, product_cards};
use acom_contract::{Check, ErrorClass, ErrorCode, ToolError};
use acom_core::enums::DisplayStatus;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::display_slot::{channel_words, discontinued, discontinued_check};
use crate::model::{DisplayRules, page_status};
use crate::repo::{self, db};
use crate::{audience, check_id};

pub const COLLECTION_PAGE_NOT_FOUND: ErrorCode =
    ErrorCode::new("collection_page_not_found", ErrorClass::NotFound);
/// A linked banner that does not exist.
pub const BANNER_NOT_FOUND: ErrorCode = ErrorCode::new("banner_not_found", ErrorClass::NotFound);
/// A product on the page was discontinued (`display.exclude_discontinued`).
pub const PRODUCT_DISCONTINUED: ErrorCode =
    ErrorCode::new("product_discontinued", ErrorClass::PolicyViolation);
/// The page has no product to publish (`curation.no_empty_section`).
pub const EMPTY_CURATION: ErrorCode = ErrorCode::new("empty_curation", ErrorClass::PolicyViolation);
/// The page's period is over; it can no longer change.
pub const ALREADY_ENDED: ErrorCode = ErrorCode::new("already_ended", ErrorClass::InvalidState);
/// The page is published already (scheduled, showing, or ended).
pub const ALREADY_PUBLISHED: ErrorCode =
    ErrorCode::new("already_published", ErrorClass::InvalidState);
/// Another page took the id made from the name since the preview.
pub const DUPLICATE_COLLECTION_PAGE: ErrorCode =
    ErrorCode::new("duplicate_collection_page", ErrorClass::AlreadyExists);

/// One section of a curated page: a heading and its products in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Section {
    /// The section's heading.
    #[schemars(length(min = 1, max = 100), example = &"Coats")]
    pub name: String,
    /// Its products in order (`search_products`).
    #[schemars(length(max = 200), example = ["prd_wool_overcoat"])]
    pub item_list_element: Vec<String>,
}

/// A curated page as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct PageRow {
    pub id: String,
    pub name: String,
    pub has_part: Value,
    /// `None` for everyone.
    pub valid_for_member_tier: Option<String>,
    pub valid_from: Option<DateTime<Utc>>,
    pub valid_through: Option<DateTime<Utc>>,
    pub display_status: String,
    /// `None` for every channel.
    pub sales_channel: Option<Vec<String>>,
    pub banner_id: Vec<String>,
    pub keywords: Vec<String>,
    #[serde(skip)]
    pub version: i32,
}

impl PageRow {
    /// The sections, in order.
    pub(crate) fn sections(&self) -> Vec<Section> {
        serde_json::from_value(self.has_part.clone()).unwrap_or_default()
    }

    /// Every product of the page, each once, in page order.
    pub(crate) fn products(&self) -> Vec<String> {
        products_of(&self.sections())
    }

    /// Its status at `at`.
    pub(crate) fn status(&self, at: DateTime<Utc>) -> DisplayStatus {
        page_status(
            &self.display_status,
            self.valid_from,
            self.valid_through,
            at,
        )
    }

    /// Published: scheduled or showing (ended pages are history).
    pub(crate) fn published(&self, at: DateTime<Utc>) -> bool {
        matches!(
            self.status(at),
            DisplayStatus::Scheduled | DisplayStatus::Active
        )
    }

    /// The same page without its row version, to compare a read-back with a plan.
    pub(crate) fn unversioned(&self) -> PageRow {
        PageRow {
            version: 0,
            ..self.clone()
        }
    }
}

/// Every product of the sections, each once, in order.
pub(crate) fn products_of(sections: &[Section]) -> Vec<String> {
    let mut seen = BTreeSet::new();
    sections
        .iter()
        .flat_map(|s| s.item_list_element.iter())
        .filter(|p| seen.insert(p.as_str()))
        .cloned()
        .collect()
}

/// A page in a list: what it is, when and for whom, and how big.
pub(crate) fn page_summary(cx: &Cx, p: &PageRow, at: DateTime<Utc>) -> Value {
    json!({
        "id": p.id,
        "name": p.name,
        "status": p.status(at),
        "audience": audience(p.valid_for_member_tier.as_deref()),
        "valid_from": p.valid_from.map(|t| cx.time.format(t)),
        "valid_through": p.valid_through.map(|t| cx.time.format(t)),
        "channels": channel_words(p.sales_channel.as_deref()),
        "section_count": p.sections().len(),
        "product_count": p.products().len(),
        "banner_ids": p.banner_id,
        "keywords": p.keywords,
    })
}

pub(crate) fn not_found(id: &str) -> ToolError {
    ToolError::new(
        COLLECTION_PAGE_NOT_FOUND,
        format!("There is no curated page {id}."),
    )
    .detail(json!({"entity": "collection_page", "id": id}))
    .next("list_curations")
}

/// Sections as given: headings trimmed and not empty, product ids well formed, no product twice
/// in one section.
pub(crate) fn clean_sections(sections: Vec<Section>) -> Result<Vec<Section>, ToolError> {
    let mut out = Vec::new();
    for (n, s) in sections.into_iter().enumerate() {
        let name = s.name.trim().to_owned();
        if name.is_empty() {
            return Err(
                ToolError::invalid_input(format!("Section {} needs a heading.", n + 1))
                    .detail(json!({"field": "has_part.name", "reason": "missing"})),
            );
        }
        let mut seen = BTreeSet::new();
        for p in &s.item_list_element {
            check_id("has_part.item_list_element", "prd", p)?;
            if !seen.insert(p.as_str()) {
                return Err(ToolError::invalid_input(format!(
                    "{p} is twice in the section {name}."
                ))
                .detail(
                    json!({"field": "has_part.item_list_element", "reason": "format",
                               "id": p}),
                ));
            }
        }
        out.push(Section {
            name,
            item_list_element: s.item_list_element,
        });
    }
    Ok(out)
}

/// Banner ids well formed, each once.
pub(crate) fn clean_banners(ids: Vec<String>) -> Result<Vec<String>, ToolError> {
    let mut out: Vec<String> = Vec::new();
    for b in ids {
        check_id("banner_ids", "bnr", &b)?;
        if !out.contains(&b) {
            out.push(b);
        }
    }
    Ok(out)
}

/// Keywords trimmed, lowercase, each once.
pub(crate) fn clean_keywords(words: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for w in words {
        let w = w.trim().to_lowercase();
        if !w.is_empty() && !out.contains(&w) {
            out.push(w);
        }
    }
    out
}

/// Every product and banner exists; returns the products' cards.
pub(crate) async fn known_content(
    tx: &mut TxCx,
    sections: &[Section],
    banners: &[String],
) -> Result<Vec<ProductCard>, ToolError> {
    let products = products_of(sections);
    let cards = product_cards(tx, &products).await?;
    if let Some(p) = products
        .iter()
        .find(|p| !cards.iter().any(|c| &c.product_id == *p))
    {
        return Err(
            ToolError::new(ErrorCode::NOT_FOUND, format!("There is no product {p}."))
                .detail(json!({"entity": "product", "id": p}))
                .next("search_products"),
        );
    }
    let found = repo::banner_ids_known(tx, banners).await?;
    if let Some(b) = banners.iter().find(|b| !found.contains(b)) {
        return Err(
            ToolError::new(BANNER_NOT_FOUND, format!("There is no banner {b}."))
                .detail(json!({"entity": "banner", "id": b}))
                .next("list_banners"),
        );
    }
    Ok(cards)
}

/// `curation.no_empty_section`: every section has a product.
pub(crate) fn sections_check(rules: &DisplayRules, sections: &[Section], fix: &str) -> Check {
    let empty: Vec<&str> = sections
        .iter()
        .filter(|s| s.item_list_element.is_empty())
        .map(|s| s.name.as_str())
        .collect();
    let passed = !rules.no_empty_section || (empty.is_empty() && !sections.is_empty());
    Check {
        rule: "curation.no_empty_section".into(),
        passed,
        detail: Some(if sections.is_empty() {
            "The page has no section".into()
        } else if empty.is_empty() {
            format!(
                "{} section{}, each with products",
                sections.len(),
                if sections.len() == 1 { "" } else { "s" }
            )
        } else {
            format!("No products in: {}", empty.join(", "))
        }),
        fix_tool: (!passed).then(|| fix.to_owned()),
    }
}

/// The content checks a write of a page runs: no discontinued product, no empty section.
pub(crate) fn content_checks(
    rules: &DisplayRules,
    sections: &[Section],
    cards: &[ProductCard],
    fix: &str,
) -> Vec<Check> {
    vec![
        discontinued_check(rules, cards),
        sections_check(rules, sections, fix),
    ]
}

/// At a run: a product may have been discontinued since the preview.
pub(crate) async fn still_on_sale(
    tx: &mut TxCx,
    rules: &DisplayRules,
    products: &[String],
    next: &'static str,
) -> Result<(), ToolError> {
    let gone = discontinued(&product_cards(tx, products).await?);
    if rules.exclude_discontinued && !gone.is_empty() {
        return Err(ToolError::new(
            PRODUCT_DISCONTINUED,
            format!(
                "{} can no longer be displayed: discontinued.",
                gone.join(", ")
            ),
        )
        .detail(json!({"rule": "display.exclude_discontinued", "items": gone}))
        .next(next));
    }
    Ok(())
}

const COLUMNS: &str = "id, name, has_part, valid_for_member_tier, valid_from, valid_through,
    display_status, sales_channel, banner_id, keywords, version";

pub(crate) async fn page(tx: &mut TxCx, id: &str) -> Result<Option<PageRow>, ToolError> {
    sqlx::query_as::<_, PageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM merchandising.collection_page WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Every curated page: published ones by start, then drafts, newest first.
pub(crate) async fn pages(tx: &mut TxCx) -> Result<Vec<PageRow>, ToolError> {
    sqlx::query_as::<_, PageRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM merchandising.collection_page WHERE tenant_id = $1
         ORDER BY valid_from NULLS LAST, created_at DESC, id"
    )))
    .bind(tx.tenant.as_str())
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Inserts a new page; returns whether it was inserted (false when the id is taken).
pub(crate) async fn insert(tx: &mut TxCx, p: &PageRow) -> Result<bool, ToolError> {
    Ok(sqlx::query(
        "INSERT INTO merchandising.collection_page (tenant_id, id, name, has_part,
           valid_for_member_tier, valid_from, valid_through, display_status, sales_channel,
           banner_id, keywords)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(&p.id)
    .bind(&p.name)
    .bind(&p.has_part)
    .bind(&p.valid_for_member_tier)
    .bind(p.valid_from)
    .bind(p.valid_through)
    .bind(&p.display_status)
    .bind(&p.sales_channel)
    .bind(&p.banner_id)
    .bind(&p.keywords)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?
    .rows_affected()
        > 0)
}

/// Writes what `update_curation` changes (name, sections, audience, banners, keywords); only
/// touches the row when something differs.
pub(crate) async fn write_content(tx: &mut TxCx, p: &PageRow) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE merchandising.collection_page SET name = $3, has_part = $4,
           valid_for_member_tier = $5, banner_id = $6, keywords = $7, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2
           AND (name, has_part, valid_for_member_tier, banner_id, keywords)
               IS DISTINCT FROM ($3, $4, $5, $6, $7)",
    )
    .bind(tx.tenant.as_str())
    .bind(&p.id)
    .bind(&p.name)
    .bind(&p.has_part)
    .bind(&p.valid_for_member_tier)
    .bind(&p.banner_id)
    .bind(&p.keywords)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Writes what `publish_curation` fixes (status, period, audience, channels).
pub(crate) async fn write_publication(tx: &mut TxCx, p: &PageRow) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE merchandising.collection_page SET display_status = $3, valid_from = $4,
           valid_through = $5, valid_for_member_tier = $6, sales_channel = $7,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(&p.id)
    .bind(&p.display_status)
    .bind(p.valid_from)
    .bind(p.valid_through)
    .bind(&p.valid_for_member_tier)
    .bind(&p.sales_channel)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Removes a draft (undoing `create_curation`); a published page stays.
pub(crate) async fn delete_draft(tx: &mut TxCx, id: &str) -> Result<u64, ToolError> {
    Ok(sqlx::query(
        "DELETE FROM merchandising.collection_page
         WHERE tenant_id = $1 AND id = $2 AND display_status = 'draft'",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?
    .rows_affected())
}

/// The ids among `ids` that are pages already.
pub(crate) async fn taken_ids(tx: &mut TxCx, ids: &[String]) -> Result<Vec<String>, ToolError> {
    repo::collection_page_ids_known(tx, ids).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::policies;

    fn section(name: &str, products: &[&str]) -> Section {
        Section {
            name: name.into(),
            item_list_element: products.iter().map(|p| (*p).to_owned()).collect(),
        }
    }

    #[test]
    fn sections_are_checked_and_counted() {
        let rules = DisplayRules::from_policy(&policies(false)).unwrap();
        let s = vec![
            section("Coats", &["prd_wool_overcoat", "prd_chore_coat"]),
            section("Knits", &["prd_cashmere_sweater", "prd_wool_overcoat"]),
        ];
        assert_eq!(products_of(&s).len(), 3);
        assert!(sections_check(&rules, &s, "create_curation").passed);
        let c = sections_check(&rules, &[section("Boots", &[])], "update_curation");
        assert!(!c.passed);
        assert_eq!(c.detail.as_deref(), Some("No products in: Boots"));
        assert_eq!(c.fix_tool.as_deref(), Some("update_curation"));
        assert!(!sections_check(&rules, &[], "create_curation").passed);

        let e = clean_sections(vec![section(" ", &["prd_a"])]).unwrap_err();
        assert_eq!(e.code, "invalid_input");
        let e = clean_sections(vec![section("Coats", &["prd_a", "prd_a"])]).unwrap_err();
        assert_eq!(e.code, "invalid_input");
        assert!(clean_sections(vec![section("Coats", &["cat_knitwear"])]).is_err());
        assert_eq!(
            clean_keywords(vec![
                " Winter coat".into(),
                "winter coat".into(),
                String::new()
            ]),
            ["winter coat"]
        );
    }
}
