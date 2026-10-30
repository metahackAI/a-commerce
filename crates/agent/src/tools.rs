//! The tool manifest as the agent reads it (`GET /v1/tools`, already filtered by the caller's
//! scopes). The agent knows tools only from here; it never links a domain crate.

use serde::Deserialize;
use serde_json::Value;

/// R, W, W$, and person-only (never in a manifest, refused if it ever is).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Read,
    Write,
    WriteApproval,
    Person,
}

/// A value that flows between tools (`product_id[]`, `promotion_id`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SlotSpec {
    pub name: String,
    #[serde(default)]
    pub many: bool,
}

impl std::fmt::Display for SlotSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.name, if self.many { "[]" } else { "" })
    }
}

/// One manifest entry.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolSpec {
    /// `verb_noun`, what the model sees.
    pub tool: String,
    /// `domain.resource.operation`.
    pub address: String,
    pub kind: ToolKind,
    #[serde(default)]
    pub summary: String,
    #[serde(default)]
    pub consumes: Vec<SlotSpec>,
    #[serde(default)]
    pub produces: Vec<SlotSpec>,
    /// Policy check keys the preview runs.
    #[serde(default)]
    pub checks: Vec<String>,
    #[serde(default)]
    pub input_schema: Value,
}

/// `GET /v1/tools`.
#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub items: Vec<ToolSpec>,
}

impl ToolSpec {
    pub fn domain(&self) -> &str {
        self.address.split('.').next().unwrap_or(&self.address)
    }

    pub fn resource(&self) -> &str {
        self.address.split('.').nth(1).unwrap_or("")
    }

    pub fn is_write(&self) -> bool {
        matches!(self.kind, ToolKind::Write | ToolKind::WriteApproval)
    }

    /// A write that only prepares (`prepare_order`): it produces a `preview_id` that a person's
    /// button runs. The runtime previews it and never commits it.
    pub fn preview_only(&self) -> bool {
        self.is_write() && self.produces.iter().any(|s| s.name == "preview_id")
    }

    /// Whether it produces a remembered preference (`memory_id`).
    pub fn remembers(&self) -> bool {
        self.produces.iter().any(|s| s.name == "memory_id")
    }

    pub fn properties(&self) -> Option<&serde_json::Map<String, Value>> {
        self.input_schema
            .get("properties")
            .and_then(Value::as_object)
    }

    pub fn required(&self) -> Vec<&str> {
        self.input_schema
            .get("required")
            .and_then(Value::as_array)
            .map(|r| r.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default()
    }

    /// The input field a slot fills: a field named like the slot (`product_id`), its plural
    /// (`product_ids`), or one that declares `"x-slot": "<slot>"`. Returns the field and whether
    /// it takes a list.
    pub fn field_for_slot(&self, slot: &str) -> Option<(String, bool)> {
        let props = self.properties()?;
        let plural = format!("{slot}s");
        let found = props
            .get_key_value(slot)
            .or_else(|| props.get_key_value(plural.as_str()))
            .or_else(|| {
                props
                    .iter()
                    .find(|(_, p)| p.get("x-slot").and_then(Value::as_str) == Some(slot))
            })?;
        let is_array = found.1.get("type").and_then(Value::as_str) == Some("array");
        Some((found.0.clone(), is_array))
    }

    /// One line for prompts and step logs: `set_sale_prices (write, needs approval)`.
    pub fn kind_words(&self) -> &'static str {
        match self.kind {
            ToolKind::Read => "read",
            ToolKind::Write => "write",
            ToolKind::WriteApproval => "write, always needs a person's approval",
            ToolKind::Person => "person-only",
        }
    }
}

#[cfg(test)]
pub(crate) mod fixtures {
    //! A small manifest in the shape commerce serves, with the tools of scenes B1 and F2.

    use serde_json::{Value, json};

    use super::{Manifest, ToolSpec};

    fn id(prefix: &str, what: &str) -> Value {
        json!({"type": "string", "pattern": format!("^{prefix}_[a-z0-9_]+$"), "description": what})
    }

    #[allow(clippy::too_many_arguments)]
    fn tool(
        tool: &str,
        address: &str,
        kind: &str,
        summary: &str,
        consumes: Value,
        produces: Value,
        properties: Value,
        required: Value,
    ) -> Value {
        json!({
            "tool": tool, "address": address, "kind": kind, "summary": summary,
            "consumes": consumes, "produces": produces, "checks": [],
            "group": "offer", "method": "GET", "path": "/v1/x", "errors": [],
            "audience": "both", "action_type": "SearchAction", "person_scope": "",
            "input_schema": {"type": "object", "additionalProperties": false,
                             "properties": properties, "required": required}
        })
    }

    pub fn manifest_json() -> Value {
        let money = json!({"type": "object", "additionalProperties": false, "required": ["value", "currency"],
            "description": "An amount.",
            "properties": {"value": {"type": "number", "description": "Amount."},
                           "currency": {"type": "string", "pattern": "^[A-Z]{3}$", "description": "Code."}}});
        json!({"items": [
            tool("get_policy", "platform.policy.get", "read",
                 "Return the store's policy values for a key, such as the maximum discount.",
                 json!([{"name": "policy_key", "many": false}]), json!([{"name": "policy_key", "many": true}]),
                 json!({"key": {"type": "string", "description": "Policy key, like max_discount."}}), json!(["key"])),
            tool("search_products", "search.product.search_products", "read",
                 "Find products by words, category, price, or stock.",
                 json!([]), json!([{"name": "product_id", "many": true}, {"name": "category_id", "many": false}]),
                 json!({"query": {"type": "string", "description": "Words to look for."},
                        "category": {"type": "string", "description": "Category word, like summer_outerwear."},
                        "limit": {"type": "integer", "minimum": 1, "maximum": 100, "description": "Max results."}}),
                 json!([])),
            tool("list_tiers", "customer.membership.list_tiers", "read",
                 "List the store's member tiers with member counts.",
                 json!([]), json!([{"name": "tier_id", "many": true}]), json!({}), json!([])),
            tool("list_display_slots", "merchandising.display_slot.list_display_slots", "read",
                 "List display slots such as the main page hero and what is booked in them.",
                 json!([]), json!([{"name": "slot_id", "many": true}]),
                 json!({"page": {"type": "string", "description": "Page, like home."}}), json!([])),
            tool("set_sale_prices", "promotion.offer.set_sale_prices", "write_approval",
                 "Set member-tier or period sale prices for products and group them as one promotion.",
                 json!([{"name": "product_id", "many": true}, {"name": "category_id", "many": false}, {"name": "tier_id", "many": false}]),
                 json!([{"name": "promotion_id", "many": false}]),
                 json!({"name": {"type": "string", "description": "Promotion name."},
                        "valid_for_member_tier": {"type": "string", "pattern": "^[a-z][a-z0-9_]*$", "description": "Tier id or everyone.",
                                                  "x-aliases": {"vip": ["VIP members", "vips"]}},
                        "valid_from": {"type": "string", "format": "date-time", "description": "Start."},
                        "valid_through": {"type": "string", "format": "date-time", "description": "End."},
                        "scope_mode": {"type": "string", "enum": ["category", "products"], "description": "category or products."},
                        "category_id": id("cat", "Category."),
                        "product_ids": {"type": "array", "items": id("prd", "Product."), "description": "Products."},
                        "percent_off": {"type": "number", "exclusiveMinimum": 0, "exclusiveMaximum": 100, "description": "Percent off."},
                        "exclude_low_stock": {"type": "boolean", "description": "Leave out products under the stock minimum."},
                        "overrides": {"type": "array", "description": "Per-product percent.", "items": {
                            "type": "object", "additionalProperties": false, "required": ["product_id", "percent_off"],
                            "properties": {"product_id": id("prd", "Product."),
                                           "percent_off": {"type": "number", "description": "Percent off."}}}}}),
                 json!(["name", "valid_for_member_tier", "valid_from", "valid_through", "scope_mode", "percent_off"])),
            tool("create_coupon", "promotion.coupon.create_coupon", "write_approval",
                 "Create a coupon for a tier that records a promotion's benefit at checkout.",
                 json!([{"name": "promotion_id", "many": false}, {"name": "tier_id", "many": false}]),
                 json!([{"name": "coupon_id", "many": false}]),
                 json!({"name": {"type": "string", "description": "Coupon name."},
                        "promotion_id": id("promo", "Promotion the coupon records.")}),
                 json!(["name", "promotion_id"])),
            tool("schedule_banner", "merchandising.banner.schedule_banner", "write_approval",
                 "Schedule a banner in a display slot for an audience and period.",
                 json!([{"name": "slot_id", "many": false}, {"name": "promotion_id", "many": false}]),
                 json!([{"name": "booking_id", "many": false}]),
                 json!({"slot_id": id("slot", "Slot."), "promotion_id": id("promo", "Promotion."),
                        "audience": {"type": "string", "description": "Tier id or everyone."}}),
                 json!(["slot_id", "promotion_id"])),
            tool("get_cart", "sales.cart.get_cart", "read", "Show the shopper's cart with totals.",
                 json!([]), json!([{"name": "cart_id", "many": false}]), json!({}), json!([])),
            tool("add_to_cart", "sales.cart.add_to_cart", "write", "Add a product in a size and color to the cart.",
                 json!([{"name": "product_id", "many": false}]), json!([{"name": "cart_id", "many": false}]),
                 json!({"product_id": id("prd", "Product."), "size": {"type": "string", "description": "Size."},
                        "quantity": {"type": "integer", "minimum": 1, "description": "How many."}}),
                 json!(["product_id"])),
            tool("prepare_order", "sales.order.prepare_order", "write",
                 "Prepare an order from the cart; only the shopper can place it.",
                 json!([{"name": "cart_id", "many": false}]), json!([{"name": "preview_id", "many": false}]),
                 json!({"delivery_mode": {"type": "string", "enum": ["saved", "one_time"], "description": "Address kind."},
                        "client_total": money}),
                 json!(["delivery_mode", "client_total"])),
            tool("update_my_preferences", "customer.preference.update_my_preferences", "write",
                 "Remember what the shopper said they like.", json!([]), json!([{"name": "memory_id", "many": false}]),
                 json!({"add": {"type": "array", "items": {"type": "string", "description": "Fact."}, "description": "Facts to add."}}),
                 json!([])),
        ], "total": 11})
    }

    pub fn manifest() -> Vec<ToolSpec> {
        serde_json::from_value::<Manifest>(manifest_json())
            .map(|m| m.items)
            .unwrap_or_default()
    }

    pub fn get<'a>(m: &'a [ToolSpec], tool: &str) -> &'a ToolSpec {
        m.iter()
            .find(|t| t.tool == tool)
            .unwrap_or_else(|| panic!("no {tool} in the fixture"))
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::{get, manifest};

    #[test]
    fn reads_the_manifest_shape_and_finds_slot_fields() {
        let m = manifest();
        assert_eq!(m.len(), 11);
        let sale = get(&m, "set_sale_prices");
        assert_eq!(sale.domain(), "promotion");
        assert!(sale.is_write() && !sale.preview_only());
        assert_eq!(
            sale.field_for_slot("product_id"),
            Some(("product_ids".into(), true))
        );
        assert_eq!(
            sale.field_for_slot("category_id"),
            Some(("category_id".into(), false))
        );
        assert_eq!(sale.field_for_slot("tier_id"), None);
        assert!(get(&m, "prepare_order").preview_only());
        assert!(get(&m, "update_my_preferences").remembers());
        assert_eq!(
            get(&m, "create_coupon").required(),
            ["name", "promotion_id"]
        );
    }
}
