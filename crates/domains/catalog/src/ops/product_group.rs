//! `search_product_groups`, `get_product_group` (docs/api/catalog.md#product_group): products
//! as the catalog manages them, drafts and products under review included.

use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::StoreTime;
use acom_core::enums::{ApprovalStatus, Availability, DraftStatus};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::product::visible_group;
use super::product_not_found;
use crate::cards::{self, ProductCard};
use crate::model::{self, num};
use crate::repo::{self, Find, Order, PendingRow};
use crate::viewer::Viewer;
use crate::write::{self, DraftItem};
use crate::{INVALID_FILTER, PRODUCT_NOT_FOUND, is_operator, list, neighbors, project};

/// How `search_product_groups` orders products.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
pub enum GroupSort {
    /// Best text match first (the default with a query).
    #[serde(rename = "relevance")]
    Relevance,
    /// Latest release first (the default without a query).
    #[serde(rename = "newest")]
    Newest,
    /// Lowest list price first.
    #[serde(rename = "price")]
    Price,
    /// Highest list price first.
    #[serde(rename = "-price")]
    PriceDesc,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SearchProductGroupsInput {
    /// Words in the name, material, description or tags, or part of a SKU.
    #[schemars(length(max = 200), example = &"windbreaker")]
    pub query: Option<String>,
    /// Only products of this brand.
    #[schemars(regex(pattern = r"^brn_[a-z0-9_]+$"), example = &"brn_northwind")]
    pub brand_id: Option<String>,
    /// Only products in this category or one under it.
    #[schemars(regex(pattern = r"^cat_[a-z0-9_]+$"), example = &"cat_summer_outerwear")]
    pub category_id: Option<String>,
    /// Only products in this review state.
    #[schemars(example = &"pending_review")]
    pub approval_status: Option<ApprovalStatus>,
    /// Only products with this availability (derived from their variants).
    #[schemars(example = &"out_of_stock")]
    pub availability: Option<Availability>,
    /// Only the drafts one supplier sheet import made (`batch_import`).
    #[schemars(regex(pattern = r"^imp_[a-z0-9_]+$"), example = &"imp_fw26_supplier")]
    pub import_id: Option<String>,
    /// Only drafts in this state. ready: every field the sheet needs is there; needs_you: a
    /// field is missing or unclear; working: still being filled.
    #[schemars(example = &"needs_you")]
    pub draft_status: Option<DraftStatus>,
    /// Order of the results.
    #[schemars(example = &"newest")]
    pub sort: Option<GroupSort>,
    /// Only these top-level fields of each product.
    #[schemars(example = ["id", "name", "approval_status"])]
    pub fields: Option<Vec<String>>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

/// Find products by catalog-management conditions.
#[derive(Debug)]
pub struct SearchProductGroups;

impl Operation for SearchProductGroups {
    const META: OpMeta = OpMeta {
        address: "catalog.product_group.search",
        tool: "search_product_groups",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/product-groups",
        summary: "Find products by catalog conditions such as review state and brand, drafts and products under review included. The shopper-facing search is search_products.",
        consumes: &[],
        produces: &[Slot::many("product_id")],
        checks: &[],
        errors: &[INVALID_FILTER],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = SearchProductGroupsInput;
    type Output = Value;
}

/// Where a draft came from and what is still open on it (`acom:draft`), for operators.
pub(crate) fn draft_json(i: &DraftItem) -> Value {
    json!({"import_id": i.product_import_id, "row": i.position, "supplier_code": i.supplier_code,
           "draft_status": i.draft_status, "open_fields": i.open_fields, "note": i.draft_note,
           "supplier_text": i.supplier_text})
}

/// A product in the catalog-management list.
fn group_item(c: &ProductCard, operator: bool) -> Value {
    let mut out = json!({"id": c.id, "@type": "ProductGroup", "name": c.name,
        "brand_id": c.brand_id, "category_id": c.category_id,
        "approval_status": c.approval_status, "varies_by": c.varies_by, "image": c.image,
        "price": c.summary.price.map_or(Value::Null, |p| num(p.value)),
        "price_currency": c.summary.price.map(|p| p.currency.to_string()),
        "inventory_level": c.summary.inventory_level,
        "availability": if c.variants.is_empty() { Value::Null } else { json!(c.summary.availability) },
        "variant_count": c.variants.len(),
        "release_date": c.release_date.map(|d| d.to_string())});
    if operator {
        out["cost"] = c.summary.cost.map_or(Value::Null, |p| num(p.value));
    }
    out
}

impl ReadOp<Cx> for SearchProductGroups {
    async fn read(&self, cx: &Cx, input: SearchProductGroupsInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let query = input
            .query
            .as_deref()
            .map(str::trim)
            .filter(|q| !q.is_empty());
        let viewer = Viewer {
            operator: is_operator(&cx.principal),
            tier: None,
            at: cx.clock.now(),
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let mut only = match &input.category_id {
            Some(cat) => Some(
                neighbors::category_products(&mut tx, cat)
                    .await?
                    .ok_or_else(|| {
                        ToolError::new(INVALID_FILTER, format!("There is no category {cat}."))
                    .detail(json!({"field": "category_id", "reason": "enum", "entity": "category"}))
                    .next("search_product_groups")
                    })?,
            ),
            None => None,
        };
        if input.import_id.is_some() || input.draft_status.is_some() {
            let drafts = write::drafts(
                &mut tx,
                input.import_id.as_deref(),
                input.draft_status.map(|s| s.as_str()),
            )
            .await?;
            only = Some(match only {
                Some(ids) => ids.into_iter().filter(|i| drafts.contains(i)).collect(),
                None => drafts,
            });
        }
        let ranked = match query {
            Some(q) => Some(repo::match_text(&mut tx, q).await?),
            None => None,
        };
        let order = match input.sort {
            Some(GroupSort::Relevance) if ranked.is_some() => Order::Relevance,
            None if ranked.is_some() => Order::Relevance,
            Some(GroupSort::Price) => Order::Price,
            Some(GroupSort::PriceDesc) => Order::PriceDesc,
            _ => Order::Newest,
        };
        let availability: Vec<String> = input
            .availability
            .map(|a| vec![a.as_str().to_owned()])
            .unwrap_or_default();
        let (ids, total) = repo::find(
            &mut tx,
            &Find {
                ranked: ranked.as_deref(),
                only: only.as_deref(),
                tags: &[],
                availability: &availability,
                min_inventory: None,
                max_inventory: None,
                min_price: None,
                max_price: None,
                price_currency: None,
                active_only: !viewer.operator,
                with_drafts: viewer.operator,
                brand_id: input.brand_id.as_deref(),
                approval_status: input.approval_status.map(|s| s.as_str()),
                order,
                limit: page.limit,
                offset: page.offset,
            },
        )
        .await?;
        let found = cards::load(&mut tx, &ids, &viewer).await?;
        let drafts = if viewer.operator {
            write::items_of(&mut tx, &ids).await?
        } else {
            Vec::new()
        };
        tx.commit().await.map_err(ToolError::from)?;
        let fields = input.fields.unwrap_or_default();
        let items = found
            .iter()
            .map(|c| {
                let mut item = group_item(c, viewer.operator);
                if let Some(d) = drafts.iter().find(|d| d.product_group_id == c.id) {
                    item["acom:draft"] = draft_json(d);
                }
                project(item, &fields)
            })
            .collect();
        Ok(list(items, total, &page))
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetProductGroupInput {
    /// The product.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_windbreaker")]
    pub product_id: String,
    /// Only these top-level fields.
    #[schemars(example = ["id", "approval_status", "has_variant"])]
    pub fields: Option<Vec<String>>,
}

/// One product as the catalog manages it.
#[derive(Debug)]
pub struct GetProductGroup;

impl Operation for GetProductGroup {
    const META: OpMeta = OpMeta {
        address: "catalog.product_group.get",
        tool: "get_product_group",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/catalog/product-groups/{product_id}",
        summary: "One product as the catalog manages it: review state, option axes, every variant with its offer, and changes waiting for approval. For selling prices and stock use get_product.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::many("variant_id")],
        checks: &[],
        errors: &[PRODUCT_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetProductGroupInput;
    type Output = Value;
}

fn pending_json(p: &PendingRow, time: &StoreTime) -> Value {
    json!({"preview_id": p.id, "change_set_id": p.change_set_id, "tool": p.instrument,
           "op": p.change.get("op"), "entity": p.change.get("entity"), "id": p.change.get("id"),
           "from": p.change.get("from"), "to": p.change.get("to"),
           "created_at": time.format(p.created_at)})
}

impl ReadOp<Cx> for GetProductGroup {
    async fn read(&self, cx: &Cx, input: GetProductGroupInput) -> Result<Value, ToolError> {
        let operator = is_operator(&cx.principal);
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let Some(g) = visible_group(cx, &mut tx, &input.product_id).await? else {
            return Err(product_not_found(
                "product",
                &input.product_id,
                "search_product_groups",
            ));
        };
        let mut variants = repo::variants(&mut tx, std::slice::from_ref(&g.id)).await?;
        model::order_variants(&mut variants);
        let mut records = vec![g.id.clone()];
        for v in &variants {
            records.push(v.id.clone());
            records.extend(v.offer_id.clone());
        }
        let pending = if operator {
            repo::pending(&mut tx, &records).await?
        } else {
            Vec::new()
        };
        let category = neighbors::categories(&mut tx, Some(std::slice::from_ref(&g.id)))
            .await?
            .into_iter()
            .filter(|c| c.direct)
            .max_by_key(|c| c.is_primary)
            .map(|c| c.category_id);
        let draft = if operator {
            write::items_of(&mut tx, std::slice::from_ref(&g.id))
                .await?
                .pop()
        } else {
            None
        };
        let offers: Vec<String> = variants.iter().filter_map(|v| v.offer_id.clone()).collect();
        let schedules = if operator {
            write::schedules(&mut tx, &offers).await?
        } else {
            Vec::new()
        };
        tx.commit().await.map_err(ToolError::from)?;
        let has_variant: Vec<Value> = variants
            .iter()
            .map(|v| {
                let mut line = json!({"variant_id": v.id, "sku": v.sku, "name": v.name,
                    "size": v.size, "color": v.color, "acom:sale_type": v.sale_type,
                    "acom:managed": v.is_managed, "offer_id": v.offer_id,
                    "price": v.price.map_or(Value::Null, num), "price_currency": v.price_currency,
                    "availability": v.availability, "inventory_level": v.inventory_level,
                    "acom:safety_stock": v.safety_stock});
                if operator {
                    line["cost"] = v.cost_value.map_or(Value::Null, num);
                    let mine: Vec<Value> = schedules
                        .iter()
                        .filter(|s| Some(&s.offer_id) == v.offer_id.as_ref())
                        .map(|s| json!({"unit_price_specification_id": s.id, "price": num(s.price),
                                        "valid_from": cx.time.format(s.valid_from),
                                        "valid_through": s.valid_through.map(|t| cx.time.format(t))}))
                        .collect();
                    if !mine.is_empty() {
                        line["scheduled_prices"] = json!(mine);
                    }
                }
                line
            })
            .collect();
        let out = json!({
            "id": g.id, "@type": "ProductGroup", "name": g.name, "description": g.description,
            "brand_id": g.brand_id, "category_id": category, "approval_status": g.approval_status,
            "varies_by": g.varies_by, "material": g.material, "keywords": g.keywords,
            "additional_property": model::properties_json(g.material.as_deref(), &g.additional_property),
            "image": g.image, "release_date": g.release_date.map(|d| d.to_string()),
            "has_merchant_return_policy_id": g.has_merchant_return_policy_id,
            "has_variant": has_variant,
            "pending_changes": pending.iter().map(|p| pending_json(p, &cx.time)).collect::<Vec<_>>(),
            "version": g.version, "updated_at": cx.time.format(g.updated_at),
        });
        let mut out = out;
        if let Some(d) = &draft {
            out["acom:draft"] = draft_json(d);
        }
        Ok(project(out, &input.fields.unwrap_or_default()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops::testing;

    #[test]
    fn sort_words_follow_the_spec() {
        let s: GroupSort = serde_json::from_str("\"-price\"").unwrap_or(GroupSort::Newest);
        assert_eq!(s, GroupSort::PriceDesc);
        let schema = acom_contract::schema::closed_schema_for::<SearchProductGroupsInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert_eq!(
            schema["properties"]["sort"]["enum"],
            json!(["relevance", "newest", "price", "-price"])
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn summer_outerwear_lists_and_windbreaker_reads_back() {
        let cx = testing::operator_db().await;
        let out = SearchProductGroups
            .read(
                &cx,
                SearchProductGroupsInput {
                    query: Some("windbreaker".into()),
                    brand_id: None,
                    category_id: None,
                    approval_status: Some(ApprovalStatus::Active),
                    availability: None,
                    import_id: None,
                    draft_status: None,
                    sort: None,
                    fields: None,
                    limit: None,
                    cursor: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(out["items"][0]["id"], "prd_windbreaker");
        let g = GetProductGroup
            .read(
                &cx,
                GetProductGroupInput {
                    product_id: "prd_windbreaker".into(),
                    fields: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(g["varies_by"], json!(["color", "size"]));
        assert_eq!(g["has_variant"].as_array().map(Vec::len), Some(25));
    }
}
