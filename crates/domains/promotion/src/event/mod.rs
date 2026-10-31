//! The `event` resource (docs/api/promotion.md#event): an event is an entry mechanic and a
//! selection method, with rewards by rank. The mechanics, methods and reward types are store
//! registries in `promotion.yml event.*`, never fixed lists in code.
//!
//! What the operations share: the event's shape (reward tiers, eligibility, entry limit,
//! schedule), its status from the entry period and the clock, the checks `create_event` and
//! `update_event` both run, the seeded shuffle that makes draws repeatable from the published
//! seed, and the SQL of events, entries, draw records and rewards.

pub(crate) mod create_event;
pub(crate) mod dispatch_reward;
pub(crate) mod enter_event;
pub(crate) mod get_draw_record;
pub(crate) mod get_event;
pub(crate) mod list_event_types;
pub(crate) mod list_my_entries;
pub(crate) mod list_reward_types;
pub(crate) mod run_draw;
pub(crate) mod search_events;
pub(crate) mod update_event;

use std::collections::{BTreeMap, BTreeSet};

use acom_contract::{Check, ErrorClass, ErrorCode, ToolError};
use acom_core::enums::{EventStatus, MemberType, SalesChannel};
use acom_core::{Money, StoreTime};
use acom_store::TxCx;
use chrono::{DateTime, Datelike, Duration, Utc};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::model::money_text;
use crate::repo::db;
use crate::rules::{self, Combinations, RegistryItem};
use crate::{TIER_NOT_FOUND, audience, check_id, not_found, parse_time};

pub const TYPE_DISABLED: ErrorCode = ErrorCode::new("type_disabled", ErrorClass::Domain);
pub const COMBINATION_NOT_ALLOWED: ErrorCode =
    ErrorCode::new("combination_not_allowed", ErrorClass::Domain);
pub const REWARD_TYPE_DISABLED: ErrorCode =
    ErrorCode::new("reward_type_disabled", ErrorClass::Domain);
pub const ODDS_EXCEED_ONE: ErrorCode =
    ErrorCode::new("odds_exceed_one", ErrorClass::PolicyViolation);
pub const INVALID_SCHEDULE: ErrorCode =
    ErrorCode::new("invalid_schedule", ErrorClass::InvalidInput);
pub const CHANGES_AFTER_ENTRY: ErrorCode =
    ErrorCode::new("changes_after_entry", ErrorClass::Domain);
pub const ENTRY_LIMIT_EXCEEDED: ErrorCode =
    ErrorCode::new("entry_limit_exceeded", ErrorClass::LimitExceeded);
pub const EVENT_NOT_ACTIVE: ErrorCode =
    ErrorCode::new("event_not_active", ErrorClass::InvalidState);
pub const INVALID_PAYLOAD: ErrorCode = ErrorCode::new("invalid_payload", ErrorClass::InvalidInput);
pub const MANDATORY_CONSENT_MISSING: ErrorCode =
    ErrorCode::new("mandatory_consent_missing", ErrorClass::Domain);
pub const SOLD_OUT: ErrorCode = ErrorCode::new("sold_out", ErrorClass::Domain);
pub const ENTRIES_OPEN: ErrorCode = ErrorCode::new("entries_open", ErrorClass::InvalidState);
pub const NOT_DRAW_EVENT: ErrorCode = ErrorCode::new("not_draw_event", ErrorClass::Domain);
pub const REASON_REQUIRED: ErrorCode = ErrorCode::new("reason_required", ErrorClass::InvalidInput);
pub const WAITLIST_EMPTY: ErrorCode = ErrorCode::new("waitlist_empty", ErrorClass::Domain);
pub const COMPLIANCE_PENDING: ErrorCode = ErrorCode::new("compliance_pending", ErrorClass::Domain);
pub const MISSING_ADDRESS: ErrorCode = ErrorCode::new("missing_address", ErrorClass::Domain);

/// One reward rank of an event.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RewardTier {
    /// 1 is the top prize.
    #[schemars(range(min = 1, max = 100), example = 1)]
    pub rank: u32,
    /// A reward type id of the store registry (`list_reward_types`).
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"product")]
    pub reward_type: String,
    /// What is given: a product (`prd_…`) for `product`, a coupon (`cpn_…`) for `coupon`, or
    /// free text for other types.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(length(min = 1, max = 200), example = &"prd_wool_overcoat")]
    pub reward_ref: Option<String>,
    /// What the reward is worth; needed for store points and store credit, and when a store
    /// compliance rule looks at the value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(example = json!({"value": 289.00, "currency": "USD"}))]
    pub reward_value: Option<Money>,
    /// How many win this rank.
    #[schemars(range(min = 1, max = 100000), example = 1)]
    pub winner_count: u32,
    /// How many wait to step in if a winner drops out. Default: `event.draw.waitlist_count`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(max = 1000), example = 2)]
    pub waitlist_count: Option<u32>,
    /// `instant_win` only: the chance an entry wins this rank, 0–1.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "rust_decimal::serde::arbitrary_precision_option"
    )]
    #[schemars(with = "Option<f64>", range(min = 0, max = 1), example = 0.01)]
    pub odds: Option<Decimal>,
    /// `check_in` only: the check-in round this rank is for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 365), example = 7)]
    pub round: Option<u32>,
}

/// Which orders count for a purchase condition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PurchaseWindow {
    /// Orders placed between the event's start and end.
    EventPeriod,
}

/// The products a purchase condition counts.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PurchaseScope {
    /// These products count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(inner(regex(pattern = r"^prd_[a-z0-9_]+$")), example = ["prd_wool_overcoat"])]
    pub product_ids: Option<Vec<String>>,
    /// Products of these categories count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(inner(regex(pattern = r"^cat_[a-z0-9_]+$")), example = ["cat_outerwear"])]
    pub category_ids: Option<Vec<String>>,
    /// Not supported yet; leave it out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(inner(regex(pattern = r"^brn_[a-z0-9_]+$")), example = ["brn_ridgeline"])]
    pub brand_ids: Option<Vec<String>>,
    /// These products do not count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(inner(regex(pattern = r"^prd_[a-z0-9_]+$")), example = ["prd_rain_parka"])]
    pub exclude_product_ids: Option<Vec<String>>,
}

/// An order an entry must be based on.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PurchaseCondition {
    /// The order's subtotal before member benefits must reach this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(example = json!({"value": 50.00, "currency": "USD"}))]
    pub min_subtotal: Option<Money>,
    /// Which orders count; `event_period` (default): placed during the event.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(example = &"event_period")]
    pub window: Option<PurchaseWindow>,
    /// Which products count; any product when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(example = json!({"category_ids": ["cat_outerwear"]}))]
    pub scope: Option<PurchaseScope>,
}

/// Who may enter. Left-out fields take `event.eligibility` defaults (everyone).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Eligibility {
    /// A tier id (`list_tiers`) or `everyone`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^[a-z][a-z0-9_]*$"), example = &"everyone")]
    pub valid_for_member_tier: Option<String>,
    /// Only members of this segment (`list_segments`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(regex(pattern = r"^seg_[a-z0-9_]+$"), example = &"seg_lapsed_knit_buyers")]
    pub segment_id: Option<String>,
    /// Only these member types: individual, business. Any when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(example = ["individual"])]
    pub member_types: Option<Vec<MemberType>>,
    /// An order the entry must be based on (`purchase_triggered`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(example = json!({"min_subtotal": {"value": 50.00, "currency": "USD"}, "window": "event_period"}))]
    pub purchase_condition: Option<PurchaseCondition>,
    /// Only entries from these channels: web, mobile_web, app, agent. Any when left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(example = ["web", "app"])]
    pub channels: Option<Vec<SalesChannel>>,
}

/// The period an entry limit counts in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LimitPeriod {
    /// Per store day.
    Day,
    /// Per ISO week.
    Week,
    /// For the whole event.
    Event,
    /// No limit.
    Unlimited,
}

/// How many entries one member may make.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntryLimit {
    /// Entries per period.
    #[schemars(range(min = 1, max = 1000), example = 1)]
    pub count: u32,
    /// day, week, event or unlimited.
    #[schemars(example = &"day")]
    pub period: LimitPeriod,
}

/// When shoppers see the event.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DisplayPeriod {
    /// First moment it shows (store time without an offset).
    #[schemars(example = &"2026-11-06T00:00")]
    pub valid_from: String,
    /// Last moment it shows.
    #[schemars(example = &"2026-11-22T23:59")]
    pub valid_through: String,
}

/// When the draw, the announcement and the rewards happen, in `event.schedule.order`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventSchedule {
    /// When the draw is planned (store time without an offset).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(example = &"2026-11-16T10:00")]
    pub draw_at: Option<String>,
    /// When winners are announced.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(example = &"2026-11-16T12:00")]
    pub announce_at: Option<String>,
    /// When rewards are sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(example = &"2026-11-17T10:00")]
    pub dispatch_at: Option<String>,
}

/// Consents given when entering (`event.eligibility.consent_required` names the ones needed).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EntryConsent {
    /// The event's entry terms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(example = true)]
    pub entry_terms: Option<bool>,
    /// Being contacted if the entry wins.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(example = true)]
    pub winner_contact: Option<bool>,
    /// The store's terms.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(example = true)]
    pub terms: Option<bool>,
    /// The privacy notice.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(example = true)]
    pub privacy: Option<bool>,
    /// Marketing by email.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(example = false)]
    pub marketing_email: Option<bool>,
}

impl EntryConsent {
    pub(crate) fn given(&self, kind: &str) -> bool {
        let v = match kind {
            "entry_terms" => self.entry_terms,
            "winner_contact" => self.winner_contact,
            "terms" => self.terms,
            "privacy" => self.privacy,
            "marketing_email" => self.marketing_email,
            _ => None,
        };
        v == Some(true)
    }
}

/// A stored event.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub(crate) struct EventRow {
    pub id: String,
    pub name: String,
    pub event_status: String,
    pub start_date: DateTime<Utc>,
    pub end_date: DateTime<Utc>,
    pub display_valid_from: Option<DateTime<Utc>>,
    pub display_valid_through: Option<DateTime<Utc>>,
    pub entry_mechanic: String,
    pub selection_method: String,
    pub entry_payload_schema: Option<Value>,
    pub reward_tiers: Value,
    pub eligibility: Value,
    pub valid_for_member_tier: Option<String>,
    pub entry_limit: Value,
    pub draw_at: Option<DateTime<Utc>>,
    pub announce_at: Option<DateTime<Utc>>,
    pub dispatch_at: Option<DateTime<Utc>>,
    pub compliance_jurisdictions: Vec<String>,
    pub seed_commitment: String,
    pub draw_seed: String,
    pub change_set_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub version: i32,
}

impl EventRow {
    pub(crate) fn tiers(&self) -> Vec<RewardTier> {
        let mut t: Vec<RewardTier> =
            serde_json::from_value(self.reward_tiers.clone()).unwrap_or_default();
        t.sort_by_key(|r| r.rank);
        t
    }

    pub(crate) fn eligibility(&self) -> Eligibility {
        serde_json::from_value(self.eligibility.clone()).unwrap_or_default()
    }

    pub(crate) fn limit(&self) -> Option<EntryLimit> {
        serde_json::from_value(self.entry_limit.clone()).ok()
    }

    pub(crate) fn status(&self, now: DateTime<Utc>) -> EventStatus {
        event_status(&self.event_status, self.start_date, self.end_date, now)
    }
}

const EVENT_SELECT: &str = "SELECT id, name, event_status, start_date, end_date,
        display_valid_from, display_valid_through, entry_mechanic, selection_method,
        entry_payload_schema, reward_tiers, eligibility, valid_for_member_tier, entry_limit,
        draw_at, announce_at, dispatch_at, compliance_jurisdictions, seed_commitment, draw_seed,
        change_set_id, created_at, version
     FROM promotion.event WHERE tenant_id = $1";

pub(crate) async fn event(tx: &mut TxCx, id: &str) -> Result<Option<EventRow>, ToolError> {
    sqlx::query_as::<_, EventRow>(sqlx::AssertSqlSafe(format!("{EVENT_SELECT} AND id = $2")))
        .bind(tx.tenant.as_str())
        .bind(id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)
}

/// The event, or `not_found` with the next tool.
pub(crate) async fn known_event(
    tx: &mut TxCx,
    id: &str,
    next: &'static str,
) -> Result<EventRow, ToolError> {
    check_id("event_id", "evt", id, false)?;
    event(tx, id)
        .await?
        .ok_or_else(|| not_found("event", id, next))
}

/// Every event, newest start first (a store runs few).
pub(crate) async fn events(tx: &mut TxCx) -> Result<Vec<EventRow>, ToolError> {
    sqlx::query_as::<_, EventRow>(sqlx::AssertSqlSafe(format!(
        "{EVENT_SELECT} ORDER BY start_date DESC, id"
    )))
    .bind(tx.tenant.as_str())
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Moves an event's stored status; `false` when it was not in `from`.
pub(crate) async fn set_event_status(
    tx: &mut TxCx,
    id: &str,
    from: &str,
    to: &str,
) -> Result<bool, ToolError> {
    Ok(sqlx::query(
        "UPDATE promotion.event SET event_status = $4, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND event_status = $3",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(from)
    .bind(to)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?
    .rows_affected()
        == 1)
}

/// An event's status now: `scheduled` and `running` follow the entry period and the clock
/// (closed once it ends); the rest are set by people or by draws and stay.
pub(crate) fn event_status(
    stored: &str,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
    now: DateTime<Utc>,
) -> EventStatus {
    match stored.parse::<EventStatus>() {
        Ok(EventStatus::Scheduled | EventStatus::Running) => {
            if now < start {
                EventStatus::Scheduled
            } else if now >= end + Duration::minutes(1) {
                EventStatus::Closed
            } else {
                EventStatus::Running
            }
        }
        Ok(s) => s,
        Err(_) => EventStatus::Draft,
    }
}

/// One entry.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub(crate) struct EntryRow {
    pub id: String,
    pub event_id: String,
    pub customer_id: String,
    pub entry_status: String,
    pub order_id: Option<String>,
    pub period_bucket: String,
    pub entry_number: i32,
    pub rank: Option<i32>,
    pub waitlist_position: Option<i32>,
    pub draw_record_id: Option<String>,
    pub start_time: DateTime<Utc>,
    pub version: i32,
}

const ENTRY_SELECT: &str = "SELECT id, event_id, customer_id, entry_status, order_id,
        period_bucket, entry_number, rank, waitlist_position, draw_record_id, start_time, version
     FROM promotion.event_entry WHERE tenant_id = $1";

/// Every entry of an event, by id (the order a draw's snapshot uses).
pub(crate) async fn entries(tx: &mut TxCx, event_id: &str) -> Result<Vec<EntryRow>, ToolError> {
    sqlx::query_as::<_, EntryRow>(sqlx::AssertSqlSafe(format!(
        "{ENTRY_SELECT} AND event_id = $2 ORDER BY id"
    )))
    .bind(tx.tenant.as_str())
    .bind(event_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A member's entries, newest first, of one event or all.
pub(crate) async fn entries_of(
    tx: &mut TxCx,
    customer_id: &str,
    event_id: Option<&str>,
) -> Result<Vec<EntryRow>, ToolError> {
    sqlx::query_as::<_, EntryRow>(sqlx::AssertSqlSafe(format!(
        "{ENTRY_SELECT} AND customer_id = $2 AND ($3::text IS NULL OR event_id = $3)
         ORDER BY start_time DESC, id"
    )))
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(event_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// How many entries the event has, by status.
pub(crate) async fn entry_counts(
    tx: &mut TxCx,
    event_id: &str,
) -> Result<BTreeMap<String, i64>, ToolError> {
    let rows: Vec<(String, i64)> = sqlx::query_as(
        "SELECT entry_status, count(*) FROM promotion.event_entry
         WHERE tenant_id = $1 AND event_id = $2 GROUP BY entry_status",
    )
    .bind(tx.tenant.as_str())
    .bind(event_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(rows.into_iter().collect())
}

/// Sets what a draw or an instant result decided for one entry.
pub(crate) async fn decide_entry(
    tx: &mut TxCx,
    id: &str,
    status: &str,
    rank: Option<i32>,
    waitlist_position: Option<i32>,
    draw_record_id: Option<&str>,
) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE promotion.event_entry SET entry_status = $3, rank = $4, waitlist_position = $5,
           draw_record_id = coalesce($6, draw_record_id), updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(status)
    .bind(rank)
    .bind(waitlist_position)
    .bind(draw_record_id)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// One reward.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub(crate) struct RewardRow {
    pub id: String,
    pub entry_id: String,
    pub customer_id: String,
    pub rank: i32,
    pub reward_type: String,
    pub reward_status: String,
    pub compliance_requirements: Vec<String>,
    pub fulfillment: String,
    pub fulfillment_ref: Option<String>,
}

/// The rewards of an event (or of one member in it).
pub(crate) async fn rewards(
    tx: &mut TxCx,
    event_id: Option<&str>,
    customer_id: Option<&str>,
) -> Result<Vec<RewardRow>, ToolError> {
    sqlx::query_as::<_, RewardRow>(
        "SELECT id, entry_id, customer_id, rank, reward_type, reward_status,
                compliance_requirements, fulfillment, fulfillment_ref
         FROM promotion.event_reward
         WHERE tenant_id = $1 AND ($2::text IS NULL OR event_id = $2)
           AND ($3::text IS NULL OR customer_id = $3)
         ORDER BY id",
    )
    .bind(tx.tenant.as_str())
    .bind(event_id)
    .bind(customer_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The ids of an event's draw records, oldest first.
pub(crate) async fn draw_record_ids(
    tx: &mut TxCx,
    event_id: &str,
) -> Result<Vec<String>, ToolError> {
    sqlx::query_scalar(
        "SELECT id FROM promotion.draw_record WHERE tenant_id = $1 AND event_id = $2
         ORDER BY draw_position",
    )
    .bind(tx.tenant.as_str())
    .bind(event_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// `sha256:<hex>` of a text.
pub(crate) fn sha256(text: &str) -> String {
    let digest = Sha256::digest(text.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("sha256:{hex}")
}

/// A new secret draw seed: 160 random bits, as two lower-case ULIDs' text.
pub(crate) fn new_seed() -> String {
    format!(
        "{}{}",
        acom_core::id::new_suffix(),
        acom_core::id::new_suffix()
    )
}

/// The digest of the entrants a draw chose from: the entry ids in id order, one per line.
pub(crate) fn snapshot_digest(entry_ids: &[String]) -> String {
    let mut ids = entry_ids.to_vec();
    ids.sort();
    sha256(&ids.join("\n"))
}

/// `seeded_shuffle`: the entries ordered by `sha256(seed:position:entry_id)`. Anyone holding
/// the published seed and the entrant snapshot can repeat it.
pub(crate) fn seeded_order(seed: &str, position: i32, entry_ids: &[String]) -> Vec<String> {
    let mut keyed: Vec<(String, &String)> = entry_ids
        .iter()
        .map(|id| (sha256(&format!("{seed}:{position}:{id}")), id))
        .collect();
    keyed.sort();
    keyed.into_iter().map(|(_, id)| id.clone()).collect()
}

/// An instant-win roll for one entry: a number in [0, 1) from the secret seed and the entry id.
pub(crate) fn instant_roll(seed: &str, entry_id: &str) -> Decimal {
    let digest = Sha256::digest(format!("{seed}:instant:{entry_id}").as_bytes());
    let mut first = [0u8; 8];
    first.copy_from_slice(&digest[..8]);
    let n = u64::from_be_bytes(first) >> 11;
    Decimal::from(n) / Decimal::from(1u64 << 53)
}

/// The rank an instant roll wins, given the ranks still available: cumulative odds by rank.
pub(crate) fn instant_rank(
    roll: Decimal,
    tiers: &[RewardTier],
    available: &BTreeSet<u32>,
) -> Option<u32> {
    let mut edge = Decimal::ZERO;
    for t in tiers {
        edge += t.odds.unwrap_or_default();
        if roll < edge {
            return available.contains(&t.rank).then_some(t.rank);
        }
    }
    None
}

/// The entry-limit period an entry made at `at` counts in.
pub(crate) fn period_bucket(time: &StoreTime, period: LimitPeriod, at: DateTime<Utc>) -> String {
    let local = at.with_timezone(&time.zone);
    match period {
        LimitPeriod::Day => local.date_naive().to_string(),
        LimitPeriod::Week => {
            let w = local.iso_week();
            format!("{}-W{:02}", w.year(), w.week())
        }
        LimitPeriod::Event => "event".into(),
        LimitPeriod::Unlimited => "unlimited".into(),
    }
}

/// An event's fields after its inputs were read: what `create_event` stores and `update_event`
/// changes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct EventDraft {
    pub name: String,
    pub start_date: DateTime<Utc>,
    pub end_date: DateTime<Utc>,
    pub display_valid_from: Option<DateTime<Utc>>,
    pub display_valid_through: Option<DateTime<Utc>>,
    pub entry_mechanic: String,
    pub selection_method: String,
    pub entry_payload_schema: Option<Value>,
    pub reward_tiers: Vec<RewardTier>,
    pub eligibility: Eligibility,
    pub entry_limit: EntryLimit,
    pub draw_at: Option<DateTime<Utc>>,
    pub announce_at: Option<DateTime<Utc>>,
    pub dispatch_at: Option<DateTime<Utc>>,
    pub compliance_jurisdictions: Vec<String>,
}

impl EventDraft {
    pub(crate) fn from_row(r: &EventRow) -> Self {
        Self {
            name: r.name.clone(),
            start_date: r.start_date,
            end_date: r.end_date,
            display_valid_from: r.display_valid_from,
            display_valid_through: r.display_valid_through,
            entry_mechanic: r.entry_mechanic.clone(),
            selection_method: r.selection_method.clone(),
            entry_payload_schema: r.entry_payload_schema.clone(),
            reward_tiers: r.tiers(),
            eligibility: r.eligibility(),
            entry_limit: r.limit().unwrap_or(EntryLimit {
                count: 1,
                period: LimitPeriod::Event,
            }),
            draw_at: r.draw_at,
            announce_at: r.announce_at,
            dispatch_at: r.dispatch_at,
            compliance_jurisdictions: r.compliance_jurisdictions.clone(),
        }
    }
}

/// The store's default entry limit (`event.entry_limit.per_member`).
pub(crate) fn default_limit(cx_policy: &acom_policy::PolicySet) -> Result<EntryLimit, ToolError> {
    let key = "promotion.event.entry_limit.per_member";
    serde_json::from_value(rules::value(cx_policy, key)?.clone()).map_err(|_| rules::missing(key))
}

/// A time input of an event, or `invalid_schedule`.
pub(crate) fn event_time(
    time: &StoreTime,
    field: &str,
    s: &str,
    next: &'static str,
) -> Result<DateTime<Utc>, ToolError> {
    parse_time(time, field, s).map_err(|e| {
        ToolError::new(INVALID_SCHEDULE, e.message)
            .detail(json!({"field": field, "reason": "format"}))
            .next(next)
    })
}

/// The registry item `id` if it is enabled, else `code`.
fn enabled<'a>(
    items: &'a [RegistryItem],
    id: &str,
    code: ErrorCode,
    what: &str,
    next: &'static str,
) -> Result<&'a RegistryItem, ToolError> {
    match items.iter().find(|i| i.id == id) {
        Some(i) if i.enabled => Ok(i),
        Some(_) => Err(ToolError::new(code, format!("The {what} `{id}` is turned off in this store."))
            .detail(json!({"id": id}))
            .next(next)),
        None => Err(ToolError::new(code, format!("This store has no {what} `{id}`."))
            .detail(json!({"id": id, "allowed": items.iter().filter(|i| i.enabled).map(|i| i.id.clone()).collect::<Vec<_>>()}))
            .next(next)),
    }
}

/// The store's compliance rules that apply to the event: those of its jurisdictions, or all of
/// them when it names none (the winner's address decides at dispatch).
pub(crate) fn applicable_rules(
    p: &acom_policy::PolicySet,
    jurisdictions: &[String],
) -> Result<Vec<rules::ComplianceRule>, ToolError> {
    Ok(rules::compliance_rules(p)?
        .into_iter()
        .filter(|r| jurisdictions.is_empty() || jurisdictions.contains(&r.jurisdiction))
        .collect())
}

/// What a winner of `tier` still has to provide under the rules: `(rule jurisdictions, info)`.
pub(crate) fn requirements(rules: &[rules::ComplianceRule], tier: &RewardTier) -> Vec<String> {
    let mut out = BTreeSet::new();
    for r in rules.iter().filter(|r| r.applies(tier.reward_value)) {
        if r.actions.iter().any(|a| a == "collect_winner_info") {
            out.extend(r.winner_info_required.iter().cloned());
        }
        for a in &r.actions {
            match a.as_str() {
                "require_age_check" => {
                    out.insert("age_check".to_owned());
                }
                "winner_pays_tax" => {
                    out.insert("tax_payment".to_owned());
                }
                _ => {}
            }
        }
    }
    out.into_iter().collect()
}

/// Validates an event draft against the store's registries and policies and returns the checks
/// of the preview. Hard problems are errors with the tool that fixes them.
pub(crate) async fn check_draft(
    cx: &acom_store::Cx,
    tx: &mut TxCx,
    d: &EventDraft,
    next: &'static str,
) -> Result<Vec<Check>, ToolError> {
    let p = &cx.policy;
    if d.name.trim().is_empty() {
        return Err(ToolError::invalid_input("`name` must not be empty.")
            .detail(json!({"field": "name", "reason": "missing"})));
    }
    let mechanics = rules::registry(p, "entry_mechanics")?;
    let methods = rules::registry(p, "selection_methods")?;
    let reward_types = rules::registry(p, "reward_types")?;
    let mechanic = enabled(
        &mechanics,
        &d.entry_mechanic,
        TYPE_DISABLED,
        "entry mechanic",
        "list_event_types",
    )?;
    enabled(
        &methods,
        &d.selection_method,
        TYPE_DISABLED,
        "selection method",
        "list_event_types",
    )?;
    if !Combinations::read(p)?.allows(&d.entry_mechanic, &d.selection_method) {
        return Err(ToolError::new(
            COMBINATION_NOT_ALLOWED,
            format!(
                "This store does not combine {} with {}.",
                d.entry_mechanic, d.selection_method
            ),
        )
        .detail(json!({"rule": "event.selection_methods.allowed_combinations"}))
        .next("list_event_types"));
    }
    let schema = d
        .entry_payload_schema
        .clone()
        .or_else(|| mechanic.entry_payload_schema.clone());
    if (d.entry_mechanic == "custom"
        || !mechanic.builtin && mechanic.entry_payload_schema.is_none())
        && schema
            .as_ref()
            .is_none_or(|s| s.as_object().is_none_or(|o| o.is_empty()))
    {
        return Err(ToolError::invalid_input(format!(
            "A `{}` event needs `entry_payload_schema`: the fields an entry sends.",
            d.entry_mechanic
        ))
        .detail(json!({"field": "entry_payload_schema", "reason": "missing"})));
    }
    if let Some(s) = &d.entry_payload_schema
        && !s.is_object()
    {
        return Err(
            ToolError::invalid_input("`entry_payload_schema` is an object {field: type}.")
                .detail(json!({"field": "entry_payload_schema", "reason": "format"})),
        );
    }
    // Times: start ≤ end ≤ draw ≤ announce ≤ dispatch (`event.schedule.order`).
    let order = rules::words(p, "promotion.event.schedule.order", false)?;
    let at = |k: &str| match k {
        "start_date" => Some(d.start_date),
        "end_date" => Some(d.end_date),
        "draw_at" => d.draw_at,
        "announce_at" => d.announce_at,
        "dispatch_at" => d.dispatch_at,
        _ => None,
    };
    let given: Vec<(&String, DateTime<Utc>)> =
        order.iter().filter_map(|k| at(k).map(|t| (k, t))).collect();
    if d.start_date >= d.end_date {
        return Err(ToolError::new(
            INVALID_SCHEDULE,
            "`start_date` must come before `end_date`.",
        )
        .detail(json!({"field": "start_date", "reason": "range", "rule": "event.schedule"}))
        .next(next));
    }
    for w in given.windows(2) {
        if w[0].1 > w[1].1 {
            return Err(ToolError::new(
                INVALID_SCHEDULE,
                format!("`{}` must not come after `{}`.", w[0].0, w[1].0),
            )
            .detail(json!({"field": w[1].0, "reason": "range", "rule": "event.schedule"}))
            .next(next));
        }
    }
    if let (Some(a), Some(b)) = (d.display_valid_from, d.display_valid_through)
        && a >= b
    {
        return Err(ToolError::new(
            INVALID_SCHEDULE,
            "The display period must start before it ends.",
        )
        .detail(json!({"field": "display_period", "reason": "range"}))
        .next(next));
    }
    // Rewards.
    if d.reward_tiers.is_empty() {
        return Err(ToolError::invalid_input("Give at least one reward tier.")
            .detail(json!({"field": "reward_tiers", "reason": "missing"})));
    }
    let mut ranks = BTreeSet::new();
    let grant_max = rules::LoyaltyRules::read(p)?.grant_max;
    for t in &d.reward_tiers {
        if !ranks.insert(t.rank) {
            return Err(
                ToolError::invalid_input(format!("Rank {} is given twice.", t.rank))
                    .detail(json!({"field": "reward_tiers.rank", "reason": "format"})),
            );
        }
        let kind = enabled(
            &reward_types,
            &t.reward_type,
            REWARD_TYPE_DISABLED,
            "reward type",
            "list_reward_types",
        )?;
        if let Some(v) = t.reward_value
            && (v.currency != cx.currency || v.value < Decimal::ZERO)
        {
            return Err(ToolError::invalid_input(format!(
                "Reward values are in {}, zero or more.",
                cx.currency
            ))
            .detail(json!({"field": "reward_tiers.reward_value", "reason": "range"})));
        }
        match kind.fulfillment.as_deref() {
            Some("grant") => match t.reward_value {
                None => {
                    return Err(ToolError::invalid_input(format!(
                        "Rank {}: {} needs `reward_value`.",
                        t.rank, t.reward_type
                    ))
                    .detail(json!({"field": "reward_tiers.reward_value", "reason": "missing"})));
                }
                Some(v) if v.value > grant_max.value => {
                    return Err(ToolError::invalid_input(format!(
                        "Rank {}: {} is over the {} a grant may give (loyalty.grant.max_amount).",
                        t.rank,
                        money_text(v.value, v.currency.as_str()),
                        money_text(grant_max.value, grant_max.currency.as_str())
                    ))
                    .detail(json!({"field": "reward_tiers.reward_value", "reason": "range"})));
                }
                _ => {}
            },
            Some("issue") => {
                let id = t.reward_ref.clone().unwrap_or_default();
                check_id("reward_tiers.reward_ref", "cpn", &id, false)?;
                let c = crate::repo::coupon(tx, &id)
                    .await?
                    .ok_or_else(|| not_found("coupon", &id, "search_coupons"))?;
                if c.application == "automatic_at_checkout" {
                    return Err(ToolError::invalid_input(format!(
                        "{} applies at checkout by itself; a coupon reward must be one that is issued.",
                        c.discount_code
                    ))
                    .detail(json!({"field": "reward_tiers.reward_ref", "reason": "range"})));
                }
            }
            _ => {}
        }
        if t.reward_type == "product"
            && let Some(r) = &t.reward_ref
            && r.starts_with("prd_")
            && acom_catalog::service::for_promotion::products_with_offers(
                tx,
                std::slice::from_ref(r),
            )
            .await?
            .is_empty()
        {
            return Err(not_found("product", r, "search_products"));
        }
    }
    let mut checks = vec![
        Check {
            rule: "event.entry_mechanics".into(),
            passed: true,
            detail: Some(format!("{} is enabled", d.entry_mechanic)),
            fix_tool: None,
        },
        Check {
            rule: "event.selection_methods.allowed_combinations".into(),
            passed: true,
            detail: Some(format!(
                "{} + {} allowed",
                d.entry_mechanic, d.selection_method
            )),
            fix_tool: None,
        },
        Check {
            rule: "event.reward_types".into(),
            passed: true,
            detail: Some(format!(
                "{} {} enabled",
                crate::model::and_list(
                    &d.reward_tiers
                        .iter()
                        .map(|t| t.reward_type.clone())
                        .collect::<BTreeSet<_>>()
                        .into_iter()
                        .collect::<Vec<_>>()
                ),
                if d.reward_tiers.len() == 1 {
                    "is"
                } else {
                    "are"
                }
            )),
            fix_tool: None,
        },
    ];
    if d.selection_method == "instant_win" {
        let most = rules::number(
            p,
            "promotion.event.selection_methods.instant_win.odds_must_sum_at_most",
        )?;
        let step = rules::number(
            p,
            "promotion.event.selection_methods.instant_win.odds_precision",
        )?;
        let mut sum = Decimal::ZERO;
        for t in &d.reward_tiers {
            let o = t.odds.ok_or_else(|| {
                ToolError::invalid_input(format!(
                    "Rank {}: an instant_win event needs `odds`.",
                    t.rank
                ))
                .detail(json!({"field": "reward_tiers.odds", "reason": "missing"}))
            })?;
            if o < Decimal::ZERO || (step > Decimal::ZERO && (o % step) != Decimal::ZERO) {
                return Err(ToolError::invalid_input(format!(
                    "Rank {}: odds go in steps of {step}.",
                    t.rank
                ))
                .detail(json!({"field": "reward_tiers.odds", "reason": "range"})));
            }
            sum += o;
        }
        if sum > most {
            return Err(ToolError::new(
                ODDS_EXCEED_ONE,
                format!("The odds add up to {sum}; at most {most}."),
            )
            .detail(json!({"rule": "event.selection_methods.instant_win.odds_must_sum_at_most"}))
            .next("create_event"));
        }
        checks.push(Check {
            rule: "event.selection_methods.instant_win.odds_must_sum_at_most".into(),
            passed: true,
            detail: Some(format!("Odds add up to {}; the rest lose", sum.normalize())),
            fix_tool: None,
        });
    }
    checks.push(Check {
        rule: "event.schedule".into(),
        passed: true,
        detail: Some(match d.draw_at {
            Some(_) => "Draw is after the end date".into(),
            None => "Entries close before anything is drawn or sent".into(),
        }),
        fix_tool: None,
    });
    // Eligibility.
    let e = &d.eligibility;
    if let Some(t) = &e.valid_for_member_tier
        && t != "everyone"
        && acom_customer::service::for_promotion::tier(tx, t)
            .await?
            .is_none()
    {
        return Err(
            ToolError::new(TIER_NOT_FOUND, format!("The store has no tier `{t}`."))
                .detail(json!({"entity": "tier", "id": t}))
                .next("list_tiers"),
        );
    }
    if let Some(s) = &e.segment_id {
        check_id("eligibility.segment_id", "seg", s, false)?;
        if crate::segment::segment(tx, s).await?.is_none() {
            return Err(not_found("segment", s, "list_segments"));
        }
    }
    if let Some(pc) = &e.purchase_condition {
        if let Some(m) = pc.min_subtotal
            && m.currency != cx.currency
        {
            return Err(ToolError::invalid_input(format!("`min_subtotal` is in {}.", cx.currency))
                .detail(json!({"field": "eligibility.purchase_condition.min_subtotal", "reason": "format"})));
        }
        if pc
            .scope
            .as_ref()
            .and_then(|s| s.brand_ids.as_ref())
            .is_some_and(|b| !b.is_empty())
        {
            return Err(ToolError::invalid_input("A purchase condition by brand is not supported yet; use products or categories.")
                .detail(json!({"field": "eligibility.purchase_condition.scope.brand_ids", "reason": "range"})));
        }
    }
    // Compliance: what the store's rules ask of winners, by rank.
    let applicable = applicable_rules(p, &d.compliance_jurisdictions)?;
    let mut lines = Vec::new();
    let mut missing_value = Vec::new();
    for t in &d.reward_tiers {
        let hits: Vec<&rules::ComplianceRule> = applicable
            .iter()
            .filter(|r| r.applies(t.reward_value))
            .collect();
        let threshold_rules = applicable.iter().any(|r| r.threshold.is_some());
        if threshold_rules && t.reward_value.is_none() {
            missing_value.push(t.rank);
        }
        if !hits.is_empty() {
            let actions: BTreeSet<String> = hits
                .iter()
                .flat_map(|r| r.actions.iter().map(|a| a.replace('_', " ")))
                .collect();
            lines.push(format!(
                "Rank {} reward triggers {} store rule{}: {}",
                t.rank,
                hits.len(),
                if hits.len() == 1 { "" } else { "s" },
                actions.into_iter().collect::<Vec<_>>().join(", ")
            ));
        }
    }
    let value_rule = rules::word(p, "promotion.event.reward_types.value_required_when")?;
    let compliance_ok = missing_value.is_empty() || value_rule != "compliance_applies";
    checks.push(Check {
        rule: "event.compliance.rules".into(),
        passed: compliance_ok,
        detail: Some(if !compliance_ok {
            format!(
                "Rank {} needs a reward_value: a store compliance rule looks at the value",
                missing_value
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        } else if lines.is_empty() {
            "No store compliance rule applies".into()
        } else {
            lines.join("; ")
        }),
        fix_tool: (!compliance_ok).then(|| next.into()),
    });
    Ok(checks)
}

/// The event as the reads show it.
pub(crate) fn event_json(time: &StoreTime, r: &EventRow, now: DateTime<Utc>) -> Value {
    let t = |x: Option<DateTime<Utc>>| x.map_or(Value::Null, |x| json!(time.format(x)));
    let tiers: Vec<Value> = r
        .tiers()
        .iter()
        .map(|t| {
            let mut v = serde_json::to_value(t).unwrap_or_default();
            if let Some(m) = t.reward_value {
                v["reward_value"] = crate::money(m.value, m.currency.as_str());
            }
            v
        })
        .collect();
    let mut eligibility = r.eligibility.clone();
    if eligibility.get("valid_for_member_tier").is_none() {
        eligibility["valid_for_member_tier"] = json!(audience(r.valid_for_member_tier.as_deref()));
    }
    json!({
        "id": r.id,
        "name": r.name,
        "status": r.status(now),
        "entry_mechanic": r.entry_mechanic,
        "selection_method": r.selection_method,
        "start_date": time.format(r.start_date),
        "end_date": time.format(r.end_date),
        "display_period": {"valid_from": t(r.display_valid_from.or(Some(r.start_date))),
                           "valid_through": t(r.display_valid_through.or(Some(r.end_date)))},
        "reward_tiers": tiers,
        "eligibility": eligibility,
        "entry_limit": r.entry_limit,
        "entry_payload_schema": r.entry_payload_schema,
        "schedule": {"draw_at": t(r.draw_at), "announce_at": t(r.announce_at), "dispatch_at": t(r.dispatch_at)},
        "compliance_jurisdictions": r.compliance_jurisdictions,
        "seed_commitment": r.seed_commitment,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn the_shuffle_repeats_from_the_seed() {
        let ids: Vec<String> = (1..=6).map(|n| format!("ent_{n}")).collect();
        let a = seeded_order("seed", 1, &ids);
        assert_eq!(a, seeded_order("seed", 1, &ids));
        assert_ne!(a, seeded_order("seed", 2, &ids));
        assert_ne!(a, seeded_order("other", 1, &ids));
        let mut sorted = a.clone();
        sorted.sort();
        assert_eq!(sorted, ids);
        assert_eq!(snapshot_digest(&ids), snapshot_digest(&a));
        assert!(sha256("x").starts_with("sha256:") && sha256("x").len() == 71);
        assert_ne!(new_seed(), new_seed());
    }

    #[test]
    fn instant_rolls_fall_in_the_odds() {
        let tiers = vec![RewardTier {
            rank: 1,
            reward_type: "coupon".into(),
            reward_ref: None,
            reward_value: None,
            winner_count: 1,
            waitlist_count: None,
            odds: Some(Decimal::new(5, 1)),
            round: None,
        }];
        let all = BTreeSet::from([1]);
        assert_eq!(instant_rank(Decimal::new(4, 1), &tiers, &all), Some(1));
        assert_eq!(instant_rank(Decimal::new(6, 1), &tiers, &all), None);
        assert_eq!(
            instant_rank(Decimal::new(4, 1), &tiers, &BTreeSet::new()),
            None
        );
        let r = instant_roll("seed", "ent_1");
        assert!(r >= Decimal::ZERO && r < Decimal::ONE);
        assert_eq!(r, instant_roll("seed", "ent_1"));
    }

    #[test]
    fn status_and_buckets_follow_the_store_clock() {
        let time = StoreTime::new(chrono_tz::America::New_York);
        let at = |s: &str| time.parse(s).unwrap();
        let (s, e) = (at("2026-11-09T00:00"), at("2026-11-15T23:59"));
        assert_eq!(
            event_status("scheduled", s, e, at("2026-11-06T11:00")),
            EventStatus::Scheduled
        );
        assert_eq!(
            event_status("scheduled", s, e, at("2026-11-10T09:00")),
            EventStatus::Running
        );
        assert_eq!(
            event_status("running", s, e, at("2026-11-16T00:00")),
            EventStatus::Closed
        );
        assert_eq!(
            event_status("draft", s, e, at("2026-11-10T09:00")),
            EventStatus::Draft
        );
        assert_eq!(
            period_bucket(&time, LimitPeriod::Day, at("2026-11-09T23:30")),
            "2026-11-09"
        );
        assert_eq!(
            period_bucket(&time, LimitPeriod::Week, at("2026-11-09T09:00")),
            "2026-W46"
        );
        assert_eq!(
            period_bucket(&time, LimitPeriod::Event, at("2026-11-09T09:00")),
            "event"
        );
    }

    #[test]
    fn shapes_are_closed() {
        for s in [
            closed_schema_for::<RewardTier>(),
            closed_schema_for::<Eligibility>(),
            closed_schema_for::<EntryLimit>(),
            closed_schema_for::<EventSchedule>(),
            closed_schema_for::<EntryConsent>(),
        ] {
            assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        }
    }
}
