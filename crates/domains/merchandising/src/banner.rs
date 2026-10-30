//! `schedule_banner`, `list_banners`, `set_banner` (docs/api/merchandising.md#banner).
//!
//! A banner shown in a slot is a booking (`display_booking`, `dsp_`) of banner content
//! (`banner`, `bnr_`) for one audience over a period. Running a schedule creates the booking;
//! whether it shows is computed from its period when a page asks.

use std::collections::BTreeMap;

use acom_catalog::service::for_merchandising::{ProductCard, product_cards};
use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorClass,
    ErrorCode, Group, Kind, Method, OpMeta, Operation, Planned, ReadOp, Slot, ToolError,
    UndoSupport, Verification, WriteOp,
};
use acom_core::enums::{DisplayStatus, SalesChannel};
use acom_core::ids::{BannerId, BookingId};
use acom_store::{Cx, Page, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::display_slot::{SLOT_NOT_FOUND, StorePage};
use crate::model::{
    DisplayRules, count_text, display_status, image_format, page_words, period_words, slot_words,
};
use crate::repo::{self, BannerRow, BookingRow};
use crate::{
    INVALID_PERIOD, TIER_NOT_FOUND, audience, check_id, check_id_or_placeholder, known_tier,
    parse_time,
};

pub const SLOT_TAKEN: ErrorCode = ErrorCode::new("slot_taken", ErrorClass::PolicyViolation);
pub const VALID_FROM_IN_PAST: ErrorCode =
    ErrorCode::new("valid_from_in_past", ErrorClass::InvalidInput);
pub const PROMOTION_NOT_FOUND: ErrorCode =
    ErrorCode::new("promotion_not_found", ErrorClass::NotFound);

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

/// The approval sentence of a booking: where it shows and to whom.
pub(crate) fn banner_reason(
    page: &str,
    tier: Option<&acom_customer::service::for_merchandising::Tier>,
) -> String {
    let reach = match tier {
        Some(t) => format!("{} {} members", count_text(t.member_count), t.name),
        None => "everyone".to_owned(),
    };
    format!("Shows on {} to {reach}", page_words(page))
}

/// `slot_free`: no other booking of the slot for a blocking audience shares a moment.
pub(crate) fn check_slot_free(
    rules: &DisplayRules,
    slot_type: &str,
    audience_word: &str,
    audience_name: &str,
    period: &str,
    others: &[BookingRow],
) -> Check {
    let taken: Vec<String> = others
        .iter()
        .filter(|b| rules.blocks(audience_word, audience(b.valid_for_member_tier.as_deref())))
        .map(|b| {
            format!(
                "{} holds {}",
                b.id,
                audience(b.valid_for_member_tier.as_deref())
            )
        })
        .collect();
    if taken.is_empty() {
        Check {
            rule: "slot_free".into(),
            passed: true,
            detail: Some(format!(
                "{} open {period} for {audience_name}",
                slot_words(slot_type)
            )),
            fix_tool: None,
        }
    } else {
        Check {
            rule: "slot_free".into(),
            passed: false,
            detail: Some(taken.join("; ")),
            fix_tool: Some("list_display_slots".into()),
        }
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

/// A channel a banner shows on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum BannerChannel {
    /// The website on a computer.
    Web,
    /// The website on a phone.
    MobileWeb,
    /// The app.
    App,
}

/// The channels a banner shows on. Banners have no channel limit stored, so all of them.
fn every_channel() -> Vec<&'static str> {
    SalesChannel::ALL
        .iter()
        .filter(|c| **c != SalesChannel::Agent)
        .map(|c| c.as_str())
        .collect()
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ListBannersInput {
    /// Only banners booked on this page.
    #[schemars(example = &"main")]
    pub page: Option<StorePage>,
    /// Only banners shown on this channel: web, mobile_web or app.
    #[schemars(example = &"web")]
    pub channel: Option<BannerChannel>,
    /// Only banners showing at this moment (store time without an offset).
    #[schemars(example = &"2026-11-07T10:00")]
    pub as_of: Option<String>,
    /// Page size, 1–100. Default 20.
    #[schemars(range(min = 1, max = 100), example = 20)]
    pub limit: Option<u32>,
    /// `next_cursor` from the previous page.
    #[schemars(example = &"o20")]
    pub cursor: Option<String>,
}

pub struct ListBanners;

impl Operation for ListBanners {
    const META: OpMeta = OpMeta {
        address: "merchandising.banner.list",
        tool: "list_banners",
        group: Group::Offer,
        kind: Kind::Read,
        method: Method::Get,
        path: "/v1/merchandising/banners",
        summary: "Banner content (image, link, words) with the bookings that show it, optionally only what shows on a page or at a moment.",
        consumes: &[],
        produces: &[Slot::many("banner_id")],
        checks: &[],
        errors: &[],
        audience: Audience::Both,
        action_type: ActionType::SEARCH,
        person_scope: "",
    };
    type Input = ListBannersInput;
    type Output = Value;
}

impl ReadOp<Cx> for ListBanners {
    async fn read(&self, cx: &Cx, input: ListBannersInput) -> Result<Value, ToolError> {
        let page = Page::new(input.limit, input.cursor.as_deref())?;
        let at = input
            .as_of
            .as_deref()
            .map(|s| parse_time(&cx.time, "as_of", s))
            .transpose()?;
        // Every banner shows on every channel (no per-banner channel is stored), so `channel`
        // narrows nothing.
        let _channel = input.channel;
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let (total, rows) =
            repo::banners(&mut tx, input.page.map(StorePage::word), at, page).await?;
        let ids: Vec<String> = rows.iter().map(|b| b.id.clone()).collect();
        let bookings = repo::bookings_of_banners(&mut tx, &ids).await?;
        let products: Vec<String> = rows
            .iter()
            .filter_map(|b| b.image_product_group_id.clone())
            .collect();
        let cards: BTreeMap<String, ProductCard> = product_cards(&mut tx, &products)
            .await?
            .into_iter()
            .map(|c| (c.product_id.clone(), c))
            .collect();
        tx.commit().await.map_err(ToolError::from)?;
        let items: Vec<Value> = rows
            .iter()
            .map(|b| {
                let content_url = b.image.clone().or_else(|| {
                    b.image_product_group_id
                        .as_ref()
                        .and_then(|p| cards.get(p))
                        .and_then(|c| c.image.clone())
                });
                let shown: Vec<Value> = bookings
                    .iter()
                    .filter(|d| d.banner_id.as_deref() == Some(b.id.as_str()))
                    .map(|d| {
                        json!({"display_booking_id": d.id, "slot_id": d.display_slot_id,
                               "audience": audience(d.valid_for_member_tier.as_deref()),
                               "valid_from": cx.time.format(d.valid_from),
                               "valid_through": d.valid_through.map(|t| cx.time.format(t)),
                               "status": display_status(&d.display_status, d.valid_from,
                                                        d.valid_through, at.unwrap_or(now))})
                    })
                    .collect();
                json!({
                    "id": b.id, "name": b.name, "content_url": content_url, "url": b.url,
                    "channels": every_channel(), "priority": Value::Null,
                    "eyebrow": b.eyebrow, "headline": b.headline,
                    "alternative_headline": b.alternative_headline, "cta_label": b.cta_label,
                    "image_product_id": b.image_product_group_id, "image_color": b.image_color,
                    "bookings": shown,
                })
            })
            .collect();
        let mut out = json!({"items": items, "total": total});
        if let Some(c) = page.next_cursor(rows.len(), total) {
            out["next_cursor"] = json!(c);
        }
        Ok(out)
    }
}

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
