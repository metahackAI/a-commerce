//! `set_popup` (docs/api/merchandising.md#popup).
//!
//! Registers or changes a popup. A popup opens for every shopper on its pages during its
//! period, so the store's `reaches_customers` rule asks a person to approve it.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::PopupType;
use acom_store::{Cx, TxCx};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::{PRIORITY_CONFLICT, PopupRow, TOO_MANY_POPUPS, delete, popup, popups_meeting, save};
use crate::banner::VALID_FROM_IN_PAST;
use crate::display_slot::StorePage;
use crate::model::{DisplayRules, id_candidates, most_at_once, page_words, period_words};
use crate::{INVALID_PERIOD, check_id, parse_time};

/// What the popup shows: words, a picture, or both, and where pressing it leads.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PopupContent {
    /// The words the popup shows. Needed for notice, event and consent popups.
    #[schemars(length(min = 1), example = &"We close at 3 pm on Dec 24 and reopen Dec 26.")]
    pub text: Option<String>,
    /// URL of the picture the popup shows. Needed for an image popup.
    #[schemars(regex(pattern = r"^https?://"), example = &"https://cdn.northwind.example/p/holiday-hours.webp")]
    pub image_url: Option<String>,
    /// Store path the popup opens when pressed, starting with `/`. Opens nothing when left out.
    #[schemars(regex(pattern = r"^/"), example = &"/pages/holiday-hours")]
    pub url: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetPopupInput {
    /// The popup to change (`list_popups`). Leave it out to register a new popup; its id is
    /// made from `name`.
    #[schemars(regex(pattern = r"^pop_[a-z0-9_]+$"), example = &"pop_holiday_hours")]
    pub popup_id: Option<String>,
    /// Name for operators. A new popup without one is named after its text; a changed popup
    /// keeps its name.
    #[schemars(length(min = 1, max = 100), example = &"Holiday hours")]
    pub name: Option<String>,
    /// What the popup is for: image, notice, event or consent.
    #[schemars(example = &"notice")]
    pub popup_type: PopupType,
    /// What the popup shows.
    pub content: PopupContent,
    /// Pages it opens on: main, category, product, search.
    #[schemars(length(min = 1, max = 4), example = ["main"])]
    pub pages: Vec<StorePage>,
    /// Order among the popups of a page, 1 first. Default: the current one, or for a new popup
    /// the first priority no other popup of its pages uses at the time.
    #[schemars(range(min = 1, max = 100), example = 1)]
    pub priority: Option<i32>,
    /// Days a shopper who chose "do not show again" does not see it, up to the store's limit
    /// (policy `popup.suppress_days_max`). Leave it out to offer no such choice.
    #[schemars(range(min = 0, max = 365), example = 1)]
    pub suppress_days: Option<i32>,
    /// First moment it opens (inclusive), not in the past. Without an offset it is read in
    /// store time.
    #[schemars(example = &"2026-12-20T00:00")]
    pub valid_from: String,
    /// Last moment it opens (inclusive, minute precision), after `valid_from`.
    #[schemars(example = &"2026-12-26T23:59")]
    pub valid_through: String,
}

/// What `set_popup` writes, and the popup as it was (`None` when it registers a new one).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetPopupPlan {
    pub popup: PopupRow,
    pub before: Option<PopupRow>,
}

/// Register or change a popup.
#[derive(Debug)]
pub struct SetPopup;

impl Operation for SetPopup {
    const META: OpMeta = OpMeta {
        address: "merchandising.popup.set",
        tool: "set_popup",
        group: Group::Offer,
        kind: Kind::Write,
        method: Method::Put,
        path: "/v1/merchandising/popups/{popup_id}",
        summary: "Register or change a popup (image, notice, event or consent) that opens on store pages for a period, with an optional do-not-show-again period. It opens for every shopper, so a person approves it.",
        consumes: &[],
        produces: &[Slot::one("popup_id")],
        checks: &[
            "popup.max_concurrent_per_page",
            "display.priority_unique",
            "display.start_not_in_past",
        ],
        errors: &[
            TOO_MANY_POPUPS,
            PRIORITY_CONFLICT,
            VALID_FROM_IN_PAST,
            INVALID_PERIOD,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = SetPopupInput;
    type Output = Value;
}

fn field_error(field: &str, reason: &str, message: impl Into<String>) -> ToolError {
    ToolError::invalid_input(message).detail(json!({"field": field, "reason": reason}))
}

fn trimmed(s: Option<&str>) -> Option<String> {
    s.map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

/// The content a popup kind needs (a picture for an image popup, words for the others),
/// trimmed.
fn check_content(kind: PopupType, c: &PopupContent) -> Result<PopupContent, ToolError> {
    let text = trimmed(c.text.as_deref());
    let image = trimmed(c.image_url.as_deref());
    let url = trimmed(c.url.as_deref());
    if let Some(i) = &image
        && !(i.starts_with("https://") || i.starts_with("http://"))
    {
        return Err(field_error(
            "content.image_url",
            "format",
            "`content.image_url` is the picture's web address.",
        ));
    }
    if let Some(u) = &url
        && !u.starts_with('/')
    {
        return Err(field_error(
            "content.url",
            "format",
            "`content.url` is a store path starting with `/`.",
        ));
    }
    match kind {
        PopupType::Image if image.is_none() => Err(field_error(
            "content.image_url",
            "missing",
            "An image popup needs `content.image_url`.",
        )),
        PopupType::Notice | PopupType::Event | PopupType::Consent if text.is_none() => {
            Err(field_error(
                "content.text",
                "missing",
                format!("A {} popup needs `content.text`.", kind.as_str()),
            ))
        }
        _ => Ok(PopupContent {
            text,
            image_url: image,
            url,
        }),
    }
}

/// A name for a new popup that has none: its first words, or its kind.
fn default_name(kind: PopupType, text: Option<&str>) -> String {
    let words: Vec<&str> = text
        .unwrap_or_default()
        .split_whitespace()
        .take(6)
        .collect();
    if words.is_empty() {
        format!("{} popup", kind.as_str())
    } else {
        words.join(" ")
    }
}

/// `pages` as words, each once, in the order given.
fn page_list(pages: &[StorePage]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for p in pages {
        if !out.iter().any(|w| w == p.word()) {
            out.push(p.word().to_owned());
        }
    }
    out
}

/// `the main page and product pages`.
fn pages_words(pages: &[String]) -> String {
    let words: Vec<&str> = pages.iter().map(|p| page_words(p)).collect();
    match words.as_slice() {
        [] => "no page".to_owned(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// `popup.max_concurrent_per_page`: on each of its pages, this popup and the others that open at
/// the same moment stay within the limit.
pub(crate) fn crowd_check(rules: &DisplayRules, p: &PopupRow, others: &[PopupRow]) -> Check {
    let mut worst: Option<(String, usize, Vec<String>)> = None;
    for page in &p.page {
        let on_page: Vec<&PopupRow> = others.iter().filter(|o| o.page.contains(page)).collect();
        let periods: Vec<(DateTime<Utc>, DateTime<Utc>)> = on_page
            .iter()
            .map(|o| (o.valid_from, o.valid_through))
            .collect();
        let at_once = most_at_once(&periods, p.valid_from, p.valid_through) + 1;
        if worst.as_ref().is_none_or(|w| at_once > w.1) {
            worst = Some((
                page.clone(),
                at_once,
                on_page.iter().map(|o| o.id.clone()).collect(),
            ));
        }
    }
    let max = rules.popups_per_page;
    match worst {
        Some((page, n, ids)) if n > max => Check {
            rule: "popup.max_concurrent_per_page".into(),
            passed: false,
            detail: Some(format!(
                "{n} popups would open at once on {} ({} already); at most {max}",
                page_words(&page),
                ids.join(", ")
            )),
            fix_tool: Some("list_popups".into()),
        },
        Some((page, n, _)) => Check {
            rule: "popup.max_concurrent_per_page".into(),
            passed: true,
            detail: Some(format!(
                "At most {n} popup{} at once on {}; the limit is {max}",
                if n == 1 { "" } else { "s" },
                page_words(&page)
            )),
            fix_tool: None,
        },
        None => Check {
            rule: "popup.max_concurrent_per_page".into(),
            passed: true,
            detail: Some(format!("No other popup; the limit is {max}")),
            fix_tool: None,
        },
    }
}

/// `display.priority_unique`: no popup that shares a page and a moment has the same priority.
pub(crate) fn priority_check(p: &PopupRow, others: &[PopupRow]) -> Check {
    let clashes: Vec<String> = others
        .iter()
        .filter(|o| o.priority == p.priority && o.page.iter().any(|w| p.page.contains(w)))
        .map(|o| format!("{} also has priority {}", o.id, o.priority))
        .collect();
    Check {
        rule: "display.priority_unique".into(),
        passed: clashes.is_empty(),
        detail: Some(if clashes.is_empty() {
            format!("Priority {} is free on its pages", p.priority)
        } else {
            clashes.join("; ")
        }),
        fix_tool: (!clashes.is_empty()).then(|| "list_popups".into()),
    }
}

/// The first priority none of the others on the popup's pages uses.
fn free_priority(pages: &[String], others: &[PopupRow]) -> i32 {
    let used: Vec<i32> = others
        .iter()
        .filter(|o| o.page.iter().any(|w| pages.contains(w)))
        .map(|o| o.priority)
        .collect();
    (1..).find(|n| !used.contains(n)).unwrap_or(1)
}

/// A new id from the name: `pop_holiday_hours`, or `pop_holiday_hours_2` when that is taken.
async fn new_id(tx: &mut TxCx, name: &str) -> Result<String, ToolError> {
    let candidates = id_candidates("pop", name);
    let taken = super::taken_ids(tx, &candidates).await?;
    Ok(candidates
        .into_iter()
        .find(|c| !taken.contains(c))
        .unwrap_or_else(|| format!("pop_{}", acom_core::id::new_suffix())))
}

fn as_change(cx: &Cx, p: &PopupRow) -> Value {
    json!({
        "name": p.name, "popup_type": p.popup_type, "pages": p.page, "priority": p.priority,
        "suppress_days": p.suppress_days,
        "valid_from": cx.time.format(p.valid_from), "valid_through": cx.time.format(p.valid_through),
        "content": {"text": p.text, "image_url": p.image, "url": p.url},
    })
}

impl WriteOp<Cx> for SetPopup {
    type Plan = SetPopupPlan;
    const UNDO: UndoSupport = UndoSupport::Supported;

    async fn preview(
        &self,
        cx: &Cx,
        input: SetPopupInput,
    ) -> Result<Planned<SetPopupPlan>, ToolError> {
        let rules = DisplayRules::from_policy(&cx.policy)?;
        if let Some(id) = &input.popup_id {
            check_id("popup_id", "pop", id)?;
        }
        let content = check_content(input.popup_type, &input.content)?;
        let pages = page_list(&input.pages);
        if pages.is_empty() {
            return Err(field_error(
                "pages",
                "missing",
                "`pages` needs at least one page.",
            ));
        }
        if let Some(d) = input.suppress_days
            && !(0..=rules.suppress_days_max).contains(&i64::from(d))
        {
            return Err(field_error(
                "suppress_days",
                "range",
                format!(
                    "`suppress_days` is 0 to {} in this store.",
                    rules.suppress_days_max
                ),
            )
            .detail(json!({"field": "suppress_days", "reason": "range",
                           "limit": rules.suppress_days_max,
                           "policy_key": "merchandising.popup.suppress_days_max"})));
        }
        let from = parse_time(&cx.time, "valid_from", &input.valid_from)?;
        let through = parse_time(&cx.time, "valid_through", &input.valid_through)?;
        if through <= from {
            return Err(ToolError::new(
                INVALID_PERIOD,
                "`valid_through` must come after `valid_from`.",
            )
            .detail(json!({"field": "valid_through", "reason": "range"}))
            .next("set_popup"));
        }
        let now = cx.clock.now();

        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let existing = match &input.popup_id {
            Some(id) => popup(&mut tx, id).await?,
            None => None,
        };
        let start_moves = existing.as_ref().is_none_or(|e| e.valid_from != from);
        if rules.start_not_in_past && start_moves && from < now {
            return Err(ToolError::new(
                VALID_FROM_IN_PAST,
                format!(
                    "The popup would start in the past ({}).",
                    cx.time.format(from)
                ),
            )
            .detail(json!({"field": "valid_from", "reason": "past"}))
            .next("set_popup"));
        }
        let name = trimmed(input.name.as_deref())
            .or_else(|| existing.as_ref().map(|e| e.name.clone()))
            .unwrap_or_else(|| default_name(input.popup_type, content.text.as_deref()));
        let id = match &input.popup_id {
            Some(id) => id.clone(),
            None => new_id(&mut tx, &name).await?,
        };
        let others = popups_meeting(&mut tx, &id, &pages, from, through).await?;
        tx.commit().await.map_err(ToolError::from)?;

        let priority = input
            .priority
            .or_else(|| existing.as_ref().map(|e| e.priority))
            .unwrap_or_else(|| free_priority(&pages, &others));
        let row = PopupRow {
            id: id.clone(),
            name,
            popup_type: input.popup_type.as_str().to_owned(),
            text: content.text,
            image: content.image_url,
            url: content.url,
            page: pages,
            priority,
            suppress_days: input.suppress_days,
            valid_from: from,
            valid_through: through,
            version: 0,
        };
        let checks = vec![
            crowd_check(&rules, &row, &others),
            priority_check(&row, &others),
            Check {
                rule: "display.start_not_in_past".into(),
                passed: true,
                detail: Some(if start_moves {
                    format!("Starts {}, after now", cx.time.format(from))
                } else {
                    format!("Start unchanged ({})", cx.time.format(from))
                }),
                fix_tool: None,
            },
        ];
        let change = Change {
            op: if existing.is_some() {
                ChangeOp::Update
            } else {
                ChangeOp::Create
            },
            entity: "popup".into(),
            id: if existing.is_some() {
                id.clone()
            } else {
                String::new()
            },
            from: existing.as_ref().map_or(Value::Null, |e| as_change(cx, e)),
            to: {
                let mut v = as_change(cx, &row);
                v["popup_id"] = json!(id);
                v
            },
            note: None,
        };
        let facts = json!({"reaches_customers": rules.reaches(Self::META.tool)});
        let approval = if cx.policy.approval_for(Self::META.tool, &facts).needed {
            ApprovalNeed::Needed {
                reason: format!(
                    "Opens on {} for every shopper {}",
                    pages_words(&row.page),
                    period_words(from, Some(through), cx.time.zone)
                ),
            }
        } else {
            ApprovalNeed::NotNeeded
        };
        let basis = existing
            .iter()
            .chain(others.iter())
            .map(|p| Basis {
                entity: "merchandising.popup".into(),
                id: p.id.clone(),
                version: i64::from(p.version),
            })
            .collect();
        Ok(Planned {
            plan: SetPopupPlan {
                popup: row,
                before: existing,
            },
            changes: vec![change],
            checks,
            basis,
            approval,
        })
    }

    async fn commit(&self, tx: &mut TxCx, plan: &SetPopupPlan) -> Result<Committed, ToolError> {
        let rules = DisplayRules::from_policy(&tx.policy)?;
        let p = &plan.popup;
        // Another popup may have been set on these pages since the preview.
        let others = popups_meeting(tx, &p.id, &p.page, p.valid_from, p.valid_through).await?;
        let crowd = crowd_check(&rules, p, &others);
        if !crowd.passed {
            return Err(
                ToolError::new(TOO_MANY_POPUPS, crowd.detail.unwrap_or_default())
                    .detail(json!({"rule": "popup.max_concurrent_per_page",
                           "limit": rules.popups_per_page,
                           "items": others.iter().map(|o| &o.id).collect::<Vec<_>>()}))
                    .next("list_popups"),
            );
        }
        let priority = priority_check(p, &others);
        if !priority.passed {
            return Err(
                ToolError::new(PRIORITY_CONFLICT, priority.detail.unwrap_or_default())
                    .detail(json!({"rule": "display.priority_unique", "priority": p.priority}))
                    .next("list_popups"),
            );
        }
        save(tx, p).await?;
        Ok(Committed {
            id: p.id.clone(),
            produced: Map::from_iter([("popup_id".to_owned(), json!(p.id))]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        plan: &SetPopupPlan,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let stored = popup(&mut tx, &plan.popup.id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = stored.is_some_and(|s| PopupRow { version: 0, ..s } == plan.popup);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!(
                    "popup {} does not read back as planned",
                    plan.popup.id
                )]
            },
        })
    }

    async fn undo(
        &self,
        tx: &mut TxCx,
        plan: &SetPopupPlan,
        _: &Committed,
    ) -> Result<(), ToolError> {
        match &plan.before {
            Some(b) => save(tx, b).await,
            None => {
                let now = tx.clock.now();
                match popup(tx, &plan.popup.id).await? {
                    // Never opened: take it away.
                    Some(p) if now <= p.valid_from => delete(tx, &p.id).await,
                    // Opened already: stop it now.
                    Some(p) if now < p.valid_through => {
                        save(
                            tx,
                            &PopupRow {
                                valid_through: now,
                                ..p
                            },
                        )
                        .await
                    }
                    _ => Ok(()),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{at, policies};

    fn holiday() -> PopupRow {
        PopupRow {
            id: "pop_holiday_hours".into(),
            name: "Holiday hours".into(),
            popup_type: "notice".into(),
            text: Some("We close at 3 pm on Dec 24.".into()),
            image: None,
            url: None,
            page: vec!["main".into()],
            priority: 1,
            suppress_days: Some(1),
            valid_from: at("2026-12-20T00:00"),
            valid_through: at("2026-12-26T23:59"),
            version: 1,
        }
    }

    #[test]
    fn takes_the_spec_input_with_a_closed_schema() {
        let i: SetPopupInput = serde_json::from_value(json!({
            "popup_id": "pop_holiday_hours", "popup_type": "notice",
            "content": {"text": "We close at 3 pm on Dec 24 and reopen Dec 26."},
            "pages": ["main"], "priority": 1, "suppress_days": 1,
            "valid_from": "2026-12-20T00:00:00-05:00", "valid_through": "2026-12-26T23:59:00-05:00"}))
        .unwrap();
        assert!(check_content(i.popup_type, &i.content).is_ok());
        let e = check_content(
            PopupType::Image,
            &PopupContent {
                text: Some("x".into()),
                image_url: None,
                url: None,
            },
        )
        .unwrap_err();
        assert_eq!(e.detail.unwrap()["field"], "content.image_url");
        assert!(
            serde_json::from_value::<SetPopupInput>(json!({
                "popup_type": "notice", "content": {"text": "x", "html": "<b>"},
                "pages": ["main"], "valid_from": "x", "valid_through": "y"}))
            .is_err()
        );
        let schema = acom_contract::schema::closed_schema_for::<SetPopupInput>();
        assert!(
            acom_contract::schema::violations(&schema).is_empty(),
            "{schema:#}"
        );
        assert_eq!(default_name(PopupType::Image, None), "image popup");
        assert_eq!(
            pages_words(&["main".into(), "product".into()]),
            "the main page and product pages"
        );
    }

    #[test]
    fn two_popups_at_once_per_page_and_unique_priorities() {
        let rules = DisplayRules::from_policy(&policies(false)).unwrap();
        let mine = holiday();
        assert!(crowd_check(&rules, &mine, &[]).passed);
        let sale = PopupRow {
            id: "pop_sale".into(),
            priority: 2,
            valid_from: at("2026-12-18T00:00"),
            valid_through: at("2026-12-21T23:59"),
            ..holiday()
        };
        let survey = PopupRow {
            id: "pop_survey".into(),
            priority: 3,
            valid_from: at("2026-12-22T00:00"),
            valid_through: at("2026-12-31T23:59"),
            ..holiday()
        };
        // Sale and survey never meet, so with holiday hours at most two open at once.
        let c = crowd_check(&rules, &mine, &[sale.clone(), survey.clone()]);
        assert!(c.passed, "{c:?}");
        let wrap = PopupRow {
            id: "pop_wrap".into(),
            priority: 4,
            valid_from: at("2026-12-19T00:00"),
            valid_through: at("2026-12-25T23:59"),
            ..holiday()
        };
        let c = crowd_check(&rules, &mine, &[sale.clone(), survey, wrap]);
        assert!(!c.passed);
        assert_eq!(c.fix_tool.as_deref(), Some("list_popups"));

        assert!(priority_check(&mine, std::slice::from_ref(&sale)).passed);
        let same = PopupRow {
            priority: 1,
            ..sale.clone()
        };
        let p = priority_check(&mine, std::slice::from_ref(&same));
        assert!(!p.passed);
        assert_eq!(p.detail.as_deref(), Some("pop_sale also has priority 1"));
        assert_eq!(free_priority(&mine.page, &[same, sale]), 3);
    }
}
