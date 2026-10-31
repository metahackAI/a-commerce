//! `schedule_banner` (docs/api/merchandising.md#banner).

use acom_catalog::service::for_merchandising::{ProductCard, product_cards};
use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::DisplayStatus;
use acom_core::ids::{BannerId, BookingId};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::*;
use crate::display_slot::SLOT_NOT_FOUND;
use crate::model::{DisplayRules, period_words};
use crate::repo::{self, BannerRow, BookingRow};
use crate::{
    INVALID_PERIOD, TIER_NOT_FOUND, audience, check_id, check_id_or_placeholder, known_tier,
    parse_time,
};

/// Where a banner's picture comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ImageSource {
    /// A product's main image: needs `image_product_id`, `image_color` is optional.
    Product,
    /// A registered image: needs `image_id`.
    Image,
}

/// The banner's words, link and picture.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BannerContent {
    /// Small header above the headline (policy `banner.text_max_length.eyebrow`).
    #[schemars(example = &"VIP WEEKEND")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub eyebrow: Option<String>,
    /// The headline (policy `banner.text_max_length.headline`).
    #[schemars(example = &"20% off summer outerwear")]
    pub headline: String,
    /// Button text (policy `banner.text_max_length.cta_label`).
    #[schemars(example = &"Shop the sale")]
    pub cta_label: String,
    /// Store path the button opens, starting with `/`.
    #[schemars(regex(pattern = r"^/"), example = &"/sale/vip-weekend")]
    pub url: String,
    /// Where the picture comes from: `product` or `image`.
    #[schemars(example = &"product")]
    pub image_source: ImageSource,
    /// The product whose image to show, when `image_source` is `product`.
    #[schemars(regex(pattern = r"^prd_[a-z0-9_]+$"), example = &"prd_field_jacket")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_product_id: Option<String>,
    /// That product's color to show, when `image_source` is `product`. Default: its main color.
    #[schemars(length(min = 1, max = 40), example = &"Sand")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_color: Option<String>,
    /// A registered banner image (`set_banner`, `bnr_`) or generated image (`img_`), when
    /// `image_source` is `image`.
    #[schemars(regex(pattern = r"^(bnr|img)_[a-z0-9_]+$"), example = &"bnr_fall_layers")]
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScheduleBannerInput {
    /// The slot (`list_display_slots`).
    #[schemars(regex(pattern = r"^slot_[a-z0-9_]+$"), example = &"slot_home_hero")]
    pub slot_id: String,
    /// Who sees the banner: a tier id (`list_tiers`) or `everyone`. Other audiences keep
    /// seeing what they see now.
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"vip")]
    pub audience: String,
    /// Start (inclusive), not in the past. Without an offset it is read in store time.
    #[schemars(example = &"2026-11-07T00:00")]
    pub valid_from: String,
    /// End (inclusive, minute precision), after `valid_from`.
    #[schemars(example = &"2026-11-08T23:59")]
    pub valid_through: String,
    /// The banner's words, link and picture.
    pub content: BannerContent,
    /// The promotion the banner announces; the banner's period must lie inside its period.
    #[schemars(regex(pattern = r"^promo_[a-z0-9_]+$"), example = &"promo_autumn_knit")]
    pub promotion_id: Option<String>,
}

/// Banner content a run creates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlannedBanner {
    pub id: String,
    pub eyebrow: Option<String>,
    pub headline: String,
    pub cta_label: String,
    pub url: String,
    /// URL of a registered image, or `None` when a product image is used.
    pub image: Option<String>,
    pub image_product_id: Option<String>,
    pub image_color: Option<String>,
}

/// What a run of `schedule_banner` writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BannerRun {
    pub booking_id: String,
    pub slot_id: String,
    /// `None` for everyone.
    pub valid_for_member_tier: Option<String>,
    pub valid_from: DateTime<Utc>,
    pub valid_through: DateTime<Utc>,
    /// The promotion id, or a `$item[n].promotion_id` placeholder until the runner binds it.
    pub promotion_id: Option<String>,
    pub banner: PlannedBanner,
}

pub struct ScheduleBanner;

impl Operation for ScheduleBanner {
    const META: OpMeta = OpMeta {
        address: "merchandising.banner.schedule",
        tool: "schedule_banner",
        group: Group::Offer,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/merchandising/banners",
        summary: "Book a banner (small header, headline, button, link, image) into a slot for one audience and period. Other audiences keep their current banner. Reaches shoppers, so it always needs a person's approval.",
        consumes: &[
            Slot::one("slot_id"),
            Slot::one("tier_id"),
            Slot::one("promotion_id"),
        ],
        produces: &[Slot::one("display_booking_id")],
        checks: &["slot_free"],
        errors: &[
            SLOT_TAKEN,
            SLOT_NOT_FOUND,
            VALID_FROM_IN_PAST,
            INVALID_PERIOD,
            PROMOTION_NOT_FOUND,
            TIER_NOT_FOUND,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::SCHEDULE,
        person_scope: "",
    };
    type Input = ScheduleBannerInput;
    type Output = Value;
}

fn content_error(field: &str, reason: &str, message: String) -> ToolError {
    ToolError::invalid_input(message)
        .detail(json!({"field": format!("content.{field}"), "reason": reason}))
}

/// Text limits and the fields each image source needs.
pub(crate) fn check_content(rules: &DisplayRules, c: &BannerContent) -> Result<(), ToolError> {
    let limits = [
        (
            "eyebrow",
            c.eyebrow.as_deref().unwrap_or(""),
            rules.eyebrow_max,
        ),
        ("headline", c.headline.as_str(), rules.headline_max),
        ("cta_label", c.cta_label.as_str(), rules.cta_label_max),
    ];
    for (field, text, max) in limits {
        if text.chars().count() > max {
            return Err(content_error(
                field,
                "range",
                format!("`content.{field}` is at most {max} characters."),
            ));
        }
    }
    for (field, text) in [("headline", &c.headline), ("cta_label", &c.cta_label)] {
        if text.trim().is_empty() {
            return Err(content_error(
                field,
                "missing",
                format!("`content.{field}` must not be empty."),
            ));
        }
    }
    if !c.url.starts_with('/') {
        return Err(content_error(
            "url",
            "format",
            "`content.url` is a store path starting with `/`.".into(),
        ));
    }
    match c.image_source {
        ImageSource::Product if c.image_product_id.is_none() => Err(content_error(
            "image_product_id",
            "missing",
            "`image_source: product` needs `content.image_product_id`.".into(),
        )),
        ImageSource::Image if c.image_id.is_none() => Err(content_error(
            "image_id",
            "missing",
            "`image_source: image` needs `content.image_id`.".into(),
        )),
        _ => Ok(()),
    }
}

impl WriteOp<Cx> for ScheduleBanner {
    type Plan = BannerRun;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ScheduleBannerInput,
    ) -> Result<Planned<BannerRun>, ToolError> {
        let rules = DisplayRules::from_policy(&cx.policy)?;
        check_id("slot_id", "slot", &input.slot_id)?;
        let promotion_placeholder = match &input.promotion_id {
            Some(p) => check_id_or_placeholder("promotion_id", "promo", p)?,
            None => false,
        };
        check_content(&rules, &input.content)?;
        let from = parse_time(&cx.time, "valid_from", &input.valid_from)?;
        let through = parse_time(&cx.time, "valid_through", &input.valid_through)?;
        if through <= from {
            return Err(ToolError::new(
                INVALID_PERIOD,
                "`valid_through` must come after `valid_from`.",
            )
            .detail(json!({"field": "valid_through", "reason": "range"}))
            .next("get_promotion"));
        }
        let now = cx.clock.now();
        if rules.start_not_in_past && from < now {
            return Err(ToolError::new(
                VALID_FROM_IN_PAST,
                format!(
                    "The banner would start in the past ({}).",
                    cx.time.format(from)
                ),
            )
            .detail(json!({"field": "valid_from", "reason": "past"}))
            .next("schedule_banner"));
        }

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let slot = repo::slot(&mut tx, &input.slot_id).await?.ok_or_else(|| {
            ToolError::new(
                SLOT_NOT_FOUND,
                format!("There is no slot {}.", input.slot_id),
            )
            .detail(json!({"entity": "display_slot", "id": input.slot_id}))
            .next("list_display_slots")
        })?;
        let tier = known_tier(&mut tx, &input.audience).await?;
        let tier_id = tier.as_ref().map(|t| t.id.clone());
        let c = &input.content;
        let (image, image_product_id, image_color) = match c.image_source {
            ImageSource::Product => {
                let id = c.image_product_id.clone().unwrap_or_default();
                check_id("content.image_product_id", "prd", &id)?;
                let card: ProductCard = product_cards(&mut tx, std::slice::from_ref(&id))
                    .await?
                    .into_iter()
                    .next()
                    .ok_or_else(|| {
                        ToolError::new(ErrorCode::NOT_FOUND, format!("There is no product {id}."))
                            .detail(json!({"entity": "product", "id": id}))
                            .next("search_products")
                    })?;
                if let Some(color) = &c.image_color
                    && !card.colors.iter().any(|k| k == color)
                {
                    return Err(content_error(
                        "image_color",
                        "enum",
                        format!("{} does not come in {color}.", card.name),
                    )
                    .detail(json!({"field": "content.image_color", "reason": "enum",
                                   "allowed": card.colors})));
                }
                (None, Some(id), c.image_color.clone())
            }
            ImageSource::Image => {
                let id = c.image_id.clone().unwrap_or_default();
                let registered = match id.strip_prefix("bnr_") {
                    Some(_) => repo::banner(&mut tx, &id).await?,
                    None => None,
                };
                // Generated images (`img_`) have no store yet, so only registered banners resolve.
                let b = registered.ok_or_else(|| {
                    ToolError::new(ErrorCode::NOT_FOUND, format!("There is no image {id}."))
                        .detail(json!({"entity": "image", "id": id}))
                        .next("search_products")
                })?;
                (b.image, b.image_product_group_id, b.image_color)
            }
        };
        if let (Some(pid), false) = (&input.promotion_id, promotion_placeholder) {
            let p = acom_promotion::service::for_merchandising::promotion(&mut tx, pid)
                .await?
                .ok_or_else(|| {
                    ToolError::new(PROMOTION_NOT_FOUND, format!("There is no promotion {pid}."))
                        .detail(json!({"entity": "promotion", "id": pid}))
                        .next("list_active_promotions")
                })?;
            let inside = p.valid_from <= from && p.valid_through.is_none_or(|t| through <= t);
            if !inside {
                return Err(ToolError::new(
                    INVALID_PERIOD,
                    format!(
                        "The banner runs {} but {} runs {}; it must lie inside the promotion.",
                        period_words(from, Some(through), cx.time.zone),
                        p.name,
                        period_words(p.valid_from, p.valid_through, cx.time.zone)
                    ),
                )
                .detail(json!({"field": "valid_from", "reason": "range", "promotion_id": pid}))
                .next("get_promotion"));
            }
        }
        let others =
            repo::bookings_between(&mut tx, std::slice::from_ref(&slot.id), from, through).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let audience_name = tier
            .as_ref()
            .map_or("everyone".to_owned(), |t| t.name.clone());
        let period = period_words(from, Some(through), cx.time.zone);
        let check = check_slot_free(
            &rules,
            &slot.slot_type,
            &input.audience,
            &audience_name,
            &period,
            &others,
        );
        let change = Change {
            op: ChangeOp::Schedule,
            entity: "display_booking".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({
                "slot_id": slot.id,
                "audience": input.audience,
                "valid_from": cx.time.format(from),
                "valid_through": cx.time.format(through),
                "content": c,
                "promotion_id": input.promotion_id,
            }),
            note: None,
        };
        let basis = others
            .iter()
            .map(|b| Basis {
                entity: "merchandising.display_booking".into(),
                id: b.id.clone(),
                version: i64::from(b.version),
            })
            .chain([Basis {
                entity: "merchandising.display_slot".into(),
                id: slot.id.clone(),
                version: i64::from(slot.version),
            }])
            .collect();
        Ok(Planned {
            plan: BannerRun {
                booking_id: BookingId::generate().to_string(),
                slot_id: slot.id.clone(),
                valid_for_member_tier: tier_id,
                valid_from: from,
                valid_through: through,
                promotion_id: input.promotion_id.clone(),
                banner: PlannedBanner {
                    id: BannerId::generate().to_string(),
                    eyebrow: c.eyebrow.clone(),
                    headline: c.headline.clone(),
                    cta_label: c.cta_label.clone(),
                    url: c.url.clone(),
                    image,
                    image_product_id,
                    image_color,
                },
            },
            changes: vec![change],
            checks: vec![check],
            basis,
            approval: ApprovalNeed::Needed {
                reason: banner_reason(&slot.page, tier.as_ref()),
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &BannerRun) -> Result<Committed, ToolError> {
        let rules = DisplayRules::from_policy(&tx.policy)?;
        let audience_word = audience(run.valid_for_member_tier.as_deref()).to_owned();
        // Someone may have booked the slot since the preview.
        let others: Vec<BookingRow> = repo::bookings_between(
            tx,
            std::slice::from_ref(&run.slot_id),
            run.valid_from,
            run.valid_through,
        )
        .await?
        .into_iter()
        .filter(|b| b.id != run.booking_id)
        .filter(|b| rules.blocks(&audience_word, audience(b.valid_for_member_tier.as_deref())))
        .collect();
        if !others.is_empty() {
            let ids: Vec<&String> = others.iter().map(|b| &b.id).collect();
            return Err(ToolError::new(
                SLOT_TAKEN,
                format!("{} now holds the slot for {audience_word}.", ids[0]),
            )
            .detail(json!({"rule": "slot_free", "items": ids}))
            .next("list_display_slots"));
        }
        let b = &run.banner;
        repo::insert_banner(
            tx,
            &BannerRow {
                id: b.id.clone(),
                name: b.headline.clone(),
                eyebrow: b.eyebrow.clone(),
                headline: b.headline.clone(),
                alternative_headline: None,
                cta_label: Some(b.cta_label.clone()),
                url: b.url.clone(),
                image: b.image.clone(),
                image_product_group_id: b.image_product_id.clone(),
                image_color: b.image_color.clone(),
                version: 1,
            },
        )
        .await?;
        let status = if tx.clock.now() < run.valid_from {
            DisplayStatus::Scheduled
        } else {
            DisplayStatus::Active
        };
        repo::insert_booking(
            tx,
            &repo::NewBooking {
                id: &run.booking_id,
                slot_id: &run.slot_id,
                banner_id: &b.id,
                tier: run.valid_for_member_tier.as_deref(),
                valid_from: run.valid_from,
                valid_through: run.valid_through,
                status: status.as_str(),
                promotion_id: run.promotion_id.as_deref(),
            },
        )
        .await?;
        Ok(Committed {
            id: run.booking_id.clone(),
            produced: serde_json::Map::from_iter([(
                "display_booking_id".to_owned(),
                json!(run.booking_id),
            )]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &BannerRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let stored = repo::booking(&mut tx, &run.booking_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(compare_booking(run, stored.as_ref()))
    }

    async fn undo(&self, tx: &mut TxCx, run: &BannerRun, _: &Committed) -> Result<(), ToolError> {
        let Some(b) = repo::booking(tx, &run.booking_id).await? else {
            return Ok(());
        };
        let now = tx.clock.now();
        if now < b.valid_from {
            // Never shown: cancel it outright.
            repo::delete_booking(tx, &b.id).await?;
            repo::delete_banner_if_unbooked(tx, &run.banner.id).await?;
        } else {
            repo::end_booking(tx, &b.id, now).await?;
        }
        Ok(())
    }
}

/// Read-back check of a scheduled banner.
pub(crate) fn compare_booking(run: &BannerRun, stored: Option<&BookingRow>) -> Verification {
    let mut mismatches = Vec::new();
    match stored {
        None => mismatches.push(format!("booking {} is missing", run.booking_id)),
        Some(b) => {
            if b.display_slot_id != run.slot_id
                || b.valid_for_member_tier != run.valid_for_member_tier
                || b.valid_from != run.valid_from
                || b.valid_through != Some(run.valid_through)
                || b.promotion_id != run.promotion_id
            {
                mismatches.push(format!(
                    "booking {} differs from the plan (slot, audience, period or promotion)",
                    run.booking_id
                ));
            }
            if b.banner_id.as_deref() != Some(run.banner.id.as_str())
                || b.headline.as_deref() != Some(run.banner.headline.as_str())
                || b.url.as_deref() != Some(run.banner.url.as_str())
            {
                mismatches.push(format!("banner {} differs from the plan", run.banner.id));
            }
        }
    }
    Verification {
        matched: mismatches.is_empty(),
        mismatches,
    }
}
