//! `generate_page_image` (docs/api/catalog.md#page_image): a generated picture for a store page
//! that shows no single product, such as the top of a curated page. With an image model
//! connected (`IMAGE_MODEL`) the picture is made at preview, so the person approving sees it;
//! without one it is a placeholder that says so. Generated content reaches shoppers once used, so
//! a person always approves it (W$). Approved pictures go on a page through `create_curation` or
//! `update_curation` (`image_id`).

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Change, ChangeOp, Check, Committed, Group, Kind, Method,
    OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{
    NewPageImage, content_url, image_id, image_model, insert, placeholder, row, set_approved,
};
use crate::rules::ContentRules;
use crate::{BRAND_GUIDELINE_VIOLATION, GENERATION_FAILED, model};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GeneratePageImageInput {
    /// What the picture should show: subject, setting, light, mood. The store adds the shape
    /// and what to leave out (policy `content.page_image.prompt`).
    #[serde(rename = "acom:prompt", alias = "prompt")]
    #[schemars(length(min = 1, max = 500), example = &"Wool coats and knit scarves on a light grey backdrop, soft winter daylight")]
    pub prompt: String,
    /// The alternative text shown with the picture. Default: the prompt.
    #[schemars(length(min = 1, max = 200), example = &"Wool coats and scarves in winter light")]
    pub caption: Option<String>,
}

/// What `generate_page_image` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageImagePlan {
    pub image_id: String,
    pub prompt: String,
    pub caption: String,
    pub generator: String,
    pub at: DateTime<Utc>,
    /// The prompt word the brand guideline refuses, if any (then no picture was made).
    pub blocked_word: Option<String>,
}

/// Generate a picture for a store page.
#[derive(Debug)]
pub struct GeneratePageImage;

impl Operation for GeneratePageImage {
    const META: OpMeta = OpMeta {
        address: "catalog.page_image.generate",
        tool: "generate_page_image",
        group: Group::Offer,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/catalog/page-images",
        summary: "Generate a wide picture for a store page that shows no single product, such as the top of a curated page. The picture is made now so a person sees it before approving; use it with create_curation or update_curation (image_id).",
        consumes: &[],
        produces: &[Slot::one("image_id")],
        checks: &["content.brand_guideline", "content.ai_content_disclosure"],
        errors: &[GENERATION_FAILED, BRAND_GUIDELINE_VIOLATION],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = GeneratePageImageInput;
    type Output = Value;
}

/// The operator's prompt with what the store adds to every page picture.
fn full_prompt(prompt: &str, store_adds: &str) -> String {
    let prompt = prompt.trim_end_matches(['.', ' ']);
    if store_adds.trim().is_empty() {
        prompt.to_owned()
    } else {
        format!("{prompt}. {}", store_adds.trim())
    }
}

impl WriteOp<Cx> for GeneratePageImage {
    type Plan = PageImagePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: GeneratePageImageInput,
    ) -> Result<Planned<PageImagePlan>, ToolError> {
        crate::operators_only(cx, "generate page images")?;
        let content = ContentRules::from_policy(&cx.policy)?;
        let prompt = input.prompt.trim().to_owned();
        if prompt.is_empty() {
            return Err(
                ToolError::invalid_input("Say what the picture should show.")
                    .detail(json!({"field": "acom:prompt", "reason": "missing"})),
            );
        }
        let caption = input
            .caption
            .map(|c| c.trim().to_owned())
            .filter(|c| !c.is_empty())
            .unwrap_or_else(|| prompt.chars().take(200).collect());
        let blocked = model::blocked_word(&prompt, &content.prompt_blocked_words)
            .filter(|_| content.brand_guideline_enforced);
        let generator = image_model().map_or("placeholder", |m| m.model.as_str());
        let full = full_prompt(&prompt, &content.page_image_prompt);
        let id = image_id(cx.tenant.as_str(), &full, generator);
        let at = cx.clock.now();

        if blocked.is_none() {
            let mut tx = cx.read().await.map_err(ToolError::from)?;
            let found = row(&mut tx, &id).await?;
            tx.commit().await.map_err(ToolError::from)?;
            if found.is_none() {
                let picture = match image_model() {
                    Some(m) => m.generate(&full).await?,
                    None => placeholder(&prompt),
                };
                let mut tx = cx.begin().await.map_err(ToolError::from)?;
                insert(
                    &mut tx,
                    &NewPageImage {
                        id: &id,
                        prompt: &prompt,
                        caption: &caption,
                        generator,
                        picture: &picture,
                        at,
                    },
                )
                .await?;
                tx.commit().await.map_err(ToolError::from)?;
            }
        }

        let checks = vec![
            Check {
                rule: "content.brand_guideline".into(),
                passed: blocked.is_none(),
                detail: Some(match &blocked {
                    None => "The prompt uses no word the brand guideline rules out".into(),
                    Some(w) => format!("The brand guideline rules out \"{w}\" in prompts"),
                }),
                fix_tool: blocked.is_some().then(|| "generate_page_image".into()),
            },
            Check {
                rule: "content.ai_content_disclosure".into(),
                passed: true,
                detail: Some("Marked as generated (acom:ai_generated: true)".into()),
                fix_tool: None,
            },
        ];
        let changes = vec![Change {
            op: ChangeOp::Create,
            entity: "page_image".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({"image_id": id,
                       "content_url": blocked.is_none().then(|| content_url(&id)),
                       "caption": caption, "acom:ai_generated": true, "generator": generator}),
            note: Some(if generator == "placeholder" {
                "A placeholder: no image model is connected. Shoppers see it only once a page uses it".into()
            } else {
                "Made now so you can see it. Shoppers see it only after approval, once a page uses it".into()
            }),
        }];
        Ok(Planned {
            changes,
            checks,
            basis: Vec::new(),
            approval: ApprovalNeed::Needed {
                reason: "A generated picture that shoppers would see".into(),
            },
            plan: PageImagePlan {
                image_id: id,
                prompt,
                caption,
                generator: generator.to_owned(),
                at,
                blocked_word: blocked,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &PageImagePlan) -> Result<Committed, ToolError> {
        if let Some(w) = &plan.blocked_word {
            return Err(ToolError::new(
                BRAND_GUIDELINE_VIOLATION,
                format!("The store's brand guideline rules out \"{w}\" in image prompts."),
            )
            .detail(json!({"rule": "content.brand_guideline", "word": w}))
            .next("generate_page_image"));
        }
        if !set_approved(tx, &plan.image_id, Some(plan.at)).await? {
            return Err(ToolError::new(
                GENERATION_FAILED,
                format!(
                    "The picture {} is no longer there (the store was reloaded). Generate it again.",
                    plan.image_id
                ),
            )
            .detail(json!({"entity": "page_image", "id": plan.image_id, "retryable": false}))
            .next("generate_page_image"));
        }
        let mut produced = Map::new();
        produced.insert("image_id".into(), json!(plan.image_id));
        Ok(Committed {
            id: plan.image_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &PageImagePlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let found = row(&mut tx, &plan.image_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = found.is_some_and(|r| r.approved_at.is_some());
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} is not approved", plan.image_id)]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &PageImagePlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        set_approved(tx, &plan.image_id, None).await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[test]
    fn the_store_adds_its_part_to_the_prompt() {
        assert_eq!(
            full_prompt("Coats on grey. ", "Wide 3:1 banner."),
            "Coats on grey. Wide 3:1 banner."
        );
        assert_eq!(full_prompt("Coats on grey", " "), "Coats on grey");
    }

    #[tokio::test]
    async fn shoppers_cannot_generate_and_the_schema_is_closed() {
        let e = GeneratePageImage
            .preview(
                &testing::shopper(),
                GeneratePageImageInput {
                    prompt: "Coats".into(),
                    caption: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
        let schema = acom_contract::schema::closed_schema_for::<GeneratePageImageInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn a_placeholder_picture_waits_for_a_person_and_previews_the_same() {
        let cx = testing::operator_db().await;
        let input = || GeneratePageImageInput {
            prompt: "Wool coats on a light grey backdrop".into(),
            caption: None,
        };
        let planned = GeneratePageImage
            .preview(&cx, input())
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(planned.approval, ApprovalNeed::Needed { .. }));
        let id = planned.plan.image_id.clone();
        assert!(id.starts_with("img_page_"));
        assert_eq!(planned.changes[0].to["content_url"], content_url(&id));
        let again = GeneratePageImage
            .preview(&cx, input())
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(planned.changes, again.changes);
        let mut tx = cx.begin().await.unwrap_or_else(|e| panic!("{e}"));
        assert!(
            super::super::page_image_content(&mut tx, &id, false)
                .await
                .unwrap_or_else(|e| panic!("{e}"))
                .is_none()
        );
        let done = GeneratePageImage
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(done.produced["image_id"], json!(id));
        let (format, _) = super::super::page_image_content(&mut tx, &id, false)
            .await
            .unwrap_or_else(|e| panic!("{e}"))
            .unwrap_or_else(|| panic!("approved picture not served"));
        assert_eq!(format, "image/svg+xml");
        GeneratePageImage
            .undo(&mut tx, &planned.plan, &done)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));
    }
}
