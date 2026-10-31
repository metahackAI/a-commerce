//! The `category` resource (docs/api/merchandising.md#category): the standard tree products are
//! classified in, the display tree shoppers browse, and the mappings that show a standard
//! category's products in display categories. What its operations share: error codes, the
//! policy values they check (`merchandising.yml category`), and the SQL for terms and mappings.

pub mod create_category;
pub mod get_category;
pub mod list_categories;
pub mod map_category;
pub mod unmap_category;
pub mod update_category;

use acom_contract::{Check, ErrorClass, ErrorCode, ToolError};
use acom_policy::PolicySet;
use acom_store::TxCx;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::model::slug;
use crate::repo::{CategoryRow, db};

pub const CATEGORY_NOT_FOUND: ErrorCode =
    ErrorCode::new("category_not_found", ErrorClass::NotFound);

fn category_json(c: &CategoryRow) -> Value {
    json!({
        "id": c.id,
        "name": c.name,
        "term_code": c.term_code,
        "term_set": c.term_set,
        "parent_id": c.parent_id,
        "display_order": c.display_order,
        "active": c.active,
    })
}

pub const PARENT_NOT_FOUND: ErrorCode = ErrorCode::new("parent_not_found", ErrorClass::NotFound);
pub const DEPTH_EXCEEDED: ErrorCode = ErrorCode::new("depth_exceeded", ErrorClass::LimitExceeded);
pub const DUPLICATE_TERM_CODE: ErrorCode =
    ErrorCode::new("duplicate_term_code", ErrorClass::AlreadyExists);
pub const CYCLE_DETECTED: ErrorCode = ErrorCode::new("cycle_detected", ErrorClass::Domain);
pub const HAS_CHILDREN: ErrorCode = ErrorCode::new("has_children", ErrorClass::InvalidState);
pub const HAS_MAPPED_PRODUCTS: ErrorCode =
    ErrorCode::new("has_mapped_products", ErrorClass::InvalidState);
pub const CROSS_SET_VIOLATION: ErrorCode =
    ErrorCode::new("cross_set_violation", ErrorClass::Domain);
pub const PRIMARY_CONFLICT: ErrorCode = ErrorCode::new("primary_conflict", ErrorClass::Domain);
pub const DISPLAY_CATEGORY_INACTIVE: ErrorCode =
    ErrorCode::new("display_category_inactive", ErrorClass::InvalidState);
pub const MAPPING_NOT_FOUND: ErrorCode = ErrorCode::new("mapping_not_found", ErrorClass::NotFound);
pub const PRIMARY_NEEDS_REASSIGN: ErrorCode =
    ErrorCode::new("primary_needs_reassign", ErrorClass::Domain);

/// `merchandising.yml category`, the parts these writes check.
#[derive(Debug, Clone, PartialEq)]
pub struct CategoryRules {
    pub max_depth: usize,
    pub primary_per_standard: u64,
    pub delete_only_if: Vec<String>,
}

impl CategoryRules {
    pub fn from_policy(p: &PolicySet) -> Result<Self, ToolError> {
        let missing = |k: &str| {
            ToolError::new(
                ErrorCode::INTERNAL,
                format!("The store policy `{k}` is missing or malformed."),
            )
        };
        Ok(Self {
            max_depth: p
                .lookup("merchandising.category.max_depth")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| missing("merchandising.category.max_depth"))?,
            primary_per_standard: p
                .lookup("merchandising.category.primary_display_per_standard")
                .and_then(Value::as_u64)
                .unwrap_or(1),
            delete_only_if: p
                .lookup("merchandising.category.delete_only_if")
                .and_then(Value::as_array)
                .map(|l| {
                    l.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        })
    }
}

fn not_found(id: &str) -> ToolError {
    ToolError::new(CATEGORY_NOT_FOUND, format!("There is no category {id}."))
        .detail(json!({"entity": "category", "id": id}))
        .next("list_categories")
}

/// The category row as a write sees it, with its version.
#[derive(Debug, Clone, sqlx::FromRow)]
struct Term {
    id: String,
    name: String,
    in_defined_term_set: String,
    broader_id: Option<String>,
    position: i32,
    is_active: bool,
    version: i32,
}

async fn term(tx: &mut TxCx, id: &str) -> Result<Option<Term>, ToolError> {
    sqlx::query_as::<_, Term>(
        "SELECT id, name, in_defined_term_set, broader_id, position, is_active, version
         FROM merchandising.defined_term WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// How many levels a category's subtree has (1 for a leaf).
async fn subtree_height(tx: &mut TxCx, id: &str) -> Result<usize, ToolError> {
    let h: i32 = sqlx::query_scalar(
        "WITH RECURSIVE down AS (
           SELECT id, 1 AS level FROM merchandising.defined_term WHERE tenant_id = $1 AND id = $2
           UNION ALL
           SELECT c.id, d.level + 1 FROM merchandising.defined_term c JOIN down d ON c.broader_id = d.id
           WHERE c.tenant_id = $1 AND d.level < 16)
         SELECT max(level) FROM down",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(usize::try_from(h).unwrap_or(1))
}

/// A category's mappings, either side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
pub struct Mapping {
    pub id: String,
    pub standard_category_id: String,
    pub display_category_id: String,
    pub is_primary: bool,
}

pub(crate) async fn mappings_of(
    tx: &mut TxCx,
    category_id: &str,
) -> Result<Vec<Mapping>, ToolError> {
    sqlx::query_as::<_, Mapping>(
        "SELECT id, standard_category_id, display_category_id, is_primary
         FROM merchandising.category_mapping
         WHERE tenant_id = $1 AND (standard_category_id = $2 OR display_category_id = $2)
         ORDER BY is_primary DESC, id",
    )
    .bind(tx.tenant.as_str())
    .bind(category_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

async fn mapping(tx: &mut TxCx, id: &str) -> Result<Option<Mapping>, ToolError> {
    sqlx::query_as::<_, Mapping>(
        "SELECT id, standard_category_id, display_category_id, is_primary
         FROM merchandising.category_mapping WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

async fn insert_mapping(tx: &mut TxCx, m: &Mapping) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO merchandising.category_mapping (tenant_id, id, standard_category_id,
           display_category_id, is_primary)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(&m.id)
    .bind(&m.standard_category_id)
    .bind(&m.display_category_id)
    .bind(m.is_primary)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

async fn delete_mapping(tx: &mut TxCx, id: &str) -> Result<(), ToolError> {
    sqlx::query("DELETE FROM merchandising.category_mapping WHERE tenant_id = $1 AND id = $2")
        .bind(tx.tenant.as_str())
        .bind(id)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
    Ok(())
}

async fn children(tx: &mut TxCx, id: &str) -> Result<i64, ToolError> {
    sqlx::query_scalar(
        "SELECT count(*) FROM merchandising.defined_term WHERE tenant_id = $1 AND broader_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// What still hangs on a category: `(children, mappings, products)`.
async fn attached(tx: &mut TxCx, id: &str) -> Result<(i64, i64, i64), ToolError> {
    let kids = children(tx, id).await?;
    let maps = mappings_of(tx, id).await?.len() as i64;
    let products: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM merchandising.product_group_category WHERE tenant_id = $1 AND category_id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok((kids, maps, products))
}

/// `category.delete_only_if`: whether a category can be switched off now.
fn delete_check(rules: &CategoryRules, (kids, maps, products): (i64, i64, i64)) -> Check {
    let mut left = Vec::new();
    for r in &rules.delete_only_if {
        match r.as_str() {
            "no_children" if kids > 0 => left.push(format!(
                "{kids} categor{} under it",
                if kids == 1 { "y" } else { "ies" }
            )),
            "no_mappings" if maps > 0 => left.push(format!(
                "{maps} mapping{}",
                if maps == 1 { "" } else { "s" }
            )),
            "no_products" if products > 0 => left.push(format!(
                "{products} product{}",
                if products == 1 { "" } else { "s" }
            )),
            _ => {}
        }
    }
    Check {
        rule: "category.delete_only_if".into(),
        passed: left.is_empty(),
        detail: Some(if left.is_empty() {
            "Nothing is under it, mapped to it or placed in it".into()
        } else {
            format!("Still has {}", left.join(", "))
        }),
        fix_tool: (!left.is_empty()).then(|| {
            if maps > 0 {
                "unmap_category".into()
            } else {
                "list_categories".into()
            }
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::create_category::CreateCategoryInput;
    use super::map_category::{MapCategoryInput, mapping_id};
    use super::unmap_category::UnmapCategoryInput;
    use super::update_category::UpdateCategoryInput;
    use super::*;
    use crate::testing::policies;

    #[test]
    fn rules_come_from_the_policy() {
        let r = CategoryRules::from_policy(&policies(false)).unwrap();
        assert_eq!(r.max_depth, 4);
        assert_eq!(r.primary_per_standard, 1);
        let c = delete_check(&r, (0, 2, 5));
        assert!(!c.passed);
        assert_eq!(
            c.detail.as_deref(),
            Some("Still has 2 mappings, 5 products")
        );
        assert_eq!(c.fix_tool.as_deref(), Some("unmap_category"));
        assert!(delete_check(&r, (0, 0, 0)).passed);
    }

    #[test]
    fn ids_and_inputs() {
        assert_eq!(
            mapping_id("cat_summer_outerwear", "cat_travel"),
            "map_summer_outerwear_travel"
        );
        assert_eq!(slug("Light Outerwear!"), "light_outerwear");
        for schema in [
            acom_contract::schema::closed_schema_for::<CreateCategoryInput>(),
            acom_contract::schema::closed_schema_for::<UpdateCategoryInput>(),
            acom_contract::schema::closed_schema_for::<MapCategoryInput>(),
            acom_contract::schema::closed_schema_for::<UnmapCategoryInput>(),
        ] {
            assert!(
                acom_contract::schema::violations(&schema).is_empty(),
                "{schema:#}"
            );
        }
    }
}
