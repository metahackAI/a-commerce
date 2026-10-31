//! `create_curation` (docs/api/merchandising.md#collection_page).
//!
//! Makes a curated page as a `draft`. Nothing shows to shoppers until `publish_curation`.

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Committed, ErrorCode, Group, Kind, Method, OpMeta,
    Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::DisplayStatus;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    BANNER_NOT_FOUND, DUPLICATE_COLLECTION_PAGE, PRODUCT_DISCONTINUED, PageRow, Section,
    clean_banners, clean_keywords, clean_sections, content_checks, delete_draft, insert,
    known_content, page, still_on_sale, taken_ids,
};
use crate::model::{DisplayRules, id_candidates};
use crate::{INVALID_PERIOD, TIER_NOT_FOUND, known_tier, parse_time, policy_approval};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateCurationInput {
    /// The page's title shoppers see; its id is made from it (`col_winter_outerwear_edit`).
    #[schemars(length(min = 1, max = 100), example = &"Winter outerwear edit")]
    pub name: String,
    /// The sections in order, each a heading and its products.
    #[schemars(length(min = 1, max = 20), example = json!([{"name": "Coats", "item_list_element": ["prd_wool_overcoat"]}]))]
    pub has_part: Vec<Section>,
    /// Who the page is for: a tier id (`list_tiers`) or `everyone`. Default: everyone.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"everyone")]
    pub audience: Option<String>,
    /// Planned start (store time without an offset); fixed when the page is published.
    #[schemars(example = &"2026-11-20T00:00")]
    pub valid_from: Option<String>,
    /// Planned end (inclusive); fixed when the page is published.
    #[schemars(example = &"2026-11-30T23:59")]
    pub valid_through: Option<String>,
    /// Banners linked to the page (`list_banners`).
    #[schemars(example = ["bnr_fall_layers"])]
    pub banner_ids: Option<Vec<String>>,
    /// Search words that lead to the page.
    #[schemars(example = ["winter coat"])]
    pub keywords: Option<Vec<String>>,
}

/// What `create_curation` writes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreateCurationPlan {
    pub page: PageRow,
}

/// Make a curated page as a draft.
#[derive(Debug)]
pub struct CreateCuration;

impl Operation for CreateCuration {
    const META: OpMeta = OpMeta {
        address: "merchandising.collection_page.create",
        tool: "create_curation",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/merchandising/collection-pages",
        summary: "Make a curated page (a collection of products in named sections, with linked banners and search keywords) as a draft. Shoppers see it only after publish_curation.",
        consumes: &[Slot::many("product_id")],
        produces: &[Slot::one("collection_page_id")],
        checks: &["display.exclude_discontinued", "curation.no_empty_section"],
        errors: &[
            PRODUCT_DISCONTINUED,
            BANNER_NOT_FOUND,
            TIER_NOT_FOUND,
            INVALID_PERIOD,
            DUPLICATE_COLLECTION_PAGE,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = CreateCurationInput;
    type Output = Value;
}

/// The page's content in a preview's `to`.
pub(crate) fn content_json(cx: &Cx, p: &PageRow) -> Value {
    json!({
        "name": p.name,
        "audience": crate::audience(p.valid_for_member_tier.as_deref()),
        "valid_from": p.valid_from.map(|t| cx.time.format(t)),
        "valid_through": p.valid_through.map(|t| cx.time.format(t)),
        "section_count": p.sections().len(),
        "product_count": p.products().len(),
        "banner_ids": p.banner_id,
        "keywords": p.keywords,
    })
}

impl WriteOp<Cx> for CreateCuration {
    type Plan = CreateCurationPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: CreateCurationInput,
    ) -> Result<Planned<CreateCurationPlan>, ToolError> {
        let rules = DisplayRules::from_policy(&cx.policy)?;
        let name = input.name.trim().to_owned();
        if name.is_empty() {
            return Err(ToolError::invalid_input("A curated page needs a name.")
                .detail(json!({"field": "name", "reason": "missing"})));
        }
        let sections = clean_sections(input.has_part)?;
        let banners = clean_banners(input.banner_ids.unwrap_or_default())?;
        let keywords = clean_keywords(input.keywords.unwrap_or_default());
        let from = input
            .valid_from
            .as_deref()
            .map(|s| parse_time(&cx.time, "valid_from", s))
            .transpose()?;
        let through = input
            .valid_through
            .as_deref()
            .map(|s| parse_time(&cx.time, "valid_through", s))
            .transpose()?;
        if let (Some(f), Some(t)) = (from, through)
            && t <= f
        {
            return Err(ToolError::new(
                INVALID_PERIOD,
                "`valid_through` must come after `valid_from`.",
            )
            .detail(json!({"field": "valid_through", "reason": "range"}))
            .next("create_curation"));
        }
        let audience = input.audience.as_deref().unwrap_or("everyone");

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let tier = known_tier(&mut tx, audience).await?;
        let cards = known_content(&mut tx, &sections, &banners).await?;
        let candidates = id_candidates("col", &name);
        let taken = taken_ids(&mut tx, &candidates).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let id = candidates
            .into_iter()
            .find(|c| !taken.contains(c))
            .unwrap_or_else(|| format!("col_{}", acom_core::id::new_suffix()));

        let row = PageRow {
            id: id.clone(),
            name,
            has_part: json!(sections),
            valid_for_member_tier: tier.map(|t| t.id),
            valid_from: from,
            valid_through: through,
            display_status: DisplayStatus::Draft.as_str().to_owned(),
            sales_channel: None,
            banner_id: banners,
            keywords,
            version: 0,
        };
        let mut to = content_json(cx, &row);
        to["collection_page_id"] = json!(id);
        to["status"] = json!("draft");
        let mut changes = vec![Change {
            op: ChangeOp::Create,
            entity: "collection_page".into(),
            id: String::new(),
            from: Value::Null,
            to,
            note: None,
        }];
        changes.extend(sections.iter().map(|s| Change {
            op: ChangeOp::Create,
            entity: "item_list".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({"collection_page_id": id, "name": s.name,
                       "item_list_element": s.item_list_element}),
            note: None,
        }));
        Ok(Planned {
            checks: content_checks(&rules, &sections, &cards, Self::META.tool),
            changes,
            basis: Vec::new(),
            approval: policy_approval(
                cx,
                Self::META.tool,
                &json!({"reaches_customers": rules.reaches(Self::META.tool)}),
            ),
            plan: CreateCurationPlan { page: row },
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &CreateCurationPlan,
    ) -> Result<Committed, ToolError> {
        let rules = DisplayRules::from_policy(&tx.policy)?;
        let p = &plan.page;
        still_on_sale(tx, &rules, &p.products(), "search_products").await?;
        if !insert(tx, p).await? {
            // Taken since the preview, unless this very plan ran already.
            let same = page(tx, &p.id)
                .await?
                .is_some_and(|s| s.unversioned() == *p);
            if !same {
                return Err(ToolError::new(
                    DUPLICATE_COLLECTION_PAGE,
                    format!("Another page took the id {} since the preview.", p.id),
                )
                .detail(json!({"entity": "collection_page", "field": "id", "existing_id": p.id}))
                .next("create_curation"));
            }
        }
        Ok(Committed {
            id: p.id.clone(),
            produced: Map::from_iter([("collection_page_id".to_owned(), json!(p.id))]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &CreateCurationPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let stored = page(&mut tx, &plan.page.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = stored.is_some_and(|s| s.unversioned() == plan.page);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!(
                    "curated page {} does not read back as planned",
                    plan.page.id
                )]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &CreateCurationPlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        if delete_draft(tx, &plan.page.id).await? == 0 && page(tx, &plan.page.id).await?.is_some() {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!(
                    "{} was published since; undo the publication first.",
                    plan.page.id
                ),
            )
            .detail(json!({"entity": "collection_page", "id": plan.page.id})));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_spec_input_with_a_closed_schema() {
        let i: CreateCurationInput = serde_json::from_value(json!({
            "name": "Winter outerwear edit",
            "has_part": [{"name": "Coats", "item_list_element": ["prd_wool_overcoat"]}],
            "audience": "everyone", "banner_ids": [], "keywords": ["winter coat"]}))
        .unwrap();
        assert_eq!(i.has_part[0].item_list_element, ["prd_wool_overcoat"]);
        assert_eq!(
            id_candidates("col", &i.name)[0],
            "col_winter_outerwear_edit"
        );
        assert!(
            serde_json::from_value::<CreateCurationInput>(json!({
                "name": "x", "has_part": [{"name": "Coats", "item_list_element": [], "rows": 2}]}))
            .is_err()
        );
        let schema = acom_contract::schema::closed_schema_for::<CreateCurationInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert_eq!(CreateCuration::META.kind, Kind::Write);
    }
}
