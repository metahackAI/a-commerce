//! `get_curation` (docs/api/merchandising.md#collection_page).

use std::collections::BTreeMap;

use acom_catalog::service::for_merchandising::{ProductCard, product_cards};
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_core::enums::DisplayStatus;
use acom_store::Cx;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::{COLLECTION_PAGE_NOT_FOUND, not_found, page, page_summary};
use crate::display_slot::{View, hidden_reason, view};
use crate::model::DisplayRules;
use crate::{audience, check_id, pick};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GetCurationInput {
    /// The curated page.
    #[schemars(regex(pattern = r"^col_[a-z0-9_]+$"), example = &"col_winter_edit")]
    pub collection_page_id: String,
    /// Only these fields (`id` always comes back).
    #[schemars(example = ["name", "status"])]
    pub fields: Option<Vec<String>>,
}

pub struct GetCuration;

impl Operation for GetCuration {
    const META: OpMeta = OpMeta {
        address: "merchandising.collection_page.get",
        tool: "get_curation",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/merchandising/collection-pages/{collection_page_id}",
        summary: "One curated page: sections and their products, linked banners, search keywords, its state, and which products are hidden now (sold out or not for sale).",
        consumes: &[Slot::one("collection_page_id")],
        produces: &[],
        checks: &[],
        errors: &[COLLECTION_PAGE_NOT_FOUND],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = GetCurationInput;
    type Output = Value;
}

impl ReadOp<Cx> for GetCuration {
    async fn read(&self, cx: &Cx, input: GetCurationInput) -> Result<Value, ToolError> {
        check_id("collection_page_id", "col", &input.collection_page_id)?;
        let rules = DisplayRules::from_policy(&cx.policy)?;
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let view = view(cx, &mut tx).await?;
        let p = page(&mut tx, &input.collection_page_id)
            .await?
            .ok_or_else(|| not_found(&input.collection_page_id))?;
        // A shopper sees a page only while it shows to them; drafts and other tiers' pages do
        // not exist for them.
        if let View::Shopper(tier) = &view {
            let to = audience(p.valid_for_member_tier.as_deref());
            if p.status(now) != DisplayStatus::Active || (to != "everyone" && to != tier) {
                return Err(not_found(&p.id));
            }
        }
        let products = p.products();
        let cards: BTreeMap<String, ProductCard> = product_cards(&mut tx, &products)
            .await?
            .into_iter()
            .map(|c| (c.product_id.clone(), c))
            .collect();
        tx.commit().await.map_err(ToolError::from)?;

        let hidden: Vec<Value> = products
            .iter()
            .filter_map(|id| {
                hidden_reason(&rules, cards.get(id)).map(|r| json!({"product_id": id, "reason": r}))
            })
            .collect();
        let mut out = page_summary(cx, &p, now);
        out["has_part"] = json!(p.sections());
        out["hidden_items"] = json!(hidden);
        Ok(pick(out, input.fields.as_deref()))
    }
}
