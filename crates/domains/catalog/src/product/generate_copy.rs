//! `generate_copy` (docs/api/catalog.md#product): a draft of a product's title, summary,
//! description or search text. No writing model is connected (`content.generator: placeholder`):
//! the draft is built, the same way every time, from the product's own data (name, material,
//! colors, sizes, how it packs and wears) in the shape of the store's tone rules
//! (`content.copy`: sentences and a care line). It is content, so it needs no approval; it is
//! saved as a draft and goes on the product page only through `update_product_group`.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::CopyType;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::model::{self, Variant};
use crate::rules::{ContentRules, CopyRules};
use crate::{GENERATION_FAILED, PRODUCT_NOT_FOUND, db, product_not_found, repo};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GenerateCopyInput {
    /// The product (`search_product_groups`).
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_windbreaker")]
    pub product_id: String,
    /// Which text: title (the product name); summary (one line); description (the store's
    /// sentences and a care line); seo (the text search results show).
    #[serde(rename = "acom:copy_type", alias = "copy_type")]
    #[schemars(example = &"description")]
    pub copy_type: CopyType,
    /// The tone to write in. Default: the store's tone (`content.copy.tone`).
    #[serde(rename = "acom:tone", alias = "tone")]
    #[schemars(length(min = 1, max = 300), example = &"Plain and warm")]
    pub tone: Option<String>,
    /// Words the text should use (title and seo take them in).
    #[serde(rename = "acom:keywords", alias = "keywords")]
    #[schemars(example = ["packable", "travel"])]
    pub keywords: Option<Vec<String>>,
}

/// What the copy is built from.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct CopyFacts {
    pub name: String,
    pub brand: Option<String>,
    pub material: Option<String>,
    pub colors: Vec<String>,
    pub sizes: Vec<String>,
    /// Phrases on how it packs and wears, in order: "packs into its own pocket".
    pub wear: Vec<String>,
}

/// A packing or warmth word of `additional_property` as a phrase.
fn phrase(name: &str, value: &Value) -> Option<String> {
    let word = value.as_str().map(|w| w.replace('_', " "));
    match (name, word.as_deref()) {
        ("packing", Some("none")) | ("warmth", None) | ("packing", None) => None,
        ("packing", Some("own pocket")) => Some("packs into its own pocket".into()),
        ("packing", Some(w)) => Some(w.to_owned()),
        ("warmth", Some("wind layer")) => Some("blocks the wind".into()),
        ("warmth", Some(w)) => Some(format!("{w} warmth")),
        ("rain", _) if value.as_bool() == Some(true) => Some("sheds light rain".into()),
        ("packed_size", _) => value
            .as_array()
            .filter(|a| a.len() == 2)
            .map(|a| format!("packs down to {} × {} cm", a[0], a[1])),
        _ => None,
    }
}

impl CopyFacts {
    pub(crate) fn from_rows(
        group: &repo::Group,
        brand: Option<String>,
        variants: &[Variant],
    ) -> Self {
        let mut colors: Vec<String> = Vec::new();
        let mut ordered = variants.to_vec();
        model::order_variants(&mut ordered);
        let mut sizes: Vec<String> = Vec::new();
        for v in &ordered {
            if let Some(c) = &v.color
                && !colors.contains(c)
            {
                colors.push(c.clone());
            }
            if let Some(s) = &v.size
                && !sizes.contains(s)
            {
                sizes.push(s.clone());
            }
        }
        let wear = group
            .additional_property
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|p| phrase(p.get("name")?.as_str()?, p.get("value")?))
            .collect();
        Self {
            name: group.name.clone(),
            brand,
            material: group.material.clone().filter(|m| !m.trim().is_empty()),
            colors,
            sizes,
            wear,
        }
    }

    fn what(&self) -> String {
        match &self.material {
            Some(m) => format!("{} in {}", self.name, m.to_lowercase()),
            None => self.name.clone(),
        }
    }

    fn range(&self) -> Option<String> {
        match self.sizes.as_slice() {
            [] => None,
            [one] => Some(format!("size {one}")),
            [first, .., last] => Some(format!("sizes {first}–{last}")),
        }
    }
}

/// A list in words: "a", "a and b", "a, b and c".
fn and_list(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Cuts text to `max` characters at a word boundary.
fn fit(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out = String::new();
    for word in text.split_whitespace() {
        let next = if out.is_empty() {
            word.to_owned()
        } else {
            format!("{out} {word}")
        };
        if next.chars().count() > max {
            break;
        }
        out = next;
    }
    out.trim_end_matches([',', ';', ':', '.', '—', '-', ' '])
        .to_owned()
}

/// The draft text, the same for the same product, type, rules and keywords.
pub(crate) fn write_copy(
    f: &CopyFacts,
    copy_type: CopyType,
    copy: &CopyRules,
    content: &ContentRules,
    keywords: &[String],
) -> String {
    let what = f.what();
    let wear = and_list(&f.wear);
    match copy_type {
        CopyType::Title => {
            let mut title = what;
            if let Some(k) = keywords.first() {
                title = format!("{title}, {k}");
            }
            fit(&title, content.title_max_chars)
        }
        CopyType::Summary => {
            if wear.is_empty() {
                format!("{what}.")
            } else {
                format!("{what}: {wear}.")
            }
        }
        CopyType::Description => {
            let mut sentences = vec![match &f.brand {
                Some(b) => format!(
                    "The {} from {b}, in {}.",
                    f.name.to_lowercase(),
                    material_or(f)
                ),
                None => format!("The {}, in {}.", f.name.to_lowercase(), material_or(f)),
            }];
            if !wear.is_empty() {
                sentences.push(format!("It {wear}."));
            }
            let options = match (f.colors.len(), f.range()) {
                (0 | 1, Some(r)) => Some(format!("Comes in {r}.")),
                (n, Some(r)) => Some(format!("Comes in {n} colors, {r}.")),
                (n, None) if n > 1 => Some(format!("Comes in {}.", and_list(&f.colors))),
                _ => None,
            };
            sentences.extend(options);
            let n = usize::try_from(copy.sentences).unwrap_or(usize::MAX);
            sentences.truncate(n.max(1));
            let mut text = sentences.join(" ");
            if copy.care_line {
                text.push_str("\nCare: follow the label on the garment.");
            }
            text
        }
        CopyType::Seo => {
            let mut parts = vec![format!("{what}.")];
            if !wear.is_empty() {
                let mut w = wear.clone();
                if let Some(first) = w.get(0..1) {
                    w.replace_range(0..1, &first.to_uppercase());
                }
                parts.push(format!("{w}."));
            }
            if let Some(r) = f.range() {
                parts.push(format!("In {r}."));
            }
            if !keywords.is_empty() {
                parts.push(keywords.join(", "));
            }
            fit(&parts.join(" "), content.seo_max_chars)
        }
    }
}

fn material_or(f: &CopyFacts) -> String {
    f.material
        .as_deref()
        .map_or_else(|| "one piece".into(), str::to_lowercase)
}

/// What `generate_copy` runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CopyPlan {
    pub copy_draft_id: String,
    pub product_id: String,
    pub copy_type: String,
    pub text: String,
    pub keywords: Vec<String>,
    pub tone: String,
    pub at: DateTime<Utc>,
}

/// Draft a product's copy.
#[derive(Debug)]
pub struct GenerateCopy;

impl Operation for GenerateCopy {
    const META: OpMeta = OpMeta {
        address: "catalog.product.generate_copy",
        tool: "generate_copy",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/catalog/products/{product_id}/generate-copy",
        summary: "Draft a product's title, one-line summary, description or search text in the store's tone, from the product's own data. Saved as a draft marked as generated; put it on the product with update_product_group.",
        consumes: &[Slot::one("product_id")],
        produces: &[Slot::one("copy_draft_id")],
        checks: &["content.ai_content_disclosure", "product.approval"],
        errors: &[GENERATION_FAILED, PRODUCT_NOT_FOUND],
        audience: Audience::Operator,
        action_type: ActionType::CREATE,
        person_scope: "",
    };
    type Input = GenerateCopyInput;
    type Output = Value;
}

impl WriteOp<Cx> for GenerateCopy {
    type Plan = CopyPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: GenerateCopyInput,
    ) -> Result<Planned<CopyPlan>, ToolError> {
        crate::operators_only(cx, "draft product copy")?;
        let copy = CopyRules::from_policy(&cx.policy)?;
        let content = ContentRules::from_policy(&cx.policy)?;
        let keywords: Vec<String> = input
            .keywords
            .unwrap_or_default()
            .into_iter()
            .map(|k| k.trim().to_owned())
            .filter(|k| !k.is_empty())
            .collect();
        let tone = input
            .tone
            .map(|t| t.trim().to_owned())
            .filter(|t| !t.is_empty())
            .unwrap_or_else(|| copy.tone.clone());
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let group = repo::group(&mut tx, &input.product_id)
            .await?
            .ok_or_else(|| {
                product_not_found("product", &input.product_id, "search_product_groups")
            })?;
        let variants = repo::variants(&mut tx, std::slice::from_ref(&group.id)).await?;
        let brand = match &group.brand_id {
            Some(b) => repo::brand(&mut tx, b).await?.map(|b| b.name),
            None => None,
        };
        tx.commit().await.map_err(ToolError::from)?;
        let facts = CopyFacts::from_rows(&group, brand, &variants);
        let text = write_copy(&facts, input.copy_type, &copy, &content, &keywords);
        if text.trim().is_empty() {
            return Err(ToolError::new(
                GENERATION_FAILED,
                format!("There is too little about {} to write from.", group.id),
            )
            .detail(json!({"upstream": content.generator, "retryable": false}))
            .next("generate_copy"));
        }
        let checks = vec![
            Check {
                rule: "content.ai_content_disclosure".into(),
                passed: true,
                detail: Some(if content.disclosure_required {
                    "Marked as generated (acom:ai_generated: true)".into()
                } else {
                    "Marked as generated, though the store does not require it".into()
                }),
                fix_tool: None,
            },
            Check {
                rule: "product.approval".into(),
                passed: true,
                detail: Some("Copy is content: saved as a draft without approval".into()),
                fix_tool: None,
            },
        ];
        Ok(Planned {
            changes: vec![Change {
                op: ChangeOp::Create,
                entity: "copy_draft".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"product_id": group.id, "copy_type": input.copy_type.as_str(),
                           "text": text, "keywords": keywords, "tone": tone,
                           "acom:ai_generated": true, "generator": content.generator}),
                note: Some(
                    "Built from the product's own data in the store's tone rules; no writing model is connected. Use it with update_product_group".into(),
                ),
            }],
            checks,
            basis: vec![Basis {
                entity: "catalog.product_group".into(),
                id: group.id.clone(),
                version: i64::from(group.version),
            }],
            approval: match cx
                .policy
                .approval_for(Self::META.tool, &json!({"change_type": "content"}))
            {
                a if a.needed => ApprovalNeed::Needed {
                    reason: a.reason.unwrap_or_default(),
                },
                _ => ApprovalNeed::NotNeeded,
            },
            plan: CopyPlan {
                copy_draft_id: format!("cpd_{}", acom_core::id::new_suffix()),
                product_id: group.id,
                copy_type: input.copy_type.as_str().to_owned(),
                text,
                keywords,
                tone,
                at: cx.clock.now(),
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &CopyPlan) -> Result<Committed, ToolError> {
        sqlx::query(
            "INSERT INTO catalog.copy_draft (tenant_id, id, product_group_id, copy_type, text,
               keywords, tone, is_ai_generated, date_created)
             VALUES ($1, $2, $3, $4, $5, $6, $7, true, $8)
             ON CONFLICT (tenant_id, id) DO NOTHING",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.copy_draft_id)
        .bind(&plan.product_id)
        .bind(&plan.copy_type)
        .bind(&plan.text)
        .bind(&plan.keywords)
        .bind(&plan.tone)
        .bind(plan.at)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        let mut produced = Map::new();
        produced.insert("copy_draft_id".into(), json!(plan.copy_draft_id));
        produced.insert("text".into(), json!(plan.text));
        Ok(Committed {
            id: plan.copy_draft_id.clone(),
            produced,
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &CopyPlan,
        _done: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let text: Option<String> = sqlx::query_scalar(
            "SELECT text FROM catalog.copy_draft WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&plan.copy_draft_id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = text.as_deref() == Some(plan.text.as_str());
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} was not saved", plan.copy_draft_id)]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &CopyPlan,
        _done: &Committed,
    ) -> Result<(), ToolError> {
        sqlx::query("DELETE FROM catalog.copy_draft WHERE tenant_id = $1 AND id = $2")
            .bind(tx.tenant.as_str())
            .bind(&plan.copy_draft_id)
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
    use acom_contract::ReadOp;

    fn windbreaker() -> CopyFacts {
        CopyFacts {
            name: "Packable windbreaker".into(),
            brand: Some("Northwind".into()),
            material: Some("Recycled nylon".into()),
            colors: vec!["Slate".into(), "Black".into()],
            sizes: vec!["XS".into(), "S".into(), "M".into(), "L".into(), "XL".into()],
            wear: vec![
                "packs down to 14 × 18 cm".into(),
                "packs into its own pocket".into(),
                "blocks the wind".into(),
                "sheds light rain".into(),
            ],
        }
    }

    fn rules() -> (CopyRules, ContentRules) {
        let p = crate::testing::policies();
        (
            CopyRules::from_policy(&p).unwrap_or_else(|e| panic!("{e}")),
            ContentRules::from_policy(&p).unwrap_or_else(|e| panic!("{e}")),
        )
    }

    #[test]
    fn copy_follows_the_store_tone_rules_and_the_product_data() {
        let (copy, content) = rules();
        let f = windbreaker();
        let d = write_copy(&f, CopyType::Description, &copy, &content, &[]);
        assert_eq!(
            d,
            "The packable windbreaker from Northwind, in recycled nylon. It packs down to 14 × 18 cm, packs into its own pocket, blocks the wind and sheds light rain.\nCare: follow the label on the garment."
        );
        // The same input gives the same draft.
        assert_eq!(
            d,
            write_copy(&f, CopyType::Description, &copy, &content, &[])
        );
        let t = write_copy(&f, CopyType::Title, &copy, &content, &["travel".into()]);
        assert_eq!(t, "Packable windbreaker in recycled nylon, travel");
        let s = write_copy(&f, CopyType::Seo, &copy, &content, &["packable".into()]);
        assert!(s.chars().count() <= content.seo_max_chars, "{s}");
        assert!(
            s.starts_with("Packable windbreaker in recycled nylon. Packs down"),
            "{s}"
        );
        let one = CopyRules {
            sentences: 1,
            care_line: false,
            ..copy
        };
        assert_eq!(
            write_copy(&f, CopyType::Description, &one, &content, &[]),
            "The packable windbreaker from Northwind, in recycled nylon."
        );
    }

    #[test]
    fn long_text_is_cut_at_a_word() {
        assert_eq!(
            fit("Merino trail tee for long days", 16),
            "Merino trail tee"
        );
        assert_eq!(fit("short", 10), "short");
        assert_eq!(
            and_list(&["a".into(), "b".into(), "c".into()]),
            "a, b and c"
        );
        assert_eq!(phrase("packing", &json!("none")), None);
        assert_eq!(
            phrase("warmth", &json!("light")).as_deref(),
            Some("light warmth")
        );
    }

    #[tokio::test]
    async fn shoppers_cannot_generate_copy() {
        let e = GenerateCopy
            .preview(
                &testing::shopper(),
                GenerateCopyInput {
                    product_id: "prd_windbreaker".into(),
                    copy_type: CopyType::Title,
                    tone: None,
                    keywords: None,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "forbidden");
        let schema = acom_contract::schema::closed_schema_for::<GenerateCopyInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
    }

    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn the_windbreaker_description_is_drafted_saved_and_taken_back() {
        let cx = testing::operator_db().await;
        let planned = GenerateCopy
            .preview(
                &cx,
                GenerateCopyInput {
                    product_id: "prd_windbreaker".into(),
                    copy_type: CopyType::Description,
                    tone: None,
                    keywords: None,
                },
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(planned.approval, ApprovalNeed::NotNeeded);
        let text = planned.changes[0].to["text"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        assert!(
            text.starts_with("The packable windbreaker from Northwind, in recycled nylon."),
            "{text}"
        );
        assert!(
            text.ends_with("Care: follow the label on the garment."),
            "{text}"
        );
        let mut tx = cx.begin().await.unwrap_or_else(|e| panic!("{e}"));
        let done = GenerateCopy
            .commit(&mut tx, &planned.plan)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        GenerateCopy
            .undo(&mut tx, &planned.plan, &done)
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        tx.tx.rollback().await.unwrap_or_else(|e| panic!("{e}"));
        // The product page is untouched until update_product_group uses the draft.
        let g = crate::product::get_product::GetProduct
            .read(
                &cx,
                serde_json::from_value(json!({"product_id": "prd_windbreaker"}))
                    .unwrap_or_else(|e| panic!("{e}")),
            )
            .await
            .unwrap_or_else(|e| panic!("{e}"));
        assert_eq!(g["id"], "prd_windbreaker");
    }
}
