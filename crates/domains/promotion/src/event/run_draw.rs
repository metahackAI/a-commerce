//! `run_draw` (docs/api/promotion.md#event): draws winners, draws again, cancels a winner,
//! promotes or adds to the waitlist, or records winners chosen by hand. Every run adds a draw
//! record (`get_draw_record`) with the seed, the entrant snapshot and the result, so anyone can
//! repeat it: the same seed and the same entrants give the same winners (`seeded_shuffle`).
//! Reaches customers, so a person always approves; after it runs it is fixed by further runs
//! (`cancel_winner`, `promote_waitlist`), not undone.

use std::collections::{BTreeMap, BTreeSet};

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Basis, Change, ChangeOp, Check, Committed, ErrorCode,
    Group, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError, UndoSupport, Verification,
    WriteOp,
};
use acom_core::enums::EventStatus;
use acom_store::{Cx, TxCx};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    ENTRIES_OPEN, EntryRow, NOT_DRAW_EVENT, REASON_REQUIRED, RewardTier, WAITLIST_EMPTY,
    applicable_rules, decide_entry, draw_record_ids, entries, event, known_event, requirements,
    seeded_order, set_event_status, snapshot_digest,
};
use crate::model::count_text;
use crate::repo::db;
use crate::rules;

/// What the run does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DrawMode {
    /// Draw the winners and waitlist of every rank (or of `rank`).
    Draw,
    /// Draw a rank again: its winners and waitlist lose, new ones are drawn (needs `reason`).
    Redraw,
    /// Cancel the wins of `entry_ids` (needs `reason`).
    CancelWinner,
    /// Move people from the waitlist up to the rank's winner count.
    PromoteWaitlist,
    /// Draw `waitlist_count` more people onto the waitlist.
    AddWaitlist,
    /// Make `entry_ids` winners of `rank`, chosen by people (needs `reason`).
    ManualSelect,
}

impl DrawMode {
    fn word(self) -> &'static str {
        match self {
            Self::Draw => "draw",
            Self::Redraw => "redraw",
            Self::CancelWinner => "cancel_winner",
            Self::PromoteWaitlist => "promote_waitlist",
            Self::AddWaitlist => "add_waitlist",
            Self::ManualSelect => "manual_select",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunDrawInput {
    /// The event.
    #[schemars(regex(pattern = r"^evt_[a-z0-9_]+$"), example = &"evt_0311")]
    pub event_id: String,
    /// draw, redraw, cancel_winner, promote_waitlist, add_waitlist or manual_select.
    #[schemars(example = &"draw")]
    pub mode: DrawMode,
    /// The rank to work on. Default: every rank (manual_select: rank 1).
    #[schemars(range(min = 1, max = 100), example = 1)]
    pub rank: Option<u32>,
    /// Winners to draw or promote for `rank`. Default: the rank's `winner_count`.
    #[schemars(range(min = 1, max = 100000), example = 1)]
    pub winner_count: Option<u32>,
    /// Waitlist places to draw for `rank`. Default: the rank's `waitlist_count`.
    #[schemars(range(min = 0, max = 1000), example = 2)]
    pub waitlist_count: Option<u32>,
    /// The entries for `cancel_winner` and `manual_select`.
    #[schemars(inner(regex(pattern = r"^ent_[a-z0-9_]+$")), example = ["ent_5521"])]
    pub entry_ids: Option<Vec<String>>,
    /// Why; needed for cancel_winner, manual_select and redraw.
    #[schemars(length(min = 1, max = 500), example = &"The winner could not be reached.")]
    pub reason: Option<String>,
}

/// What a run decides for one entry.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Decision {
    pub entry_id: String,
    pub from: String,
    pub to: String,
    pub rank: Option<i32>,
    pub waitlist_position: Option<i32>,
}

/// The result of one rank.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RankResult {
    pub rank: u32,
    pub winners: Vec<String>,
    pub waitlist: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cancelled: Vec<String>,
}

/// What a draw decides, before anything is written.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DrawOutcome {
    pub decisions: Vec<Decision>,
    pub results: Vec<RankResult>,
    /// The entries chosen from, by id.
    pub pool: Vec<String>,
    pub previous_winners: usize,
    pub invalid_entries: usize,
}

/// What a draw asks for, after its inputs were read.
#[derive(Debug, Clone, PartialEq)]
pub struct DrawAsk {
    pub mode: DrawMode,
    pub rank: Option<u32>,
    pub winner_count: Option<u32>,
    pub waitlist_count: Option<u32>,
    pub entry_ids: Vec<String>,
    pub exclude_previous_winners: bool,
    pub default_waitlist: u32,
}

fn is(e: &EntryRow, status: &str) -> bool {
    e.entry_status == status
}

/// Decides a run from the entries as they are: pure, so the same seed, position and entries
/// always give the same outcome.
pub fn decide(
    seed: &str,
    position: i32,
    entries: &[EntryRow],
    tiers: &[RewardTier],
    ask: &DrawAsk,
) -> Result<DrawOutcome, ToolError> {
    let ranks: Vec<&RewardTier> = tiers
        .iter()
        .filter(|t| ask.rank.is_none_or(|r| r == t.rank))
        .collect();
    if ranks.is_empty() {
        return Err(ToolError::invalid_input(format!(
            "The event has no rank {}.",
            ask.rank.unwrap_or(0)
        ))
        .detail(json!({"field": "rank", "reason": "range"})));
    }
    let rank_set: BTreeSet<i32> = ranks
        .iter()
        .filter_map(|t| i32::try_from(t.rank).ok())
        .collect();
    let mut out = DrawOutcome {
        invalid_entries: entries.iter().filter(|e| is(e, "invalid")).count(),
        ..DrawOutcome::default()
    };
    let push =
        |out: &mut DrawOutcome, e: &EntryRow, to: &str, rank: Option<i32>, pos: Option<i32>| {
            out.decisions.push(Decision {
                entry_id: e.id.clone(),
                from: e.entry_status.clone(),
                to: to.to_owned(),
                rank,
                waitlist_position: pos,
            });
        };
    let by_id: BTreeMap<&str, &EntryRow> = entries.iter().map(|e| (e.id.as_str(), e)).collect();
    match ask.mode {
        DrawMode::Draw | DrawMode::Redraw | DrawMode::AddWaitlist => {
            let mut taken: BTreeSet<String> = BTreeSet::new();
            if ask.mode == DrawMode::Redraw {
                // The rank's winners and waitlist lose their place.
                for e in entries.iter().filter(|e| {
                    (is(e, "won") || is(e, "waitlist"))
                        && e.rank.is_some_and(|r| rank_set.contains(&r))
                }) {
                    push(&mut out, e, "lost", e.rank, None);
                    taken.insert(e.id.clone());
                }
            }
            let previous: BTreeSet<&str> = entries
                .iter()
                .filter(|e| {
                    is(e, "won") || is(e, "waitlist") || is(e, "cancelled") && e.rank.is_some()
                })
                .map(|e| e.id.as_str())
                .collect();
            out.previous_winners = if ask.exclude_previous_winners {
                previous.len()
            } else {
                0
            };
            let open: &[&str] = match ask.mode {
                DrawMode::Draw => &["entered"],
                _ => &["entered", "lost"],
            };
            let pool: Vec<String> = entries
                .iter()
                .filter(|e| open.contains(&e.entry_status.as_str()))
                .filter(|e| !(ask.exclude_previous_winners && previous.contains(e.id.as_str())))
                .filter(|e| !(ask.mode == DrawMode::Redraw && taken.contains(&e.id)))
                .map(|e| e.id.clone())
                .collect();
            let order = seeded_order(seed, position, &pool);
            let mut next = order.iter();
            for t in &ranks {
                let rank = i32::try_from(t.rank).ok();
                let mut r = RankResult {
                    rank: t.rank,
                    ..RankResult::default()
                };
                let single = ask.rank.is_some();
                let winners = if ask.mode == DrawMode::AddWaitlist {
                    0
                } else {
                    match (single, ask.winner_count) {
                        (true, Some(n)) => n,
                        _ => t.winner_count,
                    }
                };
                let waitlist = match (single, ask.waitlist_count) {
                    (true, Some(n)) => n,
                    _ => t.waitlist_count.unwrap_or(ask.default_waitlist),
                };
                let start = if ask.mode == DrawMode::AddWaitlist {
                    entries
                        .iter()
                        .filter(|e| is(e, "waitlist") && e.rank == rank)
                        .filter_map(|e| e.waitlist_position)
                        .max()
                        .unwrap_or(0)
                } else {
                    0
                };
                for _ in 0..winners {
                    let Some(id) = next.next() else { break };
                    if let Some(e) = by_id.get(id.as_str()) {
                        push(&mut out, e, "won", rank, None);
                    }
                    r.winners.push(id.clone());
                }
                for n in 0..waitlist {
                    let Some(id) = next.next() else { break };
                    if let Some(e) = by_id.get(id.as_str()) {
                        push(
                            &mut out,
                            e,
                            "waitlist",
                            rank,
                            i32::try_from(n + 1).ok().map(|p| p + start),
                        );
                    }
                    r.waitlist.push(id.clone());
                }
                out.results.push(r);
            }
            if ask.mode == DrawMode::Draw && ask.rank.is_none() {
                for id in next {
                    if let Some(e) = by_id.get(id.as_str()) {
                        push(&mut out, e, "lost", None, None);
                    }
                }
            }
            out.pool = pool;
        }
        DrawMode::CancelWinner => {
            let mut results: BTreeMap<u32, RankResult> = BTreeMap::new();
            for id in &ask.entry_ids {
                let e = by_id
                    .get(id.as_str())
                    .ok_or_else(|| crate::not_found("event_entry", id, "get_event"))?;
                if !is(e, "won") {
                    return Err(ToolError::new(ErrorCode::INVALID_STATE, format!("{id} is {}, not a winner.", e.entry_status))
                        .detail(json!({"entity": "event_entry", "from": e.entry_status, "allowed_from": ["won"]}))
                        .next("get_event"));
                }
                push(&mut out, e, "cancelled", e.rank, None);
                let rank = u32::try_from(e.rank.unwrap_or(1)).unwrap_or(1);
                results
                    .entry(rank)
                    .or_insert(RankResult {
                        rank,
                        ..RankResult::default()
                    })
                    .cancelled
                    .push(id.clone());
            }
            out.results = results.into_values().collect();
        }
        DrawMode::PromoteWaitlist => {
            for t in &ranks {
                let rank = i32::try_from(t.rank).ok();
                let won = entries
                    .iter()
                    .filter(|e| is(e, "won") && e.rank == rank)
                    .count() as u32;
                let want = match ask.winner_count {
                    Some(n) if ask.rank.is_some() => n,
                    _ => t.winner_count.saturating_sub(won),
                };
                if want == 0 {
                    continue;
                }
                let mut waiting: Vec<&EntryRow> = entries
                    .iter()
                    .filter(|e| is(e, "waitlist") && e.rank == rank)
                    .collect();
                waiting.sort_by_key(|e| (e.waitlist_position.unwrap_or(i32::MAX), e.id.clone()));
                if waiting.is_empty() {
                    return Err(ToolError::new(
                        WAITLIST_EMPTY,
                        format!("Rank {} has nobody on its waitlist.", t.rank),
                    )
                    .detail(json!({"rank": t.rank}))
                    .next("run_draw"));
                }
                let mut r = RankResult {
                    rank: t.rank,
                    ..RankResult::default()
                };
                for e in waiting.into_iter().take(usize::try_from(want).unwrap_or(0)) {
                    push(&mut out, e, "won", rank, None);
                    r.winners.push(e.id.clone());
                }
                out.results.push(r);
            }
            if out.decisions.is_empty() {
                return Err(ToolError::invalid_input(
                    "Every rank already has its winners; nobody to promote.",
                )
                .detail(json!({"field": "mode", "reason": "range"})));
            }
        }
        DrawMode::ManualSelect => {
            let t = ranks[0];
            let rank = i32::try_from(t.rank).ok();
            let mut r = RankResult {
                rank: t.rank,
                ..RankResult::default()
            };
            for id in &ask.entry_ids {
                let e = by_id
                    .get(id.as_str())
                    .ok_or_else(|| crate::not_found("event_entry", id, "get_event"))?;
                if !matches!(e.entry_status.as_str(), "entered" | "lost" | "waitlist") {
                    return Err(ToolError::new(ErrorCode::INVALID_STATE, format!("{id} is {}; it cannot be chosen.", e.entry_status))
                        .detail(json!({"entity": "event_entry", "from": e.entry_status, "allowed_from": ["entered", "lost", "waitlist"]}))
                        .next("get_event"));
                }
                push(&mut out, e, "won", rank, None);
                r.winners.push(id.clone());
            }
            out.results.push(r);
        }
    }
    Ok(out)
}

/// What a run of `run_draw` writes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawRun {
    pub event_id: String,
    pub draw_record_id: String,
    pub position: i32,
    pub mode: String,
    pub rank: Option<u32>,
    pub method: String,
    pub reason: Option<String>,
    pub agent_id: String,
    pub outcome: DrawOutcome,
    /// The event's stored status after the run.
    pub status_to: Option<String>,
}

pub struct RunDraw;

impl Operation for RunDraw {
    const META: OpMeta = OpMeta {
        address: "promotion.event.run_draw",
        tool: "run_draw",
        group: Group::Engage,
        kind: Kind::WriteApproval,
        method: Method::Post,
        path: "/v1/promotion/events/{event_id}/draw",
        summary: "Draw a random_draw or ranked event's winners after entries close, draw a rank again, cancel a winner, promote or add to the waitlist, or record winners chosen by hand. Each run leaves a repeatable draw record. Reaches customers, so a person always approves.",
        consumes: &[Slot::one("event_id")],
        produces: &[Slot::one("draw_record_id"), Slot::many("entry_id")],
        checks: &[
            "event.draw.requires_entries_closed",
            "event.draw.exclude_previous_winners",
            "event.draw.fairness.seed_commit_before_close",
            "event.compliance.rules",
        ],
        errors: &[
            ENTRIES_OPEN,
            NOT_DRAW_EVENT,
            REASON_REQUIRED,
            WAITLIST_EMPTY,
            ErrorCode::INVALID_STATE,
            ErrorCode::NOT_FOUND,
        ],
        audience: Audience::Operator,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = RunDrawInput;
    type Output = Value;
}

impl WriteOp<Cx> for RunDraw {
    type Plan = DrawRun;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(&self, cx: &Cx, input: RunDrawInput) -> Result<Planned<DrawRun>, ToolError> {
        let p = cx.policy.clone();
        let modes = rules::words(&p, "promotion.event.draw.modes", false)?;
        if !modes.iter().any(|m| m == input.mode.word()) {
            return Err(ToolError::invalid_input(format!(
                "This store does not use `{}` draws.",
                input.mode.word()
            ))
            .detail(json!({"field": "mode", "reason": "enum", "allowed": modes})));
        }
        let reason = input
            .reason
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty())
            .map(str::to_owned);
        let cancel_reason = rules::flag(&p, "promotion.event.draw.cancel_reason_required")?;
        let needs_reason = match input.mode {
            DrawMode::CancelWinner => cancel_reason,
            DrawMode::ManualSelect | DrawMode::Redraw => true,
            _ => false,
        };
        if needs_reason && reason.is_none() {
            return Err(ToolError::new(
                REASON_REQUIRED,
                format!("A {} needs a reason.", input.mode.word().replace('_', " ")),
            )
            .detail(json!({"field": "reason", "reason": "missing"}))
            .next("run_draw"));
        }
        let ids = input.entry_ids.clone().unwrap_or_default();
        if matches!(input.mode, DrawMode::CancelWinner | DrawMode::ManualSelect) && ids.is_empty() {
            return Err(ToolError::invalid_input(format!(
                "`{}` needs `entry_ids`.",
                input.mode.word()
            ))
            .detail(json!({"field": "entry_ids", "reason": "missing"})));
        }
        let now = cx.clock.now();
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let e = known_event(&mut tx, &input.event_id, "search_events").await?;
        let random = e.selection_method == "random_draw";
        let ranked = e.selection_method == "ranked";
        if !random && !ranked {
            return Err(ToolError::new(
                NOT_DRAW_EVENT,
                format!(
                    "{} decides entries by {}; there is nothing to draw.",
                    e.name, e.selection_method
                ),
            )
            .detail(json!({"selection_method": e.selection_method}))
            .next("get_event"));
        }
        if ranked
            && matches!(
                input.mode,
                DrawMode::Draw | DrawMode::Redraw | DrawMode::AddWaitlist
            )
        {
            return Err(ToolError::invalid_input(format!("{} is judged: record its winners with manual_select.", e.name))
                .detail(json!({"field": "mode", "reason": "enum", "allowed": ["manual_select", "cancel_winner", "promote_waitlist"]})));
        }
        let status = e.status(now);
        let closed_needed = rules::flag(&p, "promotion.event.draw.requires_entries_closed")?;
        if closed_needed
            && matches!(
                status,
                EventStatus::Draft
                    | EventStatus::Scheduled
                    | EventStatus::Running
                    | EventStatus::Paused
            )
        {
            return Err(ToolError::new(
                ENTRIES_OPEN,
                format!("{} is {status}; draw after entries close at {}.", e.name, cx.time.format(e.end_date)),
            )
            .detail(json!({"entity": "event", "from": status, "rule": "event.draw.requires_entries_closed"}))
            .next("get_event"));
        }
        if matches!(status, EventStatus::Cancelled | EventStatus::Completed) {
            return Err(ToolError::new(
                ErrorCode::INVALID_STATE,
                format!("{} is {status}.", e.name),
            )
            .detail(json!({"entity": "event", "from": status}))
            .next("get_event"));
        }
        let all = entries(&mut tx, &e.id).await?;
        let position = i32::try_from(draw_record_ids(&mut tx, &e.id).await?.len()).unwrap_or(0) + 1;
        tx.commit().await.map_err(ToolError::from)?;
        let tiers = e.tiers();
        let ask = DrawAsk {
            mode: input.mode,
            rank: input
                .rank
                .or((input.mode == DrawMode::ManualSelect).then_some(1)),
            winner_count: input.winner_count,
            waitlist_count: input.waitlist_count,
            entry_ids: ids,
            exclude_previous_winners: rules::flag(
                &p,
                "promotion.event.draw.exclude_previous_winners",
            )?,
            default_waitlist: u32::try_from(rules::integer(
                &p,
                "promotion.event.draw.waitlist_count",
            )?)
            .unwrap_or(0),
        };
        let outcome = decide(&e.draw_seed, position, &all, &tiers, &ask)?;
        if outcome.decisions.is_empty() {
            return Err(ToolError::invalid_input(
                "Nothing to decide: no entry is left to draw from.",
            )
            .detail(json!({"field": "mode", "reason": "range"})));
        }
        let method = if ranked {
            "judged".to_owned()
        } else {
            rules::word(&p, "promotion.event.draw.fairness.method")?
        };
        // Checks.
        let applicable = applicable_rules(&p, &e.compliance_jurisdictions)?;
        let needing: usize = outcome
            .decisions
            .iter()
            .filter(|d| d.to == "won")
            .filter(|d| {
                let rank = d.rank.and_then(|r| u32::try_from(r).ok()).unwrap_or(1);
                tiers
                    .iter()
                    .find(|t| t.rank == rank)
                    .is_some_and(|t| !requirements(&applicable, t).is_empty())
            })
            .count();
        let committed_in_time = e.created_at <= e.end_date && !e.seed_commitment.is_empty();
        let checks = vec![
            Check {
                rule: "event.draw.requires_entries_closed".into(),
                passed: true,
                detail: Some(format!("Entries closed {}", cx.time.format(e.end_date))),
                fix_tool: None,
            },
            Check {
                rule: "event.draw.exclude_previous_winners".into(),
                passed: true,
                detail: Some(match outcome.previous_winners {
                    0 => "No previous winners".into(),
                    n => format!(
                        "{} previous winner{} left out",
                        count_text(n as i64),
                        if n == 1 { "" } else { "s" }
                    ),
                }),
                fix_tool: None,
            },
            Check {
                rule: "event.draw.fairness.seed_commit_before_close".into(),
                passed: committed_in_time,
                detail: Some(if committed_in_time {
                    format!(
                        "Seed hash {} published before close",
                        &e.seed_commitment[..e.seed_commitment.len().min(15)]
                    )
                } else {
                    "The seed hash was not published before entries closed".into()
                }),
                fix_tool: None,
            },
            Check {
                rule: "event.compliance.rules".into(),
                passed: true,
                detail: Some(match needing {
                    0 => "No store rule applies to these winners".into(),
                    n => format!(
                        "{} winner{} need{} winner info",
                        count_text(n as i64),
                        if n == 1 { "" } else { "s" },
                        if n == 1 { "s" } else { "" }
                    ),
                }),
                fix_tool: None,
            },
        ];
        // Change rows: one per winner, waitlist place or cancellation; the rest in one line.
        let mut changes: Vec<Change> = outcome
            .decisions
            .iter()
            .filter(|d| d.to != "lost" || d.from != "entered")
            .map(|d| Change {
                op: ChangeOp::Update,
                entity: "event_entry".into(),
                id: d.entry_id.clone(),
                from: json!(d.from),
                to: json!(d.to),
                note: d.rank.map(|r| match d.waitlist_position {
                    Some(p) => format!("Rank {r} waitlist #{p}"),
                    None => format!("Rank {r}"),
                }),
            })
            .collect();
        let lost = outcome
            .decisions
            .iter()
            .filter(|d| d.to == "lost" && d.from == "entered")
            .count();
        if lost > 0 {
            changes.push(Change {
                op: ChangeOp::Update,
                entity: "event_entry".into(),
                id: String::new(),
                from: json!("entered"),
                to: json!({"lost": lost}),
                note: None,
            });
        }
        let won = outcome.decisions.iter().filter(|d| d.to == "won").count() as i64;
        let reason_text = match input.mode {
            DrawMode::CancelWinner => format!(
                "Cancels {} win{}",
                count_text(outcome.decisions.len() as i64),
                if outcome.decisions.len() == 1 {
                    ""
                } else {
                    "s"
                }
            ),
            DrawMode::AddWaitlist => format!(
                "Adds {} to the waitlist",
                count_text(outcome.decisions.len() as i64)
            ),
            _ => format!(
                "Picks {} winner{}",
                count_text(won),
                if won == 1 { "" } else { "s" }
            ),
        };
        let status_to = match input.mode {
            DrawMode::Draw | DrawMode::Redraw | DrawMode::ManualSelect => {
                Some("announced".to_owned())
            }
            _ => None,
        };
        let suffix = e.id.trim_start_matches("evt_");
        Ok(Planned {
            plan: DrawRun {
                draw_record_id: format!("drw_{suffix}_{position:02}"),
                event_id: e.id.clone(),
                position,
                mode: input.mode.word().to_owned(),
                rank: ask.rank,
                method,
                reason,
                agent_id: cx.principal.subject.clone(),
                outcome,
                status_to,
            },
            changes,
            checks,
            basis: vec![Basis {
                entity: "promotion.event".into(),
                id: e.id,
                version: i64::from(e.version),
            }],
            approval: ApprovalNeed::Needed {
                reason: reason_text,
            },
        })
    }

    async fn commit(&self, tx: &mut TxCx, run: &DrawRun) -> Result<Committed, ToolError> {
        let e = event(tx, &run.event_id)
            .await?
            .ok_or_else(|| crate::not_found("event", &run.event_id, "search_events"))?;
        let now = tx.clock.now();
        let change_set_id = tx.change_set_id.clone();
        let inserted = sqlx::query(
            "INSERT INTO promotion.draw_record (tenant_id, id, event_id, draw_position, draw_mode,
               rank, method, seed_commitment, seed, entrant_snapshot_digest, entrant_count,
               excluded, result, reason, agent_id, change_set_id, start_time)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)
             ON CONFLICT (tenant_id, id) DO NOTHING",
        )
        .bind(tx.tenant.as_str())
        .bind(&run.draw_record_id)
        .bind(&e.id)
        .bind(run.position)
        .bind(&run.mode)
        .bind(run.rank.and_then(|r| i32::try_from(r).ok()))
        .bind(&run.method)
        .bind(&e.seed_commitment)
        .bind(&e.draw_seed)
        .bind(snapshot_digest(&run.outcome.pool))
        .bind(i32::try_from(run.outcome.pool.len()).unwrap_or(i32::MAX))
        .bind(json!({"previous_winners": run.outcome.previous_winners, "invalid_entries": run.outcome.invalid_entries}))
        .bind(serde_json::to_value(&run.outcome.results).unwrap_or_default())
        .bind(&run.reason)
        .bind(&run.agent_id)
        .bind(change_set_id)
        .bind(now)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?
        .rows_affected();
        if inserted == 1 {
            for d in &run.outcome.decisions {
                decide_entry(
                    tx,
                    &d.entry_id,
                    &d.to,
                    d.rank,
                    d.waitlist_position,
                    Some(&run.draw_record_id),
                )
                .await?;
            }
            if let Some(to) = &run.status_to
                && &e.event_status != to
            {
                set_event_status(tx, &e.id, &e.event_status, to).await?;
            }
        }
        let winners: Vec<&String> = run
            .outcome
            .decisions
            .iter()
            .filter(|d| d.to == "won")
            .map(|d| &d.entry_id)
            .collect();
        Ok(Committed {
            id: run.draw_record_id.clone(),
            produced: serde_json::Map::from_iter([
                ("draw_record_id".to_owned(), json!(run.draw_record_id)),
                ("entry_id".to_owned(), json!(winners)),
            ]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        run: &DrawRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let all = entries(&mut tx, &run.event_id).await?;
        let records = draw_record_ids(&mut tx, &run.event_id).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let mut mismatches = Vec::new();
        if !records.contains(&run.draw_record_id) {
            mismatches.push(format!("{} is missing", run.draw_record_id));
        }
        for d in &run.outcome.decisions {
            if !all
                .iter()
                .any(|e| e.id == d.entry_id && e.entry_status == d.to)
            {
                mismatches.push(format!("{} is not {}", d.entry_id, d.to));
            }
        }
        Ok(Verification {
            matched: mismatches.is_empty(),
            mismatches,
        })
    }

    /// A draw is fixed by later runs (`cancel_winner`, `promote_waitlist`), not undone.
    async fn undo(&self, _: &mut TxCx, _: &DrawRun, _: &Committed) -> Result<(), ToolError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    fn entry(id: &str, status: &str, rank: Option<i32>, pos: Option<i32>) -> EntryRow {
        EntryRow {
            id: id.into(),
            event_id: "evt_x".into(),
            customer_id: format!("cus_{id}"),
            entry_status: status.into(),
            order_id: None,
            period_bucket: "event".into(),
            entry_number: 1,
            rank,
            waitlist_position: pos,
            draw_record_id: None,
            start_time: chrono::DateTime::UNIX_EPOCH,
            version: 1,
        }
    }

    fn tier(rank: u32, winners: u32, waitlist: u32) -> RewardTier {
        RewardTier {
            rank,
            reward_type: "loyalty_points".into(),
            reward_ref: None,
            reward_value: None,
            winner_count: winners,
            waitlist_count: Some(waitlist),
            odds: None,
            round: None,
        }
    }

    fn ask(mode: DrawMode) -> DrawAsk {
        DrawAsk {
            mode,
            rank: None,
            winner_count: None,
            waitlist_count: None,
            entry_ids: Vec::new(),
            exclude_previous_winners: true,
            default_waitlist: 0,
        }
    }

    #[test]
    fn a_draw_is_repeatable_and_decides_everyone() {
        let all: Vec<EntryRow> = (1..=10)
            .map(|n| entry(&format!("ent_{n:02}"), "entered", None, None))
            .collect();
        let tiers = vec![tier(1, 1, 2), tier(2, 3, 0)];
        let a = decide("seed", 1, &all, &tiers, &ask(DrawMode::Draw)).unwrap();
        assert_eq!(
            a,
            decide("seed", 1, &all, &tiers, &ask(DrawMode::Draw)).unwrap()
        );
        assert_eq!(a.results[0].winners.len(), 1);
        assert_eq!(a.results[0].waitlist.len(), 2);
        assert_eq!(a.results[1].winners.len(), 3);
        assert_eq!(a.decisions.iter().filter(|d| d.to == "lost").count(), 4);
        assert_eq!(a.pool.len(), 10);
        // The order is the seeded shuffle of the pool.
        let order = seeded_order("seed", 1, &a.pool);
        assert_eq!(a.results[0].winners[0], order[0]);
        assert_eq!(a.results[0].waitlist, order[1..3].to_vec());
    }

    #[test]
    fn the_waitlist_steps_in_after_a_cancelled_win() {
        let all = vec![
            entry("ent_a", "cancelled", Some(1), None),
            entry("ent_b", "waitlist", Some(1), Some(2)),
            entry("ent_c", "waitlist", Some(1), Some(1)),
        ];
        let tiers = vec![tier(1, 1, 2)];
        let out = decide("seed", 2, &all, &tiers, &ask(DrawMode::PromoteWaitlist)).unwrap();
        assert_eq!(out.results[0].winners, vec!["ent_c"]);
        let none = vec![entry("ent_a", "cancelled", Some(1), None)];
        assert_eq!(
            decide("seed", 2, &none, &tiers, &ask(DrawMode::PromoteWaitlist))
                .unwrap_err()
                .code,
            "waitlist_empty"
        );
        let mut cancel = ask(DrawMode::CancelWinner);
        cancel.entry_ids = vec!["ent_b".into()];
        assert_eq!(
            decide("seed", 2, &all, &tiers, &cancel).unwrap_err().code,
            "invalid_state"
        );
        let s = closed_schema_for::<RunDrawInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
    }
}
