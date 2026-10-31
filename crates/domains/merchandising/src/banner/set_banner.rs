//! `set_banner` (docs/api/merchandising.md#banner).

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::DisplayStatus;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::*;
use crate::check_id;
use crate::model::{DisplayRules, display_status, image_format};
use crate::repo::{self, BannerRow};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetBannerInput {
    /// The banner image to register or change; an id that does not exist yet registers a new one.
    #[schemars(regex(pattern = r"^bnr_[a-z0-9_]+$"), example = &"bnr_vip_weekend")]
    pub banner_id: String,
    /// URL of the image file (jpeg, png or webp; policy `banner.image_spec`).
    #[schemars(regex(pattern = r"^https?://"), example = &"https://cdn.northwind.example/b/vip-weekend.webp")]
    pub content_url: String,
    /// Store path the banner opens, starting with `/`. Default for a new banner: `/`.
    #[schemars(regex(pattern = r"^/"), example = &"/sale/vip-weekend")]
    pub url: Option<String>,
    /// Channels to show on: web, mobile_web, app.
    #[schemars(length(min = 1, max = 3), example = ["web", "app"])]
    pub channels: Vec<BannerChannel>,
    /// Order among the page's banners, 1 first.
    #[schemars(range(min = 1, max = 100), example = 1)]
    pub priority: Option<i32>,
}

/// What a run of `set_banner` writes, and what was there before (for undo).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SetBannerRun {
    pub banner_id: String,
    pub content_url: String,
    pub url: String,
    /// The banner as it was, `None` when the run registers a new one.
    pub before: Option<BannerBefore>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BannerBefore {
    pub image: Option<String>,
    pub image_product_id: Option<String>,
    pub image_color: Option<String>,
    pub url: String,
}

pub struct SetBanner;

impl Operation for SetBanner {
    const META: OpMeta = OpMeta {
        address: "merchandising.banner.set",
        tool: "set_banner",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Put,
        path: "/v1/merchandising/banners/{banner_id}",
        summary: "Register or change a banner image (image file and link). Showing it happens through schedule_banner; changing a banner shoppers see now needs a person's approval.",
        consumes: &[],
        produces: &[Slot::one("banner_id")],
        checks: &["banner.image_spec", "display.priority_unique"],
        errors: &[],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SetBannerInput;
    type Output = Value;
}

impl WriteOp<Cx> for SetBanner {
    type Plan = SetBannerRun;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SetBannerInput,
    ) -> Result<Planned<SetBannerRun>, ToolError> {
        let rules = DisplayRules::from_policy(&cx.policy)?;
        check_id("banner_id", "bnr", &input.banner_id)?;
        if !(input.content_url.starts_with("https://") || input.content_url.starts_with("http://"))
        {
            return Err(
                ToolError::invalid_input("`content_url` is the image's web address.")
                    .detail(json!({"field": "content_url", "reason": "format"})),
            );
        }
        if let Some(u) = &input.url
            && !u.starts_with('/')
        {
            return Err(
                ToolError::invalid_input("`url` is a store path starting with `/`.")
                    .detail(json!({"field": "url", "reason": "format"})),
            );
        }
        if input.channels.is_empty() {
            return Err(
                ToolError::invalid_input("`channels` needs at least one channel.")
                    .detail(json!({"field": "channels", "reason": "missing"})),
            );
        }
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let existing = repo::banner(&mut tx, &input.banner_id).await?;
        let live = match &existing {
            Some(_) => repo::bookings_of_banners(&mut tx, std::slice::from_ref(&input.banner_id))
                .await?
                .iter()
                .any(|b| {
                    display_status(&b.display_status, b.valid_from, b.valid_through, now)
                        == DisplayStatus::Active
                }),
            None => false,
        };
        tx.commit().await.map_err(ToolError::from)?;

        let url = input
            .url
            .clone()
            .or_else(|| existing.as_ref().map(|b| b.url.clone()))
            .unwrap_or_else(|| "/".to_owned());
        let format = image_format(&input.content_url);
        let image_check = match format {
            Some(f) if rules.image_formats.iter().any(|x| x == f) => Check {
                rule: "banner.image_spec".into(),
                passed: true,
                detail: Some(format!(
                    "{f} is an allowed format; size is checked when the file is uploaded"
                )),
                fix_tool: None,
            },
            other => Check {
                rule: "banner.image_spec".into(),
                passed: false,
                detail: Some(format!(
                    "{} is not an allowed format; use {}",
                    other.unwrap_or("This file"),
                    rules.image_formats.join(", ")
                )),
                fix_tool: Some("set_banner".into()),
            },
        };
        let priority_check = Check {
            rule: "display.priority_unique".into(),
            passed: true,
            detail: Some(match input.priority {
                Some(_) => "Banner priorities are not stored yet, so none can clash".into(),
                None => "No priority set".into(),
            }),
            fix_tool: None,
        };
        let channels: Vec<&str> = input
            .channels
            .iter()
            .map(|c| match c {
                BannerChannel::Web => "web",
                BannerChannel::MobileWeb => "mobile_web",
                BannerChannel::App => "app",
            })
            .collect();
        let change = Change {
            op: if existing.is_some() { ChangeOp::Update } else { ChangeOp::Create },
            entity: "banner_image".into(),
            id: input.banner_id.clone(),
            from: existing.as_ref().map_or(Value::Null, |b| {
                json!({"content_url": b.image, "image_product_id": b.image_product_group_id, "url": b.url})
            }),
            to: json!({"content_url": input.content_url, "url": url, "channels": channels,
                       "priority": input.priority}),
            note: Some("Every banner shows on every channel; channel and priority are not stored yet".into()),
        };
        let facts = json!({"reaches_customers": live && rules.reaches_customers.iter().any(|t| t == Self::META.tool)});
        let approval = cx.policy.approval_for(Self::META.tool, &facts);
        let basis = existing
            .as_ref()
            .map(|b| Basis {
                entity: "merchandising.banner".into(),
                id: b.id.clone(),
                version: i64::from(b.version),
            })
            .into_iter()
            .collect();
        Ok(Planned {
            plan: SetBannerRun {
                banner_id: input.banner_id,
                content_url: input.content_url,
                url,
                before: existing.map(|b| BannerBefore {
                    image: b.image,
                    image_product_id: b.image_product_group_id,
                    image_color: b.image_color,
                    url: b.url,
                }),
            },
            changes: vec![change],
            checks: vec![image_check, priority_check],
            basis,
            approval: if approval.needed {
                ApprovalNeed::Needed {
                    reason: "Changes a banner shoppers see now".into(),
                }
            } else {
                ApprovalNeed::NotNeeded
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &SetBannerRun) -> Result<Committed, ToolError> {
        match &run.before {
            Some(_) => {
                repo::update_banner_image(
                    tx,
                    &run.banner_id,
                    Some(&run.content_url),
                    None,
                    None,
                    &run.url,
                )
                .await?;
            }
            None => {
                let name = run
                    .content_url
                    .rsplit('/')
                    .next()
                    .and_then(|f| f.split('.').next())
                    .filter(|n| !n.is_empty())
                    .unwrap_or(&run.banner_id)
                    .to_owned();
                repo::insert_banner(
                    tx,
                    &BannerRow {
                        id: run.banner_id.clone(),
                        name,
                        eyebrow: None,
                        headline: String::new(),
                        alternative_headline: None,
                        cta_label: None,
                        url: run.url.clone(),
                        image: Some(run.content_url.clone()),
                        image_product_group_id: None,
                        image_color: None,
                        version: 1,
                    },
                )
                .await?;
            }
        }
        Ok(Committed {
            id: run.banner_id.clone(),
            produced: serde_json::Map::from_iter([("banner_id".to_owned(), json!(run.banner_id))]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &SetBannerRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let b = repo::banner(&mut tx, &run.banner_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        match b {
            None => mismatches.push(format!("banner {} is missing", run.banner_id)),
            Some(b) => {
                if b.image.as_deref() != Some(run.content_url.as_str()) || b.url != run.url {
                    mismatches.push(format!(
                        "banner {} has another image or link",
                        run.banner_id
                    ));
                }
            }
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        run: &SetBannerRun,
        _: &Committed,
    ) -> Result<(), ToolError> {
        match &run.before {
            Some(b) => {
                repo::update_banner_image(
                    tx,
                    &run.banner_id,
                    b.image.as_deref(),
                    b.image_product_id.as_deref(),
                    b.image_color.as_deref(),
                    &b.url,
                )
                .await?;
            }
            None => {
                repo::delete_banner_if_unbooked(tx, &run.banner_id).await?;
            }
        }
        Ok(())
    }
}
