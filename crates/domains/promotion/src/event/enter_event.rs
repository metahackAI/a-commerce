//! `enter_event` (docs/api/promotion.md#event): a signed-in shopper enters an event. The entry
//! period, eligibility, entry limit, consents and the entry data are checked; `instant_win`,
//! `first_come` and `everyone_qualifies` decide at once (and send store points or coupons at
//! once, `event.reward_types.auto_dispatch`), `random_draw` and `ranked` wait for `run_draw`.
//!
//! An instant result is decided when the entry is made, from the event's secret seed and the
//! entry id, so previewing again cannot fish for a win.

use std::collections::BTreeSet;

use acom_catalog::service::for_promotion::products_with_offers;
use acom_contract::{
    ActionType, ApprovalNeed, Audience, Change, ChangeOp, Check, Committed, ErrorCode, Group, Kind,
    Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification, WriteOp,
};
use acom_core::enums::{ActorKind, EventStatus};
use acom_customer::service::for_promotion as customers;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::dispatch_reward::{make_rewards, plan_rewards};
use super::{
    ENTRY_LIMIT_EXCEEDED, EVENT_NOT_ACTIVE, EntryConsent, EntryRow, INVALID_PAYLOAD, LimitPeriod,
    MANDATORY_CONSENT_MISSING, SOLD_OUT, decide_entry, entries, entries_of, event, instant_rank,
    instant_roll, known_event, period_bucket,
};
use crate::benefit::NOT_ELIGIBLE;
use crate::model::money_text;
use crate::service::for_merchandising::category_products;
use crate::service::for_sales::orders;
use crate::{audience, check_id, repo, rules};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EnterEventInput {
    /// The event (`search_events`).
    #[schemars(regex(pattern = r"^evt_[a-z0-9_]+$"), example = &"evt_0311")]
    pub event_id: String,
    /// The entry's data, following the event's `entry_payload_schema` (survey answers, a caption
    /// and image, a code). Leave out when the event asks for none.
    #[schemars(example = json!({}))]
    pub entry_payload: Option<Value>,
    /// For `purchase_triggered` events: the order the entry is based on.
    #[schemars(regex(pattern = r"^ord_[a-z0-9_]+$"), example = &"ord_20518")]
    pub order_id: Option<String>,
    /// The consents given; the ones in `event.eligibility.consent_required` must be true.
    #[schemars(example = json!({"entry_terms": true, "winner_contact": true}))]
    pub entry_consent: EntryConsent,
}

/// What a run of `enter_event` writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntryRun {
    pub entry_id: String,
    pub event_id: String,
    pub customer_id: String,
    pub payload: Value,
    pub consent: EntryConsent,
    pub order_id: Option<String>,
    pub bucket: String,
    pub entry_number: i32,
    /// The member already entered in this period and the limit is one: the same entry again.
    pub already: bool,
}

pub struct EnterEvent;

impl Operation for EnterEvent {
    const META: OpMeta = OpMeta {
        address: "promotion.event.enter",
        tool: "enter_event",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/promotion/events/{event_id}/entries",
        summary: "Enter the signed-in shopper into an event, after checking the period, who may enter, the entry limit, consents and the entry data. Instant events answer at once; drawn events wait for the draw.",
        consumes: &[Slot::one("event_id")],
        produces: &[Slot::one("entry_id")],
        checks: &[
            "event.eligibility",
            "event.entry_limit",
            "event.entry_period",
            "event.payload_validation",
        ],
        errors: &[
            NOT_ELIGIBLE,
            ENTRY_LIMIT_EXCEEDED,
            EVENT_NOT_ACTIVE,
            INVALID_PAYLOAD,
            MANDATORY_CONSENT_MISSING,
            SOLD_OUT,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Shopper,
        action_type: ActionType::REGISTER,
        person_scope: "",
    };
    type Input = EnterEventInput;
    type Output = Value;
}

fn not_eligible(message: String) -> ToolError {
    ToolError::new(NOT_ELIGIBLE, message)
        .detail(json!({"rule": "event.eligibility"}))
        .next("get_event")
}

/// The entry data checked against the fields the event asks for: every field given, no other.
pub(crate) fn check_payload(schema: Option<&Value>, payload: &Value) -> Result<String, ToolError> {
    let fields: Vec<String> = schema
        .and_then(Value::as_object)
        .map(|o| o.keys().cloned().collect())
        .unwrap_or_default();
    let given = payload.as_object().cloned().unwrap_or_default();
    if !payload.is_null() && !payload.is_object() {
        return Err(ToolError::new(
            INVALID_PAYLOAD,
            "`entry_payload` is an object of the event's fields.",
        )
        .detail(json!({"field": "entry_payload", "reason": "format"}))
        .next("get_event"));
    }
    let missing: Vec<&String> = fields
        .iter()
        .filter(|f| given.get(*f).is_none_or(Value::is_null))
        .collect();
    let extra: Vec<&String> = given.keys().filter(|k| !fields.contains(k)).collect();
    if !missing.is_empty() || !extra.is_empty() {
        return Err(ToolError::new(
            INVALID_PAYLOAD,
            if fields.is_empty() {
                "This event takes no entry data.".to_owned()
            } else {
                format!("The entry needs {} and nothing else.", fields.join(", "))
            },
        )
        .detail(json!({"field": "entry_payload", "reason": "format", "missing": missing, "unknown": extra}))
        .next("get_event"));
    }
    Ok(if fields.is_empty() {
        "No entry data needed".into()
    } else {
        format!("{} given", fields.join(", "))
    })
}

impl WriteOp<Cx> for EnterEvent {
    type Plan = EntryRun;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: EnterEventInput,
    ) -> Result<Planned<EntryRun>, ToolError> {
        let customer = cx
            .principal
            .customer_id()
            .map(str::to_owned)
            .ok_or_else(|| {
                ToolError::new(ErrorCode::FORBIDDEN, "Sign in to enter an event.")
                    .detail(json!({"reason": "owner"}))
            })?;
        let now = cx.clock.now();
        let p = cx.policy.clone();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let e = known_event(&mut tx, &input.event_id, "search_events").await?;
        // The entry period.
        let status = e.status(now);
        if status != EventStatus::Running {
            return Err(ToolError::new(
                EVENT_NOT_ACTIVE,
                format!(
                    "{} is {status}; entries are taken from {} to {}.",
                    e.name,
                    cx.time.format(e.start_date),
                    cx.time.format(e.end_date)
                ),
            )
            .detail(json!({"entity": "event", "from": status, "rule": "event.entry_period"}))
            .next("get_event"));
        }
        // Eligibility.
        let el = e.eligibility();
        let tier = customers::tier_of(&mut tx, &customer).await?;
        if let Some(t) = &e.valid_for_member_tier
            && tier.as_deref() != Some(t.as_str())
        {
            return Err(not_eligible(format!("{} is for {t} members.", e.name)));
        }
        if let Some(types) = &el.member_types {
            let mine = customers::member_types(&mut tx, std::slice::from_ref(&customer)).await?;
            let mine = mine.first().map(|(_, t)| t.clone()).unwrap_or_default();
            if !types.iter().any(|t| t.as_str() == mine) {
                return Err(not_eligible(format!(
                    "{} is not open to {mine} members.",
                    e.name
                )));
            }
        }
        if let Some(chans) = &el.channels {
            let here = if cx.principal.kind == ActorKind::ExternalAgent {
                "agent"
            } else {
                "web"
            };
            if !chans.iter().any(|c| c.as_str() == here) {
                return Err(not_eligible(format!(
                    "{} takes entries only from {}.",
                    e.name,
                    chans
                        .iter()
                        .map(|c| c.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )));
            }
        }
        if let Some(seg) = &el.segment_id {
            let (_, members) = crate::segment::members_of(&mut tx, seg, now, "get_event").await?;
            if !members.iter().any(|m| m.customer_id == customer) {
                return Err(not_eligible(format!(
                    "{} is for a group of members you are not in.",
                    e.name
                )));
            }
        }
        let needs_order =
            e.entry_mechanic == "purchase_triggered" || el.purchase_condition.is_some();
        let mut order_words = String::new();
        if needs_order {
            let id = input.order_id.clone().ok_or_else(|| {
                ToolError::invalid_input("This event is entered with an order: give `order_id`.")
                    .detail(json!({"field": "order_id", "reason": "missing"}))
            })?;
            check_id("order_id", "ord", &id, false)?;
            let order = orders(&mut tx, std::slice::from_ref(&id))
                .await?
                .into_iter()
                .find(|o| o.customer_id.as_deref() == Some(customer.as_str()))
                .ok_or_else(|| crate::not_found("order", &id, "search_orders"))?;
            if order.order_status == "cancelled"
                || order.order_date < e.start_date
                || order.order_date > e.end_date + chrono::Duration::minutes(1)
            {
                return Err(not_eligible(format!(
                    "Order {id} was not placed during {} or was cancelled.",
                    e.name
                )));
            }
            if let Some(pc) = &el.purchase_condition {
                if let Some(min) = pc.min_subtotal
                    && order.subtotal.value < min.value
                {
                    return Err(not_eligible(format!(
                        "{} needs an order of {}; {id} is {}.",
                        e.name,
                        money_text(min.value, min.currency.as_str()),
                        money_text(order.subtotal.value, order.subtotal.currency.as_str())
                    )));
                }
                if let Some(scope) = &pc.scope {
                    let mut products: BTreeSet<String> =
                        scope.product_ids.iter().flatten().cloned().collect();
                    if let Some(cats) = &scope.category_ids {
                        products.extend(
                            category_products(&mut tx, cats)
                                .await?
                                .into_iter()
                                .flat_map(|c| c.product_ids),
                        );
                    }
                    for x in scope.exclude_product_ids.iter().flatten() {
                        products.remove(x);
                    }
                    let list: Vec<String> = products.into_iter().collect();
                    let variants: BTreeSet<String> = products_with_offers(&mut tx, &list)
                        .await?
                        .into_iter()
                        .flat_map(|p| p.offers.into_iter().map(|o| o.variant_id))
                        .collect();
                    if !list.is_empty() && !order.variant_ids.iter().any(|v| variants.contains(v)) {
                        return Err(not_eligible(format!(
                            "Order {id} has none of the products {} counts.",
                            e.name
                        )));
                    }
                }
            }
            order_words = format!("; order {id} counts");
        }
        // Consents.
        let required = rules::words(&p, "promotion.event.eligibility.consent_required", true)?;
        let missing: Vec<&String> = required
            .iter()
            .filter(|k| !input.entry_consent.given(k))
            .collect();
        if !missing.is_empty() {
            return Err(ToolError::new(
                MANDATORY_CONSENT_MISSING,
                format!("Entering needs these consents: {}.", required.join(", ")),
            )
            .detail(json!({"rule": "event.eligibility.consent_required", "items": missing})));
        }
        // Entry data.
        let mechanic_schema = rules::registry(&p, "entry_mechanics")?
            .into_iter()
            .find(|m| m.id == e.entry_mechanic)
            .and_then(|m| m.entry_payload_schema);
        let schema = e.entry_payload_schema.clone().or(mechanic_schema);
        let payload = input
            .entry_payload
            .clone()
            .unwrap_or(Value::Object(serde_json::Map::new()));
        let payload_words = check_payload(schema.as_ref(), &payload)?;
        // Entry limit.
        let limit = e.limit().unwrap_or(super::EntryLimit {
            count: 1,
            period: LimitPeriod::Event,
        });
        let bucket = period_bucket(&cx.time, limit.period, now);
        let mine: Vec<EntryRow> = entries_of(&mut tx, &customer, Some(&e.id))
            .await?
            .into_iter()
            .filter(|x| x.period_bucket == bucket && x.entry_status != "cancelled")
            .collect();
        let used = i32::try_from(mine.len()).unwrap_or(i32::MAX);
        let unlimited = limit.period == LimitPeriod::Unlimited;
        let cap = i32::try_from(limit.count).unwrap_or(1);
        let period_words = match limit.period {
            LimitPeriod::Day => "today",
            LimitPeriod::Week => "this week",
            LimitPeriod::Event => "for this event",
            LimitPeriod::Unlimited => "",
        };
        if !unlimited && used >= cap && cap > 1 {
            return Err(ToolError::new(
                ENTRY_LIMIT_EXCEEDED,
                format!("You already entered {used} times {period_words}; the limit is {cap}."),
            )
            .detail(json!({"limit": cap, "actual": used, "policy_key": "event.entry_limit"}))
            .next("list_my_entries"));
        }
        // Instant events with nothing left to win.
        let all = entries(&mut tx, &e.id).await?;
        let tiers = e.tiers();
        if matches!(e.selection_method.as_str(), "first_come" | "instant_win") {
            let left: u32 = tiers
                .iter()
                .map(|t| {
                    let won = all
                        .iter()
                        .filter(|x| x.entry_status == "won" && x.rank == i32::try_from(t.rank).ok())
                        .count() as u32;
                    t.winner_count.saturating_sub(won)
                })
                .sum();
            if left == 0 {
                return Err(ToolError::new(
                    SOLD_OUT,
                    format!("Every prize of {} is taken.", e.name),
                )
                .detail(json!({"entity": "event", "id": e.id}))
                .next("get_event"));
            }
        }
        tx.commit().await.map_err(ToolError::from)?;
        let already = !unlimited && used >= cap;
        let checks = vec![
            Check {
                rule: "event.eligibility".into(),
                passed: true,
                detail: Some(format!(
                    "Open to {}{order_words}",
                    audience(e.valid_for_member_tier.as_deref())
                )),
                fix_tool: None,
            },
            Check {
                rule: "event.entry_limit".into(),
                passed: true,
                detail: Some(if unlimited {
                    "No entry limit".into()
                } else if already {
                    format!("Already entered {period_words}; sending again returns that entry")
                } else {
                    format!("{} of {cap} entries {period_words}", used + 1)
                }),
                fix_tool: None,
            },
            Check {
                rule: "event.entry_period".into(),
                passed: true,
                detail: Some(format!("Entries open until {}", cx.time.format(e.end_date))),
                fix_tool: None,
            },
            Check {
                rule: "event.payload_validation".into(),
                passed: true,
                detail: Some(payload_words),
                fix_tool: None,
            },
        ];
        let (entry_id, changes) = match mine.first() {
            Some(first) if already => (first.id.clone(), Vec::new()),
            _ => (
                repo::new_id("ent"),
                vec![Change {
                    op: ChangeOp::Create,
                    entity: "event_entry".into(),
                    id: String::new(),
                    from: Value::Null,
                    to: json!({"event_id": e.id}),
                    note: None,
                }],
            ),
        };
        Ok(Planned {
            plan: EntryRun {
                entry_id,
                event_id: e.id,
                customer_id: customer,
                payload,
                consent: input.entry_consent,
                order_id: input.order_id,
                bucket,
                entry_number: used + 1,
                already,
            },
            changes,
            checks,
            basis: Vec::new(),
            approval: ApprovalNeed::NotNeeded,
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &EntryRun) -> Result<Committed, ToolError> {
        let e = event(tx, &run.event_id)
            .await?
            .ok_or_else(|| crate::not_found("event", &run.event_id, "search_events"))?;
        if !run.already {
            let now = tx.clock.now();
            sqlx::query(
                "INSERT INTO promotion.event_entry (tenant_id, id, event_id, customer_id,
                   entry_status, entry_payload, entry_consent, order_id, period_bucket,
                   entry_number, start_time)
                 VALUES ($1, $2, $3, $4, 'entered', $5, $6, $7, $8, $9, $10)
                 ON CONFLICT (tenant_id, id) DO NOTHING",
            )
            .bind(tx.tenant.as_str())
            .bind(&run.entry_id)
            .bind(&e.id)
            .bind(&run.customer_id)
            .bind(&run.payload)
            .bind(serde_json::to_value(&run.consent).unwrap_or_default())
            .bind(&run.order_id)
            .bind(&run.bucket)
            .bind(run.entry_number)
            .bind(now)
            .execute(&mut *tx.tx)
            .await
            .map_err(|err| {
                // Two entries for the same slot of the limit at once: the second one loses.
                if err
                    .as_database_error()
                    .is_some_and(|d| d.is_unique_violation())
                {
                    ToolError::new(
                        ENTRY_LIMIT_EXCEEDED,
                        "This entry was already made; list your entries.",
                    )
                    .detail(json!({"policy_key": "event.entry_limit"}))
                    .next("list_my_entries")
                } else {
                    repo::db(err)
                }
            })?;
            decide_now(tx, &e, &run.entry_id).await?;
        }
        let entry = entries(tx, &e.id)
            .await?
            .into_iter()
            .find(|x| x.id == run.entry_id);
        let status = entry
            .as_ref()
            .map_or("entered".to_owned(), |x| x.entry_status.clone());
        let reward = entry
            .as_ref()
            .filter(|x| x.entry_status == "won")
            .and_then(|x| {
                let rank = u32::try_from(x.rank.unwrap_or(1)).ok()?;
                let t = e.tiers().into_iter().find(|t| t.rank == rank)?;
                Some(json!({"rank": rank, "reward_type": t.reward_type}))
            });
        Ok(Committed {
            id: run.entry_id.clone(),
            produced: serde_json::Map::from_iter([
                ("entry_id".to_owned(), json!(run.entry_id)),
                ("entry_status".to_owned(), json!(status)),
                ("reward".to_owned(), reward.unwrap_or(Value::Null)),
            ]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &EntryRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let found = entries_of(&mut tx, &run.customer_id, Some(&run.event_id)).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let matched = found.iter().any(|x| x.id == run.entry_id);
        Ok(Verification {
            matched,
            mismatches: if matched {
                Vec::new()
            } else {
                vec![format!("{} is missing", run.entry_id)]
            },
        })
    }

    async fn undo(&self, _: &mut TxCx, _: &EntryRun, _: &Committed) -> Result<(), ToolError> {
        // A shopper's entry is theirs; it is not part of an operator's change set.
        Ok(())
    }
}

/// Decides an instant event's new entry: `everyone_qualifies` wins the first rank,
/// `first_come` the first rank with a prize left, `instant_win` a rank by its odds. A win
/// whose reward is sent automatically (`event.reward_types.auto_dispatch`) is sent here.
async fn decide_now(tx: &mut TxCx, e: &super::EventRow, entry_id: &str) -> Result<(), ToolError> {
    let tiers = e.tiers();
    let all = entries(tx, &e.id).await?;
    let left: BTreeSet<u32> = tiers
        .iter()
        .filter(|t| {
            let won = all
                .iter()
                .filter(|x| x.entry_status == "won" && x.rank == i32::try_from(t.rank).ok())
                .count() as u32;
            won < t.winner_count
        })
        .map(|t| t.rank)
        .collect();
    let rank = match e.selection_method.as_str() {
        "everyone_qualifies" => tiers.first().map(|t| t.rank),
        "first_come" => left.iter().next().copied(),
        "instant_win" => instant_rank(instant_roll(&e.draw_seed, entry_id), &tiers, &left),
        _ => return Ok(()),
    };
    match rank {
        None => decide_entry(tx, entry_id, "lost", None, None, None).await,
        Some(r) => {
            decide_entry(tx, entry_id, "won", i32::try_from(r).ok(), None, None).await?;
            let p = tx.policy.clone();
            let methods = rules::words(
                &p,
                "promotion.event.reward_types.auto_dispatch.selection_methods",
                true,
            )?;
            let ways = rules::words(
                &p,
                "promotion.event.reward_types.auto_dispatch.fulfillment",
                true,
            )?;
            if !methods.contains(&e.selection_method) {
                return Ok(());
            }
            let winner: Vec<EntryRow> = entries(tx, &e.id)
                .await?
                .into_iter()
                .filter(|x| x.id == entry_id)
                .collect();
            let (plans, _) = plan_rewards(tx, e, &winner).await?;
            let auto: Vec<_> = plans
                .into_iter()
                .filter(|r| ways.contains(&r.fulfillment))
                .collect();
            if !auto.is_empty() {
                make_rewards(tx, e, &repo::new_id("rwb"), &auto).await?;
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn entry_data_follows_the_event_schema() {
        let schema = json!({"image": "ImageObject", "caption": "Text"});
        assert!(
            check_payload(
                Some(&schema),
                &json!({"image": "img_1", "caption": "Trail day"})
            )
            .is_ok()
        );
        let e = check_payload(Some(&schema), &json!({"image": "img_1"})).unwrap_err();
        assert_eq!(e.code, "invalid_payload");
        assert!(check_payload(None, &json!({})).is_ok());
        assert!(check_payload(None, &json!({"x": 1})).is_err());
        let s = closed_schema_for::<EnterEventInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(EnterEvent::META.audience, Audience::Shopper);
    }
}
