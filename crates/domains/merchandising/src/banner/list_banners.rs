//! `list_banners` (docs/api/merchandising.md#banner).

use std::collections::BTreeMap;

use acom_catalog::service::for_merchandising::{ProductCard, product_cards};
use acom_contract::{
    ActionType, Audience, Group, Kind, Method, OpMeta, Operation, ReadOp, Slot, ToolError,
};
use acom_store::{Cx, Page};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

use super::*;
use crate::display_slot::StorePage;
use crate::model::display_status;
use crate::repo;
use crate::{audience, parse_time};

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
