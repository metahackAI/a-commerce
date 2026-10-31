//! `update_curation` (docs/api/merchandising.md#collection_page).
//!
//! Changes a draft's or a published page's sections, products, banners, keywords, name or
//! audience. Its period and status change only through `publish_curation`. Changing a page
//! shoppers can see (scheduled or showing) reaches them, so the store's `reaches_customers`
//! rule asks a person to approve it.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::DisplayStatus;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::create_curation::content_json;
use super::{
    ALREADY_ENDED, BANNER_NOT_FOUND, COLLECTION_PAGE_NOT_FOUND, PRODUCT_DISCONTINUED, PageRow,
    Section, clean_banners, clean_keywords, clean_sections, content_checks, known_content,
    not_found, page, still_on_sale, write_content,
};
use crate::model::DisplayRules;
use crate::{TIER_NOT_FOUND, audience, check_id, known_tier};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UpdateCurationInput {
    /// The curated page (`list_curations`).
    #[schemars(regex(pattern = r"^col_[a-z0-9_]+$"), example = &"col_winter_edit")]
    pub collection_page_id: String,
    /// New title.
    #[schemars(length(min = 1, max = 100), example = &"Winter coats edit")]
    pub name: Option<String>,
    /// The sections in order, replacing the current ones.
    #[schemars(length(min = 1, max = 20), example = json!([{"name": "Coats", "item_list_element": ["prd_wool_overcoat", "prd_chore_coat"]}]))]
    pub has_part: Option<Vec<Section>>,
    /// Who the page is for: a tier id (`list_tiers`) or `everyone`.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"vip")]
    pub audience: Option<String>,
    /// Banners linked to the page, replacing the current ones; an empty list unlinks all.
    #[schemars(example = ["bnr_fall_layers"])]
    pub banner_ids: Option<Vec<String>>,
    /// Search words, replacing the current ones.
    #[schemars(example = ["winter coat", "wool coat"])]
    pub keywords: Option<Vec<String>>,
}

/// What `update_curation` writes, with the page as it was.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateCurationPlan {
    pub page: PageRow,
    pub before: PageRow,
}

/// Change a curated page's content.
#[derive(Debug)]
pub struct UpdateCuration;

impl Operation for UpdateCuration {
    const META: OpMeta = OpMeta {
        address: "merchandising.collection_page.update",
        tool: "update_curation",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Patch,
        path: "/v1/merchandising/collection-pages/{collection_page_id}",
        summary: "Change a draft or scheduled curated page: sections and products, linked banners, keywords, name or audience. Publishing and the period are publish_curation's. Changing a page shoppers can see needs a person's approval.",
        consumes: &[Slot::one("collection_page_id")],
        produces: &[Slot::one("collection_page_id")],
        checks: &["display.exclude_discontinued", "curation.no_empty_section"],
        errors: &[
            COLLECTION_PAGE_NOT_FOUND,
            PRODUCT_DISCONTINUED,
            ALREADY_ENDED,
            BANNER_NOT_FOUND,
            TIER_NOT_FOUND,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = UpdateCurationInput;
    type Output = Value;
}

fn ended(id: &str) -> ToolError {
    ToolError::new(
        ALREADY_ENDED,
        format!("{id} has ended; make a new page instead."),
    )
    .detail(json!({"entity": "collection_page", "id": id, "from": "ended"}))
    .next("create_curation")
}

impl WriteOp<Cx> for UpdateCuration {
    type Plan = UpdateCurationPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: UpdateCurationInput,
    ) -> Result<Planned<UpdateCurationPlan>, ToolError> {
        check_id("collection_page_id", "col", &input.collection_page_id)?;
        let rules = DisplayRules::from_policy(&cx.policy)?;
        let sections = input.has_part.map(clean_sections).transpose()?;
        let banners = input.banner_ids.map(clean_banners).transpose()?;
        let now = cx.clock.now();

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let old = page(&mut tx, &input.collection_page_id)
            .await?
            .ok_or_else(|| not_found(&input.collection_page_id))?;
        if old.status(now) == DisplayStatus::Ended {
            return Err(ended(&old.id));
        }
        let tier = match &input.audience {
            Some(a) => Some(known_tier(&mut tx, a).await?.map(|t| t.id)),
            None => None,
        };
        let new = PageRow {
            name: input
                .name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map_or_else(|| old.name.clone(), str::to_owned),
            has_part: sections
                .as_ref()
                .map_or_else(|| old.has_part.clone(), |s| json!(s)),
            valid_for_member_tier: tier.unwrap_or_else(|| old.valid_for_member_tier.clone()),
            banner_id: banners.unwrap_or_else(|| old.banner_id.clone()),
            keywords: input
                .keywords
                .map_or_else(|| old.keywords.clone(), clean_keywords),
            version: 0,
            ..old.clone()
        };
        let new_sections = new.sections();
        let cards = known_content(&mut tx, &new_sections, &new.banner_id).await?;
        tx.commit().await.map_err(ToolError::from)?;

        if new.unversioned() == old.unversioned() {
            return Err(ToolError::invalid_input(format!(
                "Nothing to change: {} already has these values.",
                old.id
            ))
            .detail(json!({"field": "collection_page_id", "reason": "missing"})));
        }
        let published = old.published(now);
        let facts = json!({"reaches_customers": published && rules.reaches(Self::META.tool)});
        let approval = if cx.policy.approval_for(Self::META.tool, &facts).needed {
            ApprovalNeed::Needed {
                reason: format!(
                    "Changes {}, which shoppers {} ({})",
                    old.name,
                    if old.status(now) == DisplayStatus::Active {
                        "see now"
                    } else {
                        "will see when it starts"
                    },
                    audience(new.valid_for_member_tier.as_deref())
                ),
            }
        } else {
            ApprovalNeed::NotNeeded
        };
        let mut to = content_json(cx, &new);
        if sections.is_some() {
            to["has_part"] = new.has_part.clone();
        }
        let mut from = content_json(cx, &old);
        if sections.is_some() {
            from["has_part"] = old.has_part.clone();
        }
        Ok(Planned {
            checks: content_checks(&rules, &new_sections, &cards, Self::META.tool),
            changes: vec![Change {
                op: ChangeOp::Update,
                entity: "collection_page".into(),
                id: old.id.clone(),
                from,
                to,
                note: published.then(|| {
                    format!(
                        "Published page ({}); shoppers see the change",
                        old.status(now).as_str()
                    )
                }),
            }],
            basis: vec![Basis {
                entity: "merchandising.collection_page".into(),
                id: old.id.clone(),
                version: i64::from(old.version),
            }],
            approval,
            plan: UpdateCurationPlan {
                page: new,
                before: old.unversioned(),
            },
        })
    }

    async fn commit(
        &self,
        tx: &mut TxCx,
        plan: &UpdateCurationPlan,
    ) -> Result<Committed, ToolError> {
        let rules = DisplayRules::from_policy(&tx.policy)?;
        let now = tx.clock.now();
        match page(tx, &plan.page.id).await? {
            None => return Err(not_found(&plan.page.id)),
            Some(p) if p.status(now) == DisplayStatus::Ended => return Err(ended(&p.id)),
            Some(_) => {}
        }
        still_on_sale(tx, &rules, &plan.page.products(), "update_curation").await?;
        write_content(tx, &plan.page).await?;
        Ok(Committed {
            id: plan.page.id.clone(),
            produced: Map::from_iter([("collection_page_id".to_owned(), json!(plan.page.id))]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &UpdateCurationPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let stored = page(&mut tx, &plan.page.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let p = &plan.page;
        let matched = stored.is_some_and(|s| {
            s.name == p.name
                && s.has_part == p.has_part
                && s.valid_for_member_tier == p.valid_for_member_tier
                && s.banner_id == p.banner_id
                && s.keywords == p.keywords
        });
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!(
                    "curated page {} does not read back as planned",
                    p.id
                )]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &UpdateCurationPlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        write_content(tx, &plan.before).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_spec_input_with_a_closed_schema() {
        let i: UpdateCurationInput = serde_json::from_value(json!({
            "collection_page_id": "col_winter_edit", "name": "Winter coats edit",
            "has_part": [{"name": "Coats", "item_list_element": ["prd_wool_overcoat"]}],
            "audience": "everyone", "banner_ids": [], "keywords": ["winter coat"]}))
        .unwrap();
        assert_eq!(i.banner_ids, Some(vec![]));
        assert!(
            serde_json::from_value::<UpdateCurationInput>(json!({
                "collection_page_id": "col_winter_edit", "status": "active"}))
            .is_err()
        );
        let schema = acom_contract::schema::closed_schema_for::<UpdateCurationInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert_eq!(UpdateCuration::META.kind, Kind::Write);
    }
}
