//! A product as people shop it: the product, its variants with stock, the price the viewer
//! pays, and its category. `get_product` and `search_products` build items from the same card,
//! so the two always agree on prices and stock.

use std::collections::BTreeMap;

use acom_contract::ToolError;
use acom_core::StoreTime;
use acom_store::TxCx;
use chrono::NaiveDate;
use serde_json::{Value, json};

use crate::model::{self, Summary, Variant, num};
use crate::neighbors::{self, SalePrice};
use crate::repo::{self, Group};
use crate::viewer::Viewer;

/// One product with everything a product list or page shows.
#[derive(Debug, Clone, PartialEq)]
pub struct ProductCard {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub brand_id: Option<String>,
    /// The primary category, when merchandising has placed the product.
    pub category_id: Option<String>,
    pub approval_status: String,
    pub varies_by: Vec<String>,
    pub material: Option<String>,
    pub keywords: Vec<String>,
    /// Comparison attributes as stored: `[{name, value, unit_text, size?}]`.
    pub additional_property: Value,
    pub image: Option<String>,
    pub release_date: Option<NaiveDate>,
    /// Variants that have an offer, best-stocked color first.
    pub variants: Vec<Variant>,
    pub summary: Summary,
    /// The sale price the viewer pays now, if any.
    pub member_price: Option<SalePrice>,
}

impl ProductCard {
    fn new(g: Group, mut variants: Vec<Variant>, prices: &[SalePrice]) -> Self {
        variants.retain(|v| v.offer_id.is_some());
        model::order_variants(&mut variants);
        let summary = model::summarize(&variants);
        let member_price = model::member_price(&variants, prices).cloned();
        Self {
            id: g.id,
            name: g.name,
            description: g.description,
            brand_id: g.brand_id,
            category_id: None,
            approval_status: g.approval_status,
            varies_by: g.varies_by,
            material: g.material,
            keywords: g.keywords,
            additional_property: g.additional_property,
            image: g.image,
            release_date: g.release_date,
            variants,
            summary,
            member_price,
        }
    }

    fn price_fields(&self, viewer: &Viewer, time: &StoreTime, out: &mut Value) {
        out["price"] = self.summary.price.map_or(Value::Null, |p| num(p.value));
        out["price_currency"] = json!(self.summary.price.map(|p| p.currency.to_string()));
        out["member_price"] = self
            .member_price
            .as_ref()
            .map_or(Value::Null, |p| model::member_price_json(p, time));
        if viewer.operator {
            out["cost"] = self.summary.cost.map_or(Value::Null, |c| num(c.value));
        }
        out["inventory_level"] = json!(self.summary.inventory_level);
        out["availability"] = json!(self.summary.availability);
    }

    /// A `search_products` item. `cost` only for operators.
    pub fn item_json(&self, viewer: &Viewer, time: &StoreTime) -> Value {
        let mut out = json!({"id": self.id, "name": self.name, "category_id": self.category_id,
                             "brand_id": self.brand_id, "image": self.image});
        self.price_fields(viewer, time, &mut out);
        out["has_variant"] = self.variants.iter().map(Variant::line_json).collect();
        out
    }

    /// The `get_product` entity: the item plus the product's attributes.
    pub fn product_json(&self, viewer: &Viewer, time: &StoreTime) -> Value {
        let mut out = json!({"id": self.id, "@type": "ProductGroup", "name": self.name,
                             "description": self.description, "brand_id": self.brand_id,
                             "category_id": self.category_id,
                             "approval_status": self.approval_status,
                             "varies_by": self.varies_by, "image": self.image});
        self.price_fields(viewer, time, &mut out);
        out["additional_property"] = json!(model::properties_json(
            self.material.as_deref(),
            &self.additional_property
        ));
        out["has_variant"] = self.variants.iter().map(Variant::line_json).collect();
        out
    }

    /// The color shown first (the one with the most stock).
    pub fn lead_color(&self) -> Option<&str> {
        self.variants.first().and_then(|v| v.color.as_deref())
    }

    /// A comparison attribute as stored, by name.
    pub fn attribute(&self, name: &str) -> Option<&Value> {
        self.additional_property
            .as_array()?
            .iter()
            .find(|p| p.get("name").and_then(Value::as_str) == Some(name))
    }
}

/// Cards for these products, in the order given; ids that are not products are skipped.
/// Shoppers only get active products.
pub async fn load(
    tx: &mut TxCx,
    ids: &[String],
    viewer: &Viewer,
) -> Result<Vec<ProductCard>, ToolError> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let groups = repo::groups(tx, ids).await?;
    let mut variants: BTreeMap<String, Vec<Variant>> = BTreeMap::new();
    let mut all = repo::variants(tx, ids).await?;
    crate::write::apply_scheduled_prices(tx, &mut all, viewer.at).await?;
    for v in all {
        variants
            .entry(v.product_group_id.clone())
            .or_default()
            .push(v);
    }
    let offers: Vec<String> = variants
        .values()
        .flatten()
        .filter_map(|v| v.offer_id.clone())
        .collect();
    let prices = neighbors::sale_prices(tx, &offers, viewer.tier.as_deref(), viewer.at).await?;
    // The primary category, else the first one the product is placed in.
    let mut primary: BTreeMap<String, String> = BTreeMap::new();
    for c in neighbors::categories(tx, Some(ids)).await? {
        if c.direct && (c.is_primary || !primary.contains_key(&c.product_id)) {
            primary.insert(c.product_id, c.category_id);
        }
    }
    let mut by_id: BTreeMap<String, ProductCard> = groups
        .into_iter()
        .filter(|g| viewer.operator || g.approval_status == "active")
        .map(|g| {
            let vs = variants.remove(&g.id).unwrap_or_default();
            let mut card = ProductCard::new(g, vs, &prices);
            card.category_id = primary.get(&card.id).cloned();
            (card.id.clone(), card)
        })
        .collect();
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
}
