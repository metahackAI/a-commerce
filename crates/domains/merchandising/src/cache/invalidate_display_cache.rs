//! `invalidate_display_cache` (docs/api/merchandising.md#cache).
//!
//! A deterministic stand-in while the store runs no display cache (see the module above): the
//! preview lists the cache keys a clear would cover, the run records the request and clears
//! nothing, and both say so.

use acom_contract::{
    ActionType, Audience, Change, ChangeOp, Check, Committed, ErrorClass, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::CacheResourceType;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Duration, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{ClearRow, clear, full_clears_since, insert};
use crate::display_slot::StorePage;
use crate::model::DisplayRules;
use crate::{check_id, policy_approval, repo};

/// A kind of display content the store does not allow clearing
/// (`cache_invalidation.allowed_resource_types`).
pub const INVALID_RESOURCE_TYPE: ErrorCode =
    ErrorCode::new("invalid_resource_type", ErrorClass::InvalidInput);

/// What the stand-in says about every clear.
const NOTHING_CLEARED: &str = "No display cache runs in this store yet: pages read the display when asked, so the request is recorded and nothing needs clearing";

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InvalidateDisplayCacheInput {
    /// What changed: category, display_slot, banner, popup or collection_page; all clears every
    /// page (the store limits how often per hour).
    #[schemars(example = &"display_slot")]
    pub resource_type: CacheResourceType,
    /// The one item that changed, matching resource_type (`cat_`, `slot_`, `bnr_`, `pop_` or
    /// `col_`). Every item of the kind when left out; not used with all.
    #[schemars(regex(pattern = r"^(cat|slot|bnr|pop|col)_[a-z0-9_]+$"), example = &"slot_home_hero")]
    pub resource_id: Option<String>,
    /// Store pages to clear as well: main, category, product, search.
    #[schemars(length(max = 4), example = ["main"])]
    pub pages: Option<Vec<StorePage>>,
    /// Why, in a few words, for the record.
    #[schemars(length(max = 200), example = &"VIP weekend banner booked")]
    pub reason: Option<String>,
}

/// What `invalidate_display_cache` records.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClearPlan {
    pub id: String,
    pub resource_type: String,
    pub resource_id: Option<String>,
    pub pages: Vec<String>,
    pub keys: Vec<String>,
    pub reason: Option<String>,
}

/// Clear the display cache after a display change.
#[derive(Debug)]
pub struct InvalidateDisplayCache;

impl Operation for InvalidateDisplayCache {
    const META: OpMeta = OpMeta {
        address: "merchandising.cache.invalidate",
        tool: "invalidate_display_cache",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/merchandising/cache/invalidate",
        summary: "Clear what store pages cache of the display after a change to category mappings, slots, banners, popups or curated pages, as narrowly as possible. The store runs no display cache yet, so this records the request and clears nothing; it says which keys it would clear.",
        consumes: &[
            Slot::one("slot_id"),
            Slot::one("category_id"),
            Slot::one("collection_page_id"),
        ],
        produces: &[Slot::one("cache_invalidation_id")],
        checks: &[
            "cache_invalidation.allowed_resource_types",
            "cache_invalidation.all_scope_max_per_hour",
        ],
        errors: &[
            INVALID_RESOURCE_TYPE,
            ErrorCode::RATE_LIMITED,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::DELETE,
        person_scope: "",
    };
    type Input = InvalidateDisplayCacheInput;
    type Output = Value;
}

/// The id prefix of a kind's items.
fn prefix(kind: CacheResourceType) -> Option<&'static str> {
    match kind {
        CacheResourceType::Category => Some("cat"),
        CacheResourceType::DisplaySlot => Some("slot"),
        CacheResourceType::Banner => Some("bnr"),
        CacheResourceType::Popup => Some("pop"),
        CacheResourceType::CollectionPage => Some("col"),
        CacheResourceType::All => None,
    }
}

const PAGES: [&str; 4] = ["main", "category", "product", "search"];

/// The cache keys a clear covers: the item (`display_slot:slot_home_hero`) or the whole kind
/// (`display_slot:*`), and each page (`page:main`); everything (`*` and every page) for all.
pub(crate) fn cache_keys(kind: &str, id: Option<&str>, pages: &[String]) -> Vec<String> {
    if kind == "all" {
        return std::iter::once("*".to_owned())
            .chain(PAGES.iter().map(|p| format!("page:{p}")))
            .collect();
    }
    let mut keys = vec![format!("{kind}:{}", id.unwrap_or("*"))];
    for p in PAGES {
        if pages.iter().any(|x| x == p) {
            keys.push(format!("page:{p}"));
        }
    }
    keys
}

/// The pages an item shows on, so clearing it clears them too; `NOT_FOUND` when there is no
/// such item.
async fn pages_of(
    tx: &mut TxCx,
    kind: CacheResourceType,
    id: &str,
) -> Result<Vec<String>, ToolError> {
    let missing = |entity: &str, next: &'static str| {
        ToolError::new(
            ErrorCode::NOT_FOUND,
            format!("There is no {} {id}.", entity.replace('_', " ")),
        )
        .detail(json!({"entity": entity, "id": id}))
        .next(next)
    };
    Ok(match kind {
        CacheResourceType::Category => {
            repo::category(tx, id)
                .await?
                .ok_or_else(|| missing("category", "list_categories"))?;
            vec!["category".to_owned()]
        }
        CacheResourceType::DisplaySlot => vec![
            repo::slot(tx, id)
                .await?
                .ok_or_else(|| missing("display_slot", "list_display_slots"))?
                .page,
        ],
        CacheResourceType::Banner => {
            repo::banner(tx, id)
                .await?
                .ok_or_else(|| missing("banner", "list_banners"))?;
            Vec::new()
        }
        CacheResourceType::Popup => {
            crate::popup::popup(tx, id)
                .await?
                .ok_or_else(|| missing("popup", "list_popups"))?
                .page
        }
        CacheResourceType::CollectionPage => {
            crate::collection_page::page(tx, id)
                .await?
                .ok_or_else(|| missing("collection_page", "list_curations"))?;
            Vec::new()
        }
        CacheResourceType::All => Vec::new(),
    })
}

/// `cache_invalidation.all_scope_max_per_hour`, from the clears of everything in the last hour.
fn rate_check(rules: &DisplayRules, kind: &str, recent: &[DateTime<Utc>]) -> Check {
    let max = rules.all_scope_max_per_hour;
    if kind != "all" {
        return Check {
            rule: "cache_invalidation.all_scope_max_per_hour".into(),
            passed: true,
            detail: Some("Narrow clear; the hourly limit is only for all".into()),
            fix_tool: None,
        };
    }
    let n = i64::try_from(recent.len()).unwrap_or(i64::MAX);
    let passed = n < max;
    Check {
        rule: "cache_invalidation.all_scope_max_per_hour".into(),
        passed,
        detail: Some(format!(
            "{n} clear{} of everything in the last hour; at most {max}",
            if n == 1 { "" } else { "s" }
        )),
        fix_tool: (!passed).then(|| "invalidate_display_cache".into()),
    }
}

impl WriteOp<Cx> for InvalidateDisplayCache {
    type Plan = ClearPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: InvalidateDisplayCacheInput,
    ) -> Result<Planned<ClearPlan>, ToolError> {
        let rules = DisplayRules::from_policy(&cx.policy)?;
        let kind = input.resource_type;
        if !rules
            .cache_resource_types
            .iter()
            .any(|t| t == kind.as_str())
        {
            return Err(ToolError::new(
                INVALID_RESOURCE_TYPE,
                format!("The store does not clear `{}` caches.", kind.as_str()),
            )
            .detail(json!({"field": "resource_type", "reason": "enum",
                           "allowed": rules.cache_resource_types}))
            .next("invalidate_display_cache"));
        }
        match (prefix(kind), &input.resource_id) {
            (None, Some(_)) => {
                return Err(ToolError::invalid_input(
                    "`resource_id` names one item; leave it out to clear all.",
                )
                .detail(json!({"field": "resource_id", "reason": "format"})));
            }
            (Some(p), Some(id)) => check_id("resource_id", p, id)?,
            _ => {}
        }
        let mut pages: Vec<String> = input
            .pages
            .unwrap_or_default()
            .iter()
            .map(|p| p.word().to_owned())
            .collect();
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        if let Some(id) = &input.resource_id {
            pages.extend(pages_of(&mut tx, kind, id).await?);
        }
        let recent = full_clears_since(&mut tx, now - Duration::hours(1)).await?;
        tx.commit().await.map_err(ToolError::from)?;
        pages.sort_by_key(|p| PAGES.iter().position(|x| x == p));
        pages.dedup();
        let keys = cache_keys(kind.as_str(), input.resource_id.as_deref(), &pages);
        let reason = input
            .reason
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_owned);
        let checks = vec![
            Check {
                rule: "cache_invalidation.allowed_resource_types".into(),
                passed: true,
                detail: Some(format!("{} caches can be cleared", kind.as_str())),
                fix_tool: None,
            },
            rate_check(&rules, kind.as_str(), &recent),
        ];
        let change = Change {
            op: ChangeOp::Create,
            entity: "cache_invalidation".into(),
            id: String::new(),
            from: Value::Null,
            to: json!({"resource_type": kind.as_str(), "resource_id": input.resource_id,
                       "pages": pages, "keys": keys.len(), "cache_keys": keys,
                       "keys_cleared": 0}),
            note: Some(NOTHING_CLEARED.into()),
        };
        Ok(Planned {
            plan: ClearPlan {
                id: format!("cin_{}", acom_core::id::new_suffix()),
                resource_type: kind.as_str().to_owned(),
                resource_id: input.resource_id,
                pages,
                keys,
                reason,
            },
            changes: vec![change],
            checks,
            basis: Vec::new(),
            approval: policy_approval(
                cx,
                Self::META.tool,
                &json!({"reaches_customers": rules.reaches(Self::META.tool)}),
            ),
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &ClearPlan) -> Result<Committed, ToolError> {
        let rules = DisplayRules::from_policy(&tx.policy)?;
        let now = tx.clock.now();
        if plan.resource_type == "all" {
            // Others may have cleared everything since the preview.
            let recent = full_clears_since(tx, now - Duration::hours(1)).await?;
            let check = rate_check(&rules, &plan.resource_type, &recent);
            if !check.passed {
                let wait = recent
                    .first()
                    .map_or(0, |t| (*t + Duration::hours(1) - now).num_seconds().max(1));
                return Err(ToolError::new(
                    ErrorCode::RATE_LIMITED,
                    format!(
                        "Everything was cleared {} times in the last hour; clear only what changed, or wait.",
                        recent.len()
                    ),
                )
                .detail(json!({"rule": "cache_invalidation.all_scope_max_per_hour",
                               "limit": rules.all_scope_max_per_hour,
                               "retry_after_seconds": wait}))
                .next("invalidate_display_cache"));
            }
        }
        insert(
            tx,
            &ClearRow {
                id: plan.id.clone(),
                resource_type: plan.resource_type.clone(),
                resource_id: plan.resource_id.clone(),
                page: plan.pages.clone(),
                cache_key: plan.keys.clone(),
                start_time: now,
            },
            plan.reason.as_deref(),
        )
        .await?;
        Ok(Committed {
            id: plan.id.clone(),
            produced: Map::from_iter([("cache_invalidation_id".to_owned(), json!(plan.id))]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &ClearPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let stored = clear(&mut tx, &plan.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = stored.is_some_and(|c| {
            c.resource_type == plan.resource_type
                && c.resource_id == plan.resource_id
                && c.cache_key == plan.keys
        });
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("clear {} is not recorded as planned", plan.id)]
            },
        })
    }

    /// A clear has nothing to put back: a cache fills again by itself. The record stays.
    async fn undo(&self, _: &mut TxCx, _: &ClearPlan, _: &Committed) -> Result<(), ToolError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{at, policies};

    #[test]
    fn takes_the_spec_input_with_a_closed_schema() {
        let i: InvalidateDisplayCacheInput = serde_json::from_value(json!({
            "resource_type": "display_slot", "resource_id": "slot_home_hero",
            "pages": ["main"], "reason": "VIP weekend banner booked"}))
        .unwrap();
        assert_eq!(i.resource_type, CacheResourceType::DisplaySlot);
        assert!(
            serde_json::from_value::<InvalidateDisplayCacheInput>(
                json!({"resource_type": "everything"})
            )
            .is_err()
        );
        let schema = acom_contract::schema::closed_schema_for::<InvalidateDisplayCacheInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert_eq!(
            schema["properties"]["resource_type"]["enum"],
            json!([
                "category",
                "display_slot",
                "banner",
                "popup",
                "collection_page",
                "all"
            ])
        );
    }

    #[test]
    fn keys_name_the_item_and_its_pages() {
        assert_eq!(
            cache_keys("display_slot", Some("slot_home_hero"), &["main".into()]),
            ["display_slot:slot_home_hero", "page:main"]
        );
        assert_eq!(cache_keys("banner", None, &[]), ["banner:*"]);
        assert_eq!(
            cache_keys("all", None, &[]),
            [
                "*",
                "page:main",
                "page:category",
                "page:product",
                "page:search"
            ]
        );
    }

    #[test]
    fn clearing_everything_is_limited_per_hour() {
        let rules = DisplayRules::from_policy(&policies(false)).unwrap();
        assert!(rate_check(&rules, "display_slot", &[at("2026-11-06T10:30")]).passed);
        assert!(rate_check(&rules, "all", &[at("2026-11-06T10:30")]).passed);
        let c = rate_check(
            &rules,
            "all",
            &[at("2026-11-06T10:15"), at("2026-11-06T10:45")],
        );
        assert!(!c.passed);
        assert_eq!(
            c.detail.as_deref(),
            Some("2 clears of everything in the last hour; at most 2")
        );
        assert_eq!(c.fix_tool.as_deref(), Some("invalidate_display_cache"));
    }
}
