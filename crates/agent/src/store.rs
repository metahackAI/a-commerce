//! The `agent` schema (migrations/0012_agent.sql): conversations, turns, steps, and model calls,
//! so a request's model and tool calls can be read in one line with commerce's action log
//! (`request_id`). Every transaction sets `app.tenant_id` for row-level security.
//!
//! Writing the log never fails a turn: a database problem is logged and the turn goes on.
//! [`StoreInfo`] gives the store's time zone, currency, and clock (the demo clock in demo mode).

use std::collections::BTreeMap;
use std::sync::Mutex;

use acom_core::{Clock, StoreTime, SystemClock, TenantId};
use acom_llm::UsageRecord;
use acom_store::{Db, scope_to_tenant};
use chrono::{DateTime, Utc};
use futures::future::BoxFuture;
use serde_json::{Value, json};
use sqlx::Row;

use crate::memory::{ConversationContext, OpenChangeSet, OpenItem, Owner, PastTurn};

/// A turn as it starts.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnStart {
    pub tenant: String,
    pub conversation_id: String,
    pub surface: String,
    pub owner: Owner,
    /// Short title for history lists (the first request).
    pub name: String,
    pub turn_id: String,
    pub text: String,
    pub context: Value,
    pub agent: String,
    pub request_id: String,
    pub started_at: DateTime<Utc>,
}

/// A step, written when it starts and again when it ends.
#[derive(Debug, Clone, PartialEq)]
pub struct StepRow {
    pub tenant: String,
    pub step_id: String,
    pub turn_id: String,
    pub position: i32,
    pub kind: String,
    pub instrument: Option<String>,
    /// `active`, `completed`, or `failed`.
    pub status: String,
    pub summary: String,
    pub input: Option<Value>,
    pub output: Option<Value>,
    pub preview_id: Option<String>,
    pub change_set_id: Option<String>,
    pub error: Option<Value>,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
}

/// A turn as it ends.
#[derive(Debug, Clone, PartialEq)]
pub struct TurnEnd {
    pub tenant: String,
    pub turn_id: String,
    pub agent: String,
    pub reply_text: Option<String>,
    pub blocks: Vec<Value>,
    pub usage: Value,
    pub error: Option<Value>,
    pub ended_at: DateTime<Utc>,
}

/// A conversation's owner and its turns (`{turn_id, text, reply_text, blocks, …}`), oldest first.
pub type History = (Owner, Vec<Value>);

/// One line of a person's conversation list.
#[derive(Debug, Clone, PartialEq)]
pub struct ConversationSummary {
    pub conversation_id: String,
    /// The first request, short (`TurnStart::name`).
    pub name: Option<String>,
    pub turns: usize,
    pub started_at: Option<DateTime<Utc>>,
    pub last_turn_at: Option<DateTime<Utc>>,
}

/// One page of a person's conversations, the most recent first, and how many they have on that
/// surface in all.
pub type ConversationList = (Vec<ConversationSummary>, usize);

/// Where turns are recorded.
pub trait TurnLog: Send + Sync {
    /// The conversation so far, when it exists.
    fn load<'a>(
        &'a self,
        tenant: &'a str,
        conversation_id: &'a str,
        recent: usize,
    ) -> BoxFuture<'a, Result<Option<ConversationContext>, String>>;
    fn start_turn<'a>(&'a self, t: &'a TurnStart) -> BoxFuture<'a, Result<(), String>>;
    fn step<'a>(&'a self, s: &'a StepRow) -> BoxFuture<'a, Result<(), String>>;
    fn model_call<'a>(
        &'a self,
        tenant: &'a str,
        turn_id: &'a str,
        position: i32,
        step_id: Option<&'a str>,
        r: &'a UsageRecord,
    ) -> BoxFuture<'a, Result<(), String>>;
    fn finish_turn<'a>(&'a self, e: &'a TurnEnd) -> BoxFuture<'a, Result<(), String>>;
    /// Every turn of a conversation with its blocks (`GET …/conversations/{id}`).
    fn history<'a>(
        &'a self,
        tenant: &'a str,
        conversation_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<History>, String>>;
    /// The log of each turn of a conversation, by turn id: its steps (inputs, outputs, errors)
    /// and model calls (purpose, model, tokens, errors) in order
    /// (`GET …/conversations/{id}?log=true`). Stores without a log return nothing.
    fn turn_logs<'a>(
        &'a self,
        _tenant: &'a str,
        _conversation_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, Value)>, String>> {
        Box::pin(async { Ok(Vec::new()) })
    }
    /// The conversations one person had on one surface, the most recent first, at most `limit`
    /// (`GET …/conversations`). "One person" is [`Owner::same_person`]. Stores without a list
    /// have none.
    fn conversations<'a>(
        &'a self,
        _tenant: &'a str,
        _surface: &'a str,
        _owner: &'a Owner,
        _limit: usize,
    ) -> BoxFuture<'a, Result<ConversationList, String>> {
        Box::pin(async { Ok((Vec::new(), 0)) })
    }
}

/// The store's time and money settings for a turn.
#[derive(Debug, Clone, PartialEq)]
pub struct StoreSettings {
    pub time: StoreTime,
    /// ISO 4217; empty when unknown.
    pub currency: String,
    pub now: DateTime<Utc>,
}

pub trait StoreInfo: Send + Sync {
    fn settings<'a>(&'a self, tenant: &'a str) -> BoxFuture<'a, Result<StoreSettings, String>>;
}

/// No database: the system clock in UTC and an unknown currency.
#[derive(Debug, Default)]
pub struct SystemStore;

impl StoreInfo for SystemStore {
    fn settings<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<StoreSettings, String>> {
        Box::pin(async {
            Ok(StoreSettings {
                time: StoreTime::new(chrono_tz::UTC),
                currency: String::new(),
                now: SystemClock.now(),
            })
        })
    }
}

/// The Postgres turn log (the `agent` schema). Store settings come from commerce, not here.
#[derive(Debug, Clone)]
pub struct PgStore {
    db: Db,
}

impl PgStore {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    async fn begin(
        &self,
        tenant: &str,
    ) -> Result<sqlx::Transaction<'static, sqlx::Postgres>, String> {
        let tenant = TenantId::parse(tenant).map_err(|e| e.to_string())?;
        let mut tx = self.db.pool().begin().await.map_err(db)?;
        scope_to_tenant(&mut tx, &tenant)
            .await
            .map_err(|e| e.to_string())?;
        Ok(tx)
    }
}

fn db(e: sqlx::Error) -> String {
    format!("agent store: {e}")
}

fn clamp_i32(n: u64) -> i32 {
    i32::try_from(n).unwrap_or(i32::MAX)
}

impl TurnLog for PgStore {
    fn load<'a>(
        &'a self,
        tenant: &'a str,
        conversation_id: &'a str,
        recent: usize,
    ) -> BoxFuture<'a, Result<Option<ConversationContext>, String>> {
        Box::pin(async move {
            let mut tx = self.begin(tenant).await?;
            let Some(conv) = sqlx::query(
                "SELECT customer_id, admin_id, session_id, agent_profile
                 FROM agent.conversation WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tenant)
            .bind(conversation_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?
            else {
                return Ok(None);
            };
            let owner = Owner {
                customer_id: conv.try_get("customer_id").map_err(db)?,
                admin_id: conv.try_get("admin_id").map_err(db)?,
                session_id: conv.try_get("session_id").map_err(db)?,
            };
            let rows = sqlx::query(
                "SELECT text, reply_text, agent_profile, blocks FROM agent.turn
                 WHERE tenant_id = $1 AND conversation_id = $2 ORDER BY position DESC LIMIT $3",
            )
            .bind(tenant)
            .bind(conversation_id)
            .bind(i64::try_from(recent.max(1)).unwrap_or(4))
            .fetch_all(&mut *tx)
            .await
            .map_err(db)?;
            let mut turns = Vec::new();
            for r in rows.iter().rev() {
                let blocks: Value = r.try_get("blocks").map_err(db)?;
                turns.push(PastTurn {
                    text: r.try_get("text").map_err(db)?,
                    reply: r.try_get("reply_text").map_err(db)?,
                    agent: r.try_get("agent_profile").map_err(db)?,
                    blocks: blocks.as_array().cloned().unwrap_or_default(),
                });
            }
            let mut change_set = latest_change_set(&turns);
            if let Some(cs) = &mut change_set {
                let ids: Vec<String> = cs.items.iter().map(|i| i.preview_id.clone()).collect();
                let inputs = sqlx::query(
                    "SELECT DISTINCT ON (preview_id) preview_id, input FROM agent.step
                     WHERE tenant_id = $1 AND preview_id = ANY($2) ORDER BY preview_id, created_at DESC",
                )
                .bind(tenant)
                .bind(&ids)
                .fetch_all(&mut *tx)
                .await
                .map_err(db)?;
                let by_id: BTreeMap<String, Value> = inputs
                    .iter()
                    .filter_map(|r| {
                        Some((
                            r.try_get::<String, _>("preview_id").ok()?,
                            r.try_get::<Option<Value>, _>("input").ok()??,
                        ))
                    })
                    .collect();
                for i in &mut cs.items {
                    if let Some(v) = by_id.get(&i.preview_id) {
                        i.input = step_args(v);
                    }
                }
            }
            tx.commit().await.map_err(db)?;
            Ok(Some(ConversationContext {
                conversation_id: conversation_id.to_owned(),
                owner,
                agent: conv.try_get("agent_profile").map_err(db)?,
                turns,
                change_set,
            }))
        })
    }

    fn start_turn<'a>(&'a self, t: &'a TurnStart) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let mut tx = self.begin(&t.tenant).await?;
            sqlx::query(
                "INSERT INTO agent.conversation (tenant_id, id, surface, customer_id, admin_id,
                   session_id, name, agent_profile, started_at, last_turn_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $9)
                 ON CONFLICT (tenant_id, id) DO UPDATE SET agent_profile = EXCLUDED.agent_profile,
                   last_turn_at = EXCLUDED.last_turn_at, updated_at = now(),
                   version = agent.conversation.version + 1",
            )
            .bind(&t.tenant)
            .bind(&t.conversation_id)
            .bind(&t.surface)
            .bind(&t.owner.customer_id)
            .bind(&t.owner.admin_id)
            .bind(&t.owner.session_id)
            .bind(&t.name)
            .bind(&t.agent)
            .bind(t.started_at)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
            sqlx::query(
                "INSERT INTO agent.turn (tenant_id, id, conversation_id, position, text, context,
                   agent_profile, request_id, started_at)
                 SELECT $1, $2, $3, coalesce(max(position), 0) + 1, $4, $5, $6, $7, $8
                 FROM agent.turn WHERE tenant_id = $1 AND conversation_id = $3",
            )
            .bind(&t.tenant)
            .bind(&t.turn_id)
            .bind(&t.conversation_id)
            .bind(&t.text)
            .bind(&t.context)
            .bind(&t.agent)
            .bind(&t.request_id)
            .bind(t.started_at)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
            tx.commit().await.map_err(db)
        })
    }

    fn step<'a>(&'a self, s: &'a StepRow) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let mut tx = self.begin(&s.tenant).await?;
            sqlx::query(
                "INSERT INTO agent.step (tenant_id, id, turn_id, position, kind, instrument,
                   action_status, summary, input, output, preview_id, change_set_id, error,
                   started_at, ended_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
                 ON CONFLICT (tenant_id, id) DO UPDATE SET action_status = EXCLUDED.action_status,
                   summary = EXCLUDED.summary, input = EXCLUDED.input, output = EXCLUDED.output,
                   preview_id = EXCLUDED.preview_id, change_set_id = EXCLUDED.change_set_id,
                   error = EXCLUDED.error, ended_at = EXCLUDED.ended_at, updated_at = now(),
                   version = agent.step.version + 1",
            )
            .bind(&s.tenant)
            .bind(&s.step_id)
            .bind(&s.turn_id)
            .bind(s.position)
            .bind(&s.kind)
            .bind(&s.instrument)
            .bind(&s.status)
            .bind(&s.summary)
            .bind(&s.input)
            .bind(&s.output)
            .bind(&s.preview_id)
            .bind(&s.change_set_id)
            .bind(&s.error)
            .bind(s.started_at)
            .bind(s.ended_at)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
            tx.commit().await.map_err(db)
        })
    }

    fn model_call<'a>(
        &'a self,
        tenant: &'a str,
        turn_id: &'a str,
        position: i32,
        step_id: Option<&'a str>,
        r: &'a UsageRecord,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let mut tx = self.begin(tenant).await?;
            sqlx::query(
                "INSERT INTO agent.model_call (tenant_id, turn_id, position, step_id, model, purpose,
                   input_tokens, output_tokens, latency_ms, cost_value, cost_currency, succeeded,
                   error, started_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14)",
            )
            .bind(tenant)
            .bind(turn_id)
            .bind(position)
            .bind(step_id)
            .bind(&r.model)
            .bind(&r.purpose)
            .bind(clamp_i32(u64::from(r.input_tokens)))
            .bind(clamp_i32(u64::from(r.output_tokens)))
            .bind(clamp_i32(r.latency_ms))
            .bind(r.cost)
            .bind(&r.cost_currency)
            .bind(r.succeeded)
            .bind(r.error.as_ref().map(|e| json!({"code": "model_call_failed", "class": "upstream_unavailable", "message": e})))
            .bind(r.started_at)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
            tx.commit().await.map_err(db)
        })
    }

    fn finish_turn<'a>(&'a self, e: &'a TurnEnd) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let mut tx = self.begin(&e.tenant).await?;
            sqlx::query(
                "UPDATE agent.turn SET reply_text = $3, blocks = $4, usage = $5, error = $6,
                   ended_at = $7, agent_profile = $8, updated_at = now(), version = version + 1
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(&e.tenant)
            .bind(&e.turn_id)
            .bind(&e.reply_text)
            .bind(Value::Array(e.blocks.clone()))
            .bind(&e.usage)
            .bind(&e.error)
            .bind(e.ended_at)
            .bind(&e.agent)
            .execute(&mut *tx)
            .await
            .map_err(db)?;
            tx.commit().await.map_err(db)
        })
    }

    fn history<'a>(
        &'a self,
        tenant: &'a str,
        conversation_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<History>, String>> {
        Box::pin(async move {
            let mut tx = self.begin(tenant).await?;
            let Some(conv) = sqlx::query(
                "SELECT customer_id, admin_id, session_id FROM agent.conversation
                 WHERE tenant_id = $1 AND id = $2",
            )
            .bind(tenant)
            .bind(conversation_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?
            else {
                return Ok(None);
            };
            let owner = Owner {
                customer_id: conv.try_get("customer_id").map_err(db)?,
                admin_id: conv.try_get("admin_id").map_err(db)?,
                session_id: conv.try_get("session_id").map_err(db)?,
            };
            let rows = sqlx::query(
                "SELECT id, position, text, reply_text, agent_profile, blocks, usage, started_at
                 FROM agent.turn WHERE tenant_id = $1 AND conversation_id = $2 ORDER BY position",
            )
            .bind(tenant)
            .bind(conversation_id)
            .fetch_all(&mut *tx)
            .await
            .map_err(db)?;
            tx.commit().await.map_err(db)?;
            let turns = rows
                .iter()
                .map(|r| {
                    json!({
                        "turn_id": r.try_get::<String, _>("id").unwrap_or_default(),
                        "position": r.try_get::<i32, _>("position").unwrap_or_default(),
                        "text": r.try_get::<String, _>("text").unwrap_or_default(),
                        "reply_text": r.try_get::<Option<String>, _>("reply_text").ok().flatten(),
                        "agent": r.try_get::<String, _>("agent_profile").unwrap_or_default(),
                        "blocks": r.try_get::<Value, _>("blocks").unwrap_or(Value::Null),
                        "usage": r.try_get::<Value, _>("usage").unwrap_or(Value::Null),
                        "started_at": r.try_get::<DateTime<Utc>, _>("started_at").ok(),
                    })
                })
                .collect();
            Ok(Some((owner, turns)))
        })
    }

    fn turn_logs<'a>(
        &'a self,
        tenant: &'a str,
        conversation_id: &'a str,
    ) -> BoxFuture<'a, Result<Vec<(String, Value)>, String>> {
        Box::pin(self.logs(tenant, conversation_id))
    }

    fn conversations<'a>(
        &'a self,
        tenant: &'a str,
        surface: &'a str,
        owner: &'a Owner,
        limit: usize,
    ) -> BoxFuture<'a, Result<ConversationList, String>> {
        Box::pin(async move {
            let mut tx = self.begin(tenant).await?;
            // The same person as `Owner::same_person`: a member by customer id, an operator by
            // admin id, a guest by browser session (and then only what no member or operator owns).
            let rows = sqlx::query(
                "SELECT c.id, c.name, c.started_at, c.last_turn_at,
                   (SELECT count(*) FROM agent.turn t
                     WHERE t.tenant_id = c.tenant_id AND t.conversation_id = c.id) AS turns,
                   count(*) OVER () AS total
                 FROM agent.conversation c
                 WHERE c.tenant_id = $1 AND c.surface = $2
                   AND CASE WHEN $3::text IS NOT NULL THEN c.customer_id = $3
                            WHEN $4::text IS NOT NULL THEN c.admin_id = $4
                            ELSE c.customer_id IS NULL AND c.admin_id IS NULL
                                 AND c.session_id = $5::text END
                 ORDER BY coalesce(c.last_turn_at, c.started_at) DESC, c.id DESC
                 LIMIT $6",
            )
            .bind(tenant)
            .bind(surface)
            .bind(&owner.customer_id)
            .bind(&owner.admin_id)
            .bind(&owner.session_id)
            .bind(i64::try_from(limit).unwrap_or(i64::MAX))
            .fetch_all(&mut *tx)
            .await
            .map_err(db)?;
            tx.commit().await.map_err(db)?;
            let total = rows
                .first()
                .and_then(|r| r.try_get::<i64, _>("total").ok())
                .map_or(0, |n| usize::try_from(n).unwrap_or(0));
            let items = rows
                .iter()
                .map(|r| ConversationSummary {
                    conversation_id: r.try_get("id").unwrap_or_default(),
                    name: r.try_get("name").ok().flatten(),
                    turns: r
                        .try_get::<i64, _>("turns")
                        .map_or(0, |n| usize::try_from(n).unwrap_or(0)),
                    started_at: r.try_get("started_at").ok(),
                    last_turn_at: r.try_get("last_turn_at").ok().flatten(),
                })
                .collect();
            Ok((items, total))
        })
    }
}

impl PgStore {
    /// Steps and model calls of every turn of a conversation, grouped by turn.
    async fn logs(
        &self,
        tenant: &str,
        conversation_id: &str,
    ) -> Result<Vec<(String, Value)>, String> {
        let mut tx = self.begin(tenant).await?;
        let steps = sqlx::query(
            "SELECT s.turn_id, s.position, s.kind, s.instrument, s.action_status, s.summary,
                    s.input, s.output, s.error, s.started_at, s.ended_at
             FROM agent.step s JOIN agent.turn t ON t.tenant_id = s.tenant_id AND t.id = s.turn_id
             WHERE s.tenant_id = $1 AND t.conversation_id = $2 ORDER BY s.turn_id, s.position",
        )
        .bind(tenant)
        .bind(conversation_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(db)?;
        let calls = sqlx::query(
            "SELECT m.turn_id, m.position, m.model, m.purpose, m.input_tokens, m.output_tokens,
                    m.latency_ms, m.succeeded, m.error
             FROM agent.model_call m JOIN agent.turn t ON t.tenant_id = m.tenant_id AND t.id = m.turn_id
             WHERE m.tenant_id = $1 AND t.conversation_id = $2 ORDER BY m.turn_id, m.position",
        )
        .bind(tenant)
        .bind(conversation_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(db)?;
        tx.commit().await.map_err(db)?;
        let mut out: Vec<(String, Value)> = Vec::new();
        fn entry(out: &mut Vec<(String, Value)>, turn: String) -> usize {
            match out.iter().position(|(t, _)| *t == turn) {
                Some(i) => i,
                None => {
                    out.push((turn, json!({"steps": [], "model_calls": []})));
                    out.len() - 1
                }
            }
        }
        for r in &steps {
            let i = entry(&mut out, r.try_get("turn_id").map_err(db)?);
            if let Some(list) = out[i].1["steps"].as_array_mut() {
                list.push(json!({
                    "position": r.try_get::<i32, _>("position").unwrap_or_default(),
                    "kind": r.try_get::<String, _>("kind").unwrap_or_default(),
                    "tool": r.try_get::<Option<String>, _>("instrument").ok().flatten(),
                    "status": r.try_get::<String, _>("action_status").unwrap_or_default(),
                    "summary": r.try_get::<Option<String>, _>("summary").ok().flatten(),
                    "input": r.try_get::<Option<Value>, _>("input").ok().flatten(),
                    "output": r.try_get::<Option<Value>, _>("output").ok().flatten(),
                    "error": r.try_get::<Option<Value>, _>("error").ok().flatten(),
                }));
            }
        }
        for r in &calls {
            let i = entry(&mut out, r.try_get("turn_id").map_err(db)?);
            if let Some(list) = out[i].1["model_calls"].as_array_mut() {
                list.push(json!({
                    "position": r.try_get::<i32, _>("position").unwrap_or_default(),
                    "model": r.try_get::<String, _>("model").unwrap_or_default(),
                    "purpose": r.try_get::<String, _>("purpose").unwrap_or_default(),
                    "input_tokens": r.try_get::<i32, _>("input_tokens").unwrap_or_default(),
                    "output_tokens": r.try_get::<i32, _>("output_tokens").unwrap_or_default(),
                    "latency_ms": r.try_get::<i32, _>("latency_ms").unwrap_or_default(),
                    "succeeded": r.try_get::<bool, _>("succeeded").unwrap_or_default(),
                    "error": r.try_get::<Option<Value>, _>("error").ok().flatten(),
                }));
            }
        }
        Ok(out)
    }
}

/// The tool arguments a step recorded (`{"args": …, "repairs": […]}`).
fn step_args(input: &Value) -> Value {
    input.get("args").cloned().unwrap_or_else(|| input.clone())
}

/// The newest `change_set` block among the turns, as the conversation's open change set.
pub fn latest_change_set(turns: &[PastTurn]) -> Option<OpenChangeSet> {
    let data = turns
        .iter()
        .rev()
        .flat_map(|t| t.blocks.iter().rev())
        .find(|b| b.get("ui").and_then(Value::as_str) == Some("change_set"))?
        .get("data")?;
    let items = data
        .get("groups")
        .and_then(Value::as_array)
        .map(|g| {
            g.iter()
                .filter_map(|g| {
                    Some(OpenItem {
                        position: g.get("position")?.as_u64()? as usize,
                        tool: g.get("op")?.as_str()?.to_owned(),
                        preview_id: g.get("preview_id")?.as_str()?.to_owned(),
                        input: Value::Null,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Some(OpenChangeSet {
        id: data.get("change_set_id")?.as_str()?.to_owned(),
        number: data.get("number").and_then(Value::as_i64),
        version: data
            .get("version")
            .and_then(Value::as_i64)
            .and_then(|v| i32::try_from(v).ok())
            .unwrap_or(1),
        state: data
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("drafting")
            .to_owned(),
        items,
        last_view: data.clone(),
    })
}

/// An in-memory log for tests and for running without a database.
#[derive(Debug, Default)]
pub struct MemoryLog {
    inner: Mutex<MemoryInner>,
}

#[derive(Debug, Default)]
struct MemoryInner {
    conversations: BTreeMap<String, (Owner, Option<String>)>,
    turns: Vec<(TurnStart, Option<TurnEnd>)>,
    steps: BTreeMap<String, StepRow>,
    model_calls: Vec<UsageRecord>,
}

impl MemoryLog {
    pub fn steps(&self) -> Vec<StepRow> {
        self.inner
            .lock()
            .map(|i| {
                let mut s: Vec<StepRow> = i.steps.values().cloned().collect();
                s.sort_by_key(|r| (r.turn_id.clone(), r.position));
                s
            })
            .unwrap_or_default()
    }

    pub fn model_calls(&self) -> Vec<UsageRecord> {
        self.inner
            .lock()
            .map(|i| i.model_calls.clone())
            .unwrap_or_default()
    }

    pub fn ended(&self) -> Vec<TurnEnd> {
        self.inner
            .lock()
            .map(|i| i.turns.iter().filter_map(|(_, e)| e.clone()).collect())
            .unwrap_or_default()
    }
}

impl TurnLog for MemoryLog {
    fn load<'a>(
        &'a self,
        _tenant: &'a str,
        conversation_id: &'a str,
        recent: usize,
    ) -> BoxFuture<'a, Result<Option<ConversationContext>, String>> {
        Box::pin(async move {
            let inner = self.inner.lock().map_err(|e| e.to_string())?;
            let Some((owner, agent)) = inner.conversations.get(conversation_id).cloned() else {
                return Ok(None);
            };
            let all: Vec<PastTurn> = inner
                .turns
                .iter()
                .filter(|(s, _)| s.conversation_id == conversation_id)
                .map(|(s, e)| PastTurn {
                    text: s.text.clone(),
                    reply: e.as_ref().and_then(|e| e.reply_text.clone()),
                    agent: e
                        .as_ref()
                        .map_or_else(|| s.agent.clone(), |e| e.agent.clone()),
                    blocks: e.as_ref().map(|e| e.blocks.clone()).unwrap_or_default(),
                })
                .collect();
            let turns: Vec<PastTurn> = all[all.len().saturating_sub(recent.max(1))..].to_vec();
            let mut change_set = latest_change_set(&turns);
            if let Some(cs) = &mut change_set {
                for i in &mut cs.items {
                    if let Some(s) = inner
                        .steps
                        .values()
                        .filter(|s| s.preview_id.as_deref() == Some(i.preview_id.as_str()))
                        .max_by_key(|s| (s.turn_id.clone(), s.position))
                    {
                        i.input = s.input.as_ref().map_or(Value::Null, step_args);
                    }
                }
            }
            Ok(Some(ConversationContext {
                conversation_id: conversation_id.to_owned(),
                owner,
                agent,
                turns,
                change_set,
            }))
        })
    }

    fn start_turn<'a>(&'a self, t: &'a TurnStart) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let mut inner = self.inner.lock().map_err(|e| e.to_string())?;
            inner.conversations.insert(
                t.conversation_id.clone(),
                (t.owner.clone(), Some(t.agent.clone())),
            );
            inner.turns.push((t.clone(), None));
            Ok(())
        })
    }

    fn step<'a>(&'a self, s: &'a StepRow) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let mut inner = self.inner.lock().map_err(|e| e.to_string())?;
            inner.steps.insert(s.step_id.clone(), s.clone());
            Ok(())
        })
    }

    fn model_call<'a>(
        &'a self,
        _tenant: &'a str,
        _turn_id: &'a str,
        _position: i32,
        _step_id: Option<&'a str>,
        r: &'a UsageRecord,
    ) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let mut inner = self.inner.lock().map_err(|e| e.to_string())?;
            inner.model_calls.push(r.clone());
            Ok(())
        })
    }

    fn finish_turn<'a>(&'a self, e: &'a TurnEnd) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let mut inner = self.inner.lock().map_err(|e| e.to_string())?;
            if let Some((start, end)) = inner.turns.iter_mut().find(|(s, _)| s.turn_id == e.turn_id)
            {
                *end = Some(e.clone());
                let conv = start.conversation_id.clone();
                if let Some(c) = inner.conversations.get_mut(&conv) {
                    c.1 = Some(e.agent.clone());
                }
            }
            Ok(())
        })
    }

    fn history<'a>(
        &'a self,
        _tenant: &'a str,
        conversation_id: &'a str,
    ) -> BoxFuture<'a, Result<Option<History>, String>> {
        Box::pin(async move {
            let inner = self.inner.lock().map_err(|e| e.to_string())?;
            let Some((owner, _)) = inner.conversations.get(conversation_id).cloned() else {
                return Ok(None);
            };
            let turns = inner
                .turns
                .iter()
                .filter(|(s, _)| s.conversation_id == conversation_id)
                .map(|(s, e)| {
                    json!({"turn_id": s.turn_id, "text": s.text,
                           "reply_text": e.as_ref().and_then(|e| e.reply_text.clone()),
                           "blocks": e.as_ref().map(|e| e.blocks.clone()).unwrap_or_default()})
                })
                .collect();
            Ok(Some((owner, turns)))
        })
    }

    fn conversations<'a>(
        &'a self,
        _tenant: &'a str,
        surface: &'a str,
        owner: &'a Owner,
        limit: usize,
    ) -> BoxFuture<'a, Result<ConversationList, String>> {
        Box::pin(async move {
            let inner = self.inner.lock().map_err(|e| e.to_string())?;
            let mut list: Vec<ConversationSummary> = inner
                .conversations
                .iter()
                .filter(|(_, (o, _))| o.same_person(owner))
                .filter_map(|(id, _)| {
                    let turns: Vec<&TurnStart> = inner
                        .turns
                        .iter()
                        .map(|(s, _)| s)
                        .filter(|s| &s.conversation_id == id)
                        .collect();
                    let (first, last) = (turns.first()?, turns.last()?);
                    (first.surface == surface).then(|| ConversationSummary {
                        conversation_id: id.clone(),
                        name: Some(first.name.clone()),
                        turns: turns.len(),
                        started_at: Some(first.started_at),
                        last_turn_at: Some(last.started_at),
                    })
                })
                .collect();
            list.sort_by_key(|c| std::cmp::Reverse(c.last_turn_at));
            let total = list.len();
            list.truncate(limit);
            Ok((list, total))
        })
    }
}

/// Fixed settings, for tests.
#[derive(Debug, Clone)]
pub struct FixedStore(pub StoreSettings);

impl StoreInfo for FixedStore {
    fn settings<'a>(&'a self, _: &'a str) -> BoxFuture<'a, Result<StoreSettings, String>> {
        let s = self.0.clone();
        Box::pin(async move { Ok(s) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_latest_change_set_block_is_the_open_set() {
        let turns = vec![
            PastTurn {
                blocks: vec![
                    json!({"ui": "change_set", "data": {"change_set_id": "cs_1", "version": 1, "state": "drafting", "groups": []}}),
                ],
                ..Default::default()
            },
            PastTurn {
                blocks: vec![
                    json!({"ui": "change_set", "data": {"change_set_id": "cs_1042", "number": 1042, "version": 2,
                        "state": "waiting_for_approval",
                        "groups": [{"position": 1, "op": "set_sale_prices", "preview_id": "pv_9"}]}}),
                    json!({"ui": "product_table", "data": {}}),
                ],
                ..Default::default()
            },
        ];
        let cs = latest_change_set(&turns).unwrap();
        assert_eq!(
            (cs.id.as_str(), cs.version, cs.number),
            ("cs_1042", 2, Some(1042))
        );
        assert_eq!(cs.items[0].tool, "set_sale_prices");
        assert_eq!(cs.items[0].preview_id, "pv_9");
        assert!(latest_change_set(&[]).is_none());
    }

    fn started(
        conversation: &str,
        surface: &str,
        owner: &Owner,
        text: &str,
        at: &str,
    ) -> TurnStart {
        TurnStart {
            tenant: "store_northwind".into(),
            conversation_id: conversation.into(),
            surface: surface.into(),
            owner: owner.clone(),
            name: text.into(),
            turn_id: format!("turn_{}", acom_core::id::new_suffix()),
            text: text.into(),
            context: json!([]),
            agent: "promotion".into(),
            request_id: "req_test".into(),
            started_at: DateTime::parse_from_rfc3339(at)
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    #[tokio::test]
    async fn a_person_lists_their_own_conversations_on_one_surface_the_latest_first() {
        let log = MemoryLog::default();
        let sj = Owner {
            admin_id: Some("adm_sj".into()),
            ..Owner::default()
        };
        let mina = Owner {
            admin_id: Some("adm_mina".into()),
            ..Owner::default()
        };
        for t in [
            started(
                "conv_sale",
                "bo",
                &sj,
                "Run a VIP weekend sale",
                "2026-11-06T15:00:00Z",
            ),
            started(
                "conv_drop",
                "bo",
                &sj,
                "Why did sales drop?",
                "2026-11-06T16:00:00Z",
            ),
            started(
                "conv_sale",
                "bo",
                &sj,
                "Leave the overshirt out",
                "2026-11-06T17:00:00Z",
            ),
            started(
                "conv_shop",
                "fo",
                &sj,
                "A gift for my dad",
                "2026-11-06T18:00:00Z",
            ),
            started(
                "conv_mina",
                "bo",
                &mina,
                "Returns over $200",
                "2026-11-06T19:00:00Z",
            ),
        ] {
            log.start_turn(&t).await.unwrap();
        }

        let (items, total) = log
            .conversations("store_northwind", "bo", &sj, 20)
            .await
            .unwrap();
        assert_eq!(total, 2, "{items:#?}");
        // The sale had a turn after the drop question, so it comes first; it keeps its first request
        // as its name.
        assert_eq!(items[0].conversation_id, "conv_sale");
        assert_eq!(items[0].name.as_deref(), Some("Run a VIP weekend sale"));
        assert_eq!(items[0].turns, 2);
        assert_eq!(items[1].conversation_id, "conv_drop");

        let (items, total) = log
            .conversations("store_northwind", "bo", &sj, 1)
            .await
            .unwrap();
        assert_eq!((items.len(), total), (1, 2));

        let (items, _) = log
            .conversations("store_northwind", "bo", &mina, 20)
            .await
            .unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].conversation_id, "conv_mina");
    }

    #[tokio::test]
    async fn a_guest_lists_only_what_their_browser_session_started() {
        let log = MemoryLog::default();
        let guest = |s: &str| Owner {
            session_id: Some(s.into()),
            ..Owner::default()
        };
        log.start_turn(&started(
            "conv_g",
            "fo",
            &guest("ses_1"),
            "Light jackets",
            "2026-11-07T15:00:00Z",
        ))
        .await
        .unwrap();
        let count = |o: Owner| {
            let log = &log;
            async move {
                log.conversations("store_northwind", "fo", &o, 20)
                    .await
                    .unwrap()
                    .1
            }
        };
        assert_eq!(count(guest("ses_1")).await, 1);
        assert_eq!(count(guest("ses_2")).await, 0);
        let member = Owner {
            customer_id: Some("cus_jamie".into()),
            session_id: Some("ses_1".into()),
            ..Owner::default()
        };
        assert_eq!(
            count(member).await,
            0,
            "a member does not see a guest's conversations"
        );
        assert_eq!(
            count(Owner::default()).await,
            0,
            "no session, no conversations"
        );
    }

    /// Integration test: needs a migrated database (`DATABASE_URL`, role acom_app) with the store
    /// `store_northwind`. Run on request with `cargo test -p acom-agent -- --ignored`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn turns_steps_and_model_calls_round_trip() {
        let url = std::env::var("DATABASE_URL").expect("DATABASE_URL");
        let store = PgStore::new(Db::connect(&url, 2).await.expect("connect"));
        let tenant = "store_northwind";
        let settings = SystemStore.settings(tenant).await.expect("settings");
        let conv = format!("conv_{}", acom_core::id::new_suffix());
        let turn = format!("turn_{}", acom_core::id::new_suffix());
        let owner = Owner {
            admin_id: Some("adm_sj".into()),
            ..Owner::default()
        };
        store
            .start_turn(&TurnStart {
                tenant: tenant.into(),
                conversation_id: conv.clone(),
                surface: "bo".into(),
                owner: owner.clone(),
                name: "VIP weekend".into(),
                turn_id: turn.clone(),
                text: "Run a sale".into(),
                context: json!([]),
                agent: "promotion".into(),
                request_id: "req_test".into(),
                started_at: settings.now,
            })
            .await
            .expect("start");
        let step = StepRow {
            tenant: tenant.into(),
            step_id: format!("step_{}", acom_core::id::new_suffix()),
            turn_id: turn.clone(),
            position: 1,
            kind: "call_tool".into(),
            instrument: Some("set_sale_prices".into()),
            status: "completed".into(),
            summary: "preview".into(),
            input: Some(json!({"args": {"percent_off": 20}, "repairs": []})),
            output: None,
            preview_id: Some("pv_test".into()),
            change_set_id: Some("cs_test".into()),
            error: None,
            started_at: settings.now,
            ended_at: Some(settings.now),
        };
        store.step(&step).await.expect("step");
        store.step(&step).await.expect("step again (upsert)");
        store
            .finish_turn(&TurnEnd {
                tenant: tenant.into(),
                turn_id: turn.clone(),
                agent: "promotion".into(),
                reply_text: Some("Drafted.".into()),
                blocks: vec![json!({"ui": "change_set", "data": {"change_set_id": "cs_test", "version": 1,
                    "state": "drafting", "groups": [{"position": 1, "op": "set_sale_prices", "preview_id": "pv_test"}]}})],
                usage: json!({}),
                error: None,
                ended_at: settings.now,
            })
            .await
            .expect("finish");
        let ctx = store
            .load(tenant, &conv, 4)
            .await
            .expect("load")
            .expect("exists");
        assert!(ctx.owner.same_person(&owner));
        let cs = ctx.change_set.expect("open set");
        assert_eq!(cs.items[0].input, json!({"percent_off": 20}));
        let logs = store.turn_logs(tenant, &conv).await.expect("turn logs");
        assert_eq!(logs.len(), 1, "{logs:?}");
        let (_, log) = &logs[0];
        assert!(!log["steps"].as_array().unwrap().is_empty(), "{log:#}");
        assert!(log["model_calls"].is_array(), "{log:#}");
        // The newest conversation of the operator on the back office is this one, by its first
        // request; the storefront list does not have it.
        let (items, total) = store
            .conversations(tenant, "bo", &owner, 5)
            .await
            .expect("conversations");
        assert!(total >= 1);
        assert_eq!(items[0].conversation_id, conv, "{items:#?}");
        assert_eq!(items[0].name.as_deref(), Some("VIP weekend"));
        assert_eq!(items[0].turns, 1);
        let (fo, _) = store
            .conversations(tenant, "fo", &owner, 5)
            .await
            .expect("conversations");
        assert!(fo.iter().all(|c| c.conversation_id != conv));
    }
}
