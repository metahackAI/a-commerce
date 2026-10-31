//! `generate_image` (docs/api/catalog.md#product): staged product images. No image generator is
//! connected (`content.generator: placeholder`): each image is a placeholder picture that says so,
//! made the same way every time from the product, the prompt and the style. Generated content
//! reaches shoppers once used, so a person always approves it (W$). Approved images are kept as
//! drafts marked as generated; they go on the product page only through `update_product_group`.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::ImageStyle;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::rules::ContentRules;
use crate::{
    BRAND_GUIDELINE_VIOLATION, GENERATION_FAILED, PRODUCT_NOT_FOUND, db, model, product_not_found,
    repo,
};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GenerateImageInput {
    /// The product (`search_product_groups`).
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_windbreaker")]
    pub product_id: String,
    /// What the image should show.
    #[serde(rename = "acom:prompt", alias = "prompt")]
    #[schemars(length(min = 1, max = 500), example = &"Folded into its own pocket on a trail map")]
    pub prompt: String,
    /// studio: plain background; lifestyle: worn, in a scene; flat_lay: laid flat from above.
    /// Default studio.
    #[serde(rename = "acom:style", alias = "style")]
    #[schemars(example = &"flat_lay")]
    pub style: Option<ImageStyle>,
    /// How many images, 1 to the store's `content.max_images_per_request`. Default 1.
    #[serde(rename = "acom:count", alias = "count")]
    #[schemars(range(min = 1), example = 2)]
    pub count: Option<u32>,
}

/// One image to make.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageDraft {
    pub image_id: String,
    pub content_url: String,
    pub caption: String,
}

/// What `generate_image` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImagePlan {
    pub product_id: String,
    pub prompt: String,
    pub style: String,
    pub images: Vec<ImageDraft>,
    pub at: DateTime<Utc>,
    /// The prompt word the brand guideline refuses, if any.
    pub blocked_word: Option<String>,
}

/// Generate staged images of a product.
#[derive(Debug)]
pub struct GenerateImage;

impl Operation for GenerateImage {
    const META: OpMeta = OpMeta {
        address: "catalog.product.generate_image",
        tool: "generate_image",
        group: Group::Offer,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/catalog/products/{product_id}/generate-image",
        summary: "Generate staged images of a product (studio, lifestyle or flat lay) from a prompt. A person approves them; they are kept as drafts marked as generated until used on the product page.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::many("image_id")],
        checks: &["content.brand_guideline", "content.ai_content_disclosure"],
        errors: &[
            GENERATION_FAILED,
            BRAND_GUIDELINE_VIOLATION,
            PRODUCT_NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = GenerateImageInput;
    type Output = Value;
}

/// Characters a `data:` URL may not carry as they are.
fn url_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '-' | '_' | '.' | '~' | '=' | ':' | '/' | ',' => {
                out.push(c);
            }
            _ => {
                let mut buf = [0u8; 4];
                for b in c.encode_utf8(&mut buf).bytes() {
                    out.push_str(&format!("%{b:02X}"));
                }
            }
        }
    }
    out
}

/// Text placed inside SVG markup.
fn xml_text(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The placeholder picture: a grey card that names the product, the style and the image id, and
/// says no generator made it. The same input gives the same picture.
pub(crate) fn placeholder_url(image_id: &str, product: &str, style: &str) -> String {
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"800\" height=\"1000\" viewBox=\"0 0 800 1000\">\
<rect width=\"800\" height=\"1000\" fill=\"#e7e5e0\"/>\
<text x=\"400\" y=\"470\" font-family=\"sans-serif\" font-size=\"34\" text-anchor=\"middle\" fill=\"#3d3b37\">{}</text>\
<text x=\"400\" y=\"520\" font-family=\"sans-serif\" font-size=\"24\" text-anchor=\"middle\" fill=\"#6b6862\">Placeholder · {} · {}</text>\
<text x=\"400\" y=\"560\" font-family=\"sans-serif\" font-size=\"20\" text-anchor=\"middle\" fill=\"#6b6862\">No image generator is connected</text>\
</svg>",
        xml_text(product),
        xml_text(&style.replace('_', " ")),
        xml_text(image_id)
    );
    format!("data:image/svg+xml,{}", url_escape(&svg))
}

/// The ids of `count` new images of a product, after the `existing` ones it has:
/// `prd_windbreaker` → `img_windbreaker_01`, `img_windbreaker_02`.
pub(crate) fn image_ids(product_id: &str, existing: i64, count: u32) -> Vec<String> {
    let base = product_id.trim_start_matches("prd_");
    (1..=i64::from(count))
        .map(|n| format!("img_{base}_{:02}", existing + n))
        .collect()
}

impl WriteOp<Cx> for GenerateImage {
    type Plan = ImagePlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: GenerateImageInput,
    ) -> Result<Planned<ImagePlan>, ToolError> {
        crate::operators_only(cx, "generate product images")?;
        let content = ContentRules::from_policy(&cx.policy)?;
        let prompt = input.prompt.trim().to_owned();
        if prompt.is_empty() {
            return Err(ToolError::invalid_input("Say what the image should show.")
                .detail(json!({"field": "acom:prompt", "reason": "missing"})));
        }
        let count = input.count.unwrap_or(1);
        if count == 0 || count > content.max_images_per_request {
            return Err(ToolError::invalid_input(format!(
                "Ask for 1–{} images at a time.",
                content.max_images_per_request
            ))
            .detail(json!({"field": "acom:count", "reason": "range",
                           "policy_key": "content.max_images_per_request",
                           "limit": content.max_images_per_request}))
            .next("generate_image"));
        }
        let style = input
            .style
            .unwrap_or(ImageStyle::Studio)
            .as_str()
            .to_owned();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let group = repo::group(&mut tx, &input.product_id)
            .await?
            .ok_or_else(|| {
                product_not_found("product", &input.product_id, "search_product_groups")
            })?;
        let existing: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM catalog.image_object WHERE tenant_id = $1 AND product_group_id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&group.id)
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let blocked = model::blocked_word(&prompt, &content.prompt_blocked_words)
            .filter(|_| content.brand_guideline_enforced);
        let images: Vec<ImageDraft> = image_ids(&group.id, existing, count)
            .into_iter()
            .map(|id| ImageDraft {
                content_url: placeholder_url(&id, &group.name, &style),
                caption: format!("{}: {prompt} ({})", group.name, style.replace('_', " ")),
                image_id: id,
            })
            .collect();
        let checks = vec![
            Check {
                rule: "content.brand_guideline".into(),
                passed: blocked.is_none(),
                detail: Some(match &blocked {
                    None => "The prompt uses no word the brand guideline rules out".into(),
                    Some(w) => format!("The brand guideline rules out \"{w}\" in prompts"),
                }),
                fix_tool: blocked.is_some().then(|| "generate_image".into()),
            },
            Check {
                rule: "content.ai_content_disclosure".into(),
                passed: true,
                detail: Some("Marked as generated (acom:ai_generated: true)".into()),
                fix_tool: None,
            },
        ];
        let changes = images
            .iter()
            .map(|i| Change {
                op: ChangeOp::Create,
                entity: "image".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"image_id": i.image_id, "product_id": group.id, "content_url": i.content_url,
                           "caption": i.caption, "style": style, "acom:ai_generated": true,
                           "generator": content.generator}),
                note: Some(
                    "A placeholder: no image generator is connected. Kept as a draft until used on the product page".into(),
                ),
            })
            .collect();
        Ok(Planned {
            changes,
            checks,
            basis: vec![Basis {
                entity: "catalog.product_group".into(),
                id: group.id.clone(),
                version: i64::from(group.version),
            }],
            approval: ApprovalNeed::Needed {
                reason: format!(
                    "Generated image{} of {} that shoppers would see",
                    if count == 1 { "" } else { "s" },
                    group.name
                ),
            },
            plan: ImagePlan {
                product_id: group.id,
                prompt,
                style,
                images,
                at: cx.clock.now(),
                blocked_word: blocked,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ImagePlan) -> Result<Committed, ToolError> {
        if let Some(w) = &plan.blocked_word {
            return Err(ToolError::new(
                BRAND_GUIDELINE_VIOLATION,
                format!("The store's brand guideline rules out \"{w}\" in image prompts."),
            )
            .detail(json!({"rule": "content.brand_guideline", "word": w}))
            .next("generate_image"));
        }
        for i in &plan.images {
            sqlx::query(
                "INSERT INTO catalog.image_object (tenant_id, id, product_group_id, content_url,
                   encoding_format, caption, prompt, image_style, is_ai_generated, date_created)
                 VALUES ($1, $2, $3, $4, 'image/svg+xml', $5, $6, $7, true, $8)
                 ON CONFLICT (tenant_id, id) DO NOTHING",
            )
            .bind(tx.tenant.as_str())
            .bind(&i.image_id)
            .bind(&plan.product_id)
            .bind(&i.content_url)
            .bind(&i.caption)
            .bind(&plan.prompt)
            .bind(&plan.style)
            .bind(plan.at)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
        }
        let ids: Vec<&str> = plan.images.iter().map(|i| i.image_id.as_str()).collect();
        let mut produced = Map::new();
        produced.insert("image_id".into(), json!(ids));
        Ok(Committed {
            id: ids.first().map(|s| (*s).to_owned()).unwrap_or_default(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ImagePlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let ids: Vec<String> = plan.images.iter().map(|i| i.image_id.clone()).collect();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let found: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM catalog.image_object WHERE tenant_id = $1 AND id = ANY($2)",
        )
        .bind(tx.tenant.as_str())
        .bind(&ids)
        .fetch_one(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = usize::try_from(found).is_ok_and(|n| n == ids.len());
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{found} of {} images were saved", ids.len())]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &ImagePlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        let ids: Vec<String> = plan.images.iter().map(|i| i.image_id.clone()).collect();
        sqlx::query("DELETE FROM catalog.image_object WHERE tenant_id = $1 AND id = ANY($2)")
            .bind(tx.tenant.as_str())
            .bind(&ids)
            .execute(&mut *tx.tx)
            .await
            .map_err(db)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing;

    #[test]
    fn placeholders_say_so_and_ids_follow_the_product() {
        assert_eq!(
            image_ids("prd_windbreaker", 0, 2),
            ["img_windbreaker_01", "img_windbreaker_02"]
        );
        assert_eq!(image_ids("prd_windbreaker", 2, 1), ["img_windbreaker_03"]);
        let url = placeholder_url("img_windbreaker_01", "Packable <windbreaker>", "flat_lay");
        assert!(url.starts_with("data:image/svg+xml,"));
        assert!(url.contains("Placeholder"));
        assert!(url.contains("No%20image%20generator%20is%20connected"));
        assert!(!url.contains('<') && !url.contains('#'));
        assert_eq!(
            url,
            placeholder_url("img_windbreaker_01", "Packable <windbreaker>", "flat_lay")
        );
    }

    #[tokio::test]
    async fn the_count_is_bounded_by_the_policy_and_shoppers_cannot_generate() {
        let op = testing::cx(acom_core::enums::ActorKind::Operator, "adm_sj");
        let input = |count| GenerateImageInput {
            product_id: "prd_windbreaker".into(),
            prompt: "On a trail map".into(),
            style: None,
            count: Some(count),
        };
        let e = GenerateImage.preview(&op, input(5)).await.unwrap_err();
        assert_eq!(e.code, "invalid_input");
        let e = GenerateImage
            .preview(&testing::shopper(), input(1))
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
        let schema = acom_contract::schema::closed_schema_for::<GenerateImageInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn two_windbreaker_images_wait_for_a_person_and_undo_cleanly() {
        let cx = testing::operator_db().await;
        let planned = GenerateImage
            .preview(
                &cx,
                GenerateImageInput {
                    product_id: "prd_windbreaker".into(),
                    prompt: "Folded into its own pocket on a trail map".into(),
                    style: Some(ImageStyle::FlatLay),
                    count: Some(2),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert!(matches!(planned.approval, ApprovalNeed::Needed { .. }));
        assert_eq!(planned.changes.len(), 2);
        assert_eq!(planned.changes[0].to["image_id"], "img_windbreaker_01");
        // The same request previews to the same images (approval previews it again).
        let again = GenerateImage
            .preview(
                &cx,
                GenerateImageInput {
                    product_id: "prd_windbreaker".into(),
                    prompt: "Folded into its own pocket on a trail map".into(),
                    style: Some(ImageStyle::FlatLay),
                    count: Some(2),
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(planned.changes, again.changes);
        let mut tx = cx.begin().await.unwrap_or_else(|e| panic!("{e}"));
        let done = GenerateImage
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(done.produced["image_id"][1], "img_windbreaker_02");
        GenerateImage
            .undo(&mut tx, &planned.plan, &done)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));
    }
}
