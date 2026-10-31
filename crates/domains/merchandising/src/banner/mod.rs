//! The `banner` resource (docs/api/merchandising.md#banner): what its operations share.
//!
//! A banner shown in a slot is a booking (`display_booking`, `dsp_`) of banner content
//! (`banner`, `bnr_`) for one audience over a period. Running a schedule creates the booking;
//! whether it shows is computed from its period when a page asks.

pub mod list_banners;
pub mod schedule_banner;
pub mod set_banner;

use acom_contract::{Check, ErrorClass, ErrorCode};
use acom_core::enums::SalesChannel;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::audience;
use crate::model::{DisplayRules, count_text, page_words, slot_words};
use crate::repo::BookingRow;

pub const SLOT_TAKEN: ErrorCode = ErrorCode::new("slot_taken", ErrorClass::PolicyViolation);
pub const VALID_FROM_IN_PAST: ErrorCode =
    ErrorCode::new("valid_from_in_past", ErrorClass::InvalidInput);
pub const PROMOTION_NOT_FOUND: ErrorCode =
    ErrorCode::new("promotion_not_found", ErrorClass::NotFound);

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

impl BannerChannel {
    /// The `sales_channel` word.
    pub fn word(self) -> &'static str {
        match self {
            BannerChannel::Web => "web",
            BannerChannel::MobileWeb => "mobile_web",
            BannerChannel::App => "app",
        }
    }
}

/// The channel words of an input, each once in the order given; `None` (every channel) when the
/// input names none.
pub(crate) fn channel_input(channels: Option<&[BannerChannel]>) -> Option<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for c in channels.unwrap_or_default() {
        if !out.iter().any(|w| w == c.word()) {
            out.push(c.word().to_owned());
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The channels a banner shows on. Banners have no channel limit stored, so all of them.
fn every_channel() -> Vec<&'static str> {
    SalesChannel::ALL
        .iter()
        .filter(|c| **c != SalesChannel::Agent)
        .map(|c| c.as_str())
        .collect()
}
