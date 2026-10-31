//! `expire_loyalty` (docs/api/promotion.md#loyalty): records the store points that passed their
//! last day as `expire` entries. A batch for the system's scheduled job, never for the agent:
//! the daily `loyalty.expire` job ([`LoyaltyExpires`]) runs the same expiry. Running it again for
//! the same day skips what is already expired.

use acom_contract::{
    ActionType, ApprovalNeed, Audience, Change, ChangeOp, Committed, DomainEvent, ErrorCode, Group,
    Job, JobMeta, JobOutcome, Kind, Method, OpMeta, Operation, Planned, Slot, ToolError,
    UndoSupport, Verification, WriteOp,
};
use acom_core::enums::ActorKind;
use acom_store::{Cx, TxCx};
use chrono::{Days, NaiveDate};
use rust_decimal::Decimal;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{
    LoyaltyTransactionType, NewEntry, insert, ledgers_with_expired_credit, offset_of, store_date,
    stored_money, to_expire,
};
use crate::repo::{self, db};

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExpireLoyaltyInput {
    /// The day the run is for (`YYYY-MM-DD`, store time): points whose last day is before it
    /// expire. Default: today.
    #[schemars(regex(pattern = r"^\d{4}-\d{2}-\d{2}$"), example = &"2026-11-07")]
    pub as_of: Option<String>,
}

/// What a run of `expire_loyalty` writes: one `expire` entry per credit.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExpireRun {
    pub as_of: NaiveDate,
    pub entries: Vec<NewEntry>,
}

pub struct ExpireLoyalty;

impl Operation for ExpireLoyalty {
    const META: OpMeta = OpMeta {
        address: "promotion.loyalty.expire",
        tool: "expire_loyalty",
        group: Group::Engage,
        kind: Kind::Write,
        method: Method::Post,
        path: "/v1/promotion/loyalty/expirations",
        summary: "System only: record the store points that passed their last day as expired, for one day. The daily scheduled job runs it; running it again skips what already expired.",
        consumes: &[],
        produces: &[Slot::many("loyalty_transaction_id")],
        checks: &["loyalty.expiry"],
        errors: &[],
        audience: Audience::Internal,
        action_type: ActionType::UPDATE,
        person_scope: "",
    };
    type Input = ExpireLoyaltyInput;
    type Output = Value;
}

/// The expiry entries due for `as_of`.
async fn plan(tx: &mut TxCx, as_of: NaiveDate) -> Result<ExpireRun, ToolError> {
    let entries = ledgers_with_expired_credit(tx, as_of).await?;
    let mut out = Vec::new();
    for (customer, credit, left, currency) in to_expire(&entries, as_of) {
        out.push(NewEntry {
            id: repo::new_id("lyt"),
            customer_id: customer,
            transaction_type: LoyaltyTransactionType::Expire,
            amount: stored_money(left, &currency)?,
            order_id: None,
            source_transaction_id: Some(credit.clone()),
            expires: None,
            description: Some(format!("Expired unused from {credit}")),
            agent_id: Some(tx.principal.subject.clone()),
        });
    }
    Ok(ExpireRun {
        as_of,
        entries: out,
    })
}

/// Adds the planned entries that are still due. Returns the ids added.
async fn run(tx: &mut TxCx, plan: &ExpireRun) -> Result<Vec<String>, ToolError> {
    let mut done = Vec::new();
    for e in &plan.entries {
        let source = e.source_transaction_id.as_deref().unwrap_or_default();
        if offset_of(tx, source, "expire").await?.is_some() {
            continue;
        }
        insert(tx, e).await?;
        done.push(e.id.clone());
    }
    Ok(done)
}

fn run_id(as_of: NaiveDate) -> String {
    format!("job_expire_{}", as_of.format("%Y%m%d"))
}

impl WriteOp<Cx> for ExpireLoyalty {
    type Plan = ExpireRun;
    const UNDO: UndoSupport = UndoSupport::NotSupported;

    async fn preview(
        &self,
        cx: &Cx,
        input: ExpireLoyaltyInput,
    ) -> Result<Planned<ExpireRun>, ToolError> {
        if cx.principal.kind != ActorKind::System {
            return Err(ToolError::new(
                ErrorCode::FORBIDDEN,
                "Store points are expired by the system's daily job, not by a person or the agent.",
            )
            .detail(json!({"reason": "role"})));
        }
        let as_of = match &input.as_of {
            None => store_date(&cx.time, cx.clock.now()),
            Some(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| {
                ToolError::invalid_input(format!(
                    "`as_of` must be a date like 2026-11-07; got `{s}`."
                ))
                .detail(json!({"field": "as_of", "reason": "format"}))
            })?,
        };
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let p = plan(&mut tx, as_of).await?;
        tx.commit().await.map_err(ToolError::from)?;
        let total: Decimal = p.entries.iter().map(|e| e.amount.value).sum();
        let changes = if p.entries.is_empty() {
            Vec::new()
        } else {
            vec![Change {
                op: ChangeOp::Create,
                entity: "loyalty_transaction".into(),
                id: String::new(),
                from: Value::Null,
                to: json!({"type": "expire", "entries": p.entries.len(), "points": crate::amount(total)}),
                note: Some(format!("Points whose last day was before {as_of}")),
            }]
        };
        Ok(Planned {
            plan: p,
            changes,
            checks: Vec::new(),
            basis: Vec::new(),
            approval: ApprovalNeed::NotNeeded,
        })
    }

    async fn commit(&self, tx: &mut TxCx, p: &ExpireRun) -> Result<Committed, ToolError> {
        let done = run(tx, p).await?;
        Ok(Committed {
            id: run_id(p.as_of),
            produced: serde_json::Map::from_iter([
                ("expired".to_owned(), json!(done.len())),
                ("loyalty_transaction_id".to_owned(), json!(done)),
            ]),
        })
    }

    async fn verify(
        &self,
        cx: &Cx,
        p: &ExpireRun,
        _: &Committed,
    ) -> Result<Verification, ToolError> {
        let mut tx = cx.read().await.map_err(ToolError::from)?;
        let left = plan(&mut tx, p.as_of).await?;
        tx.commit().await.map_err(ToolError::from)?;
        Ok(Verification {
            matched: left.entries.is_empty(),
            mismatches: left
                .entries
                .iter()
                .map(|e| {
                    format!(
                        "{} is still not expired",
                        e.source_transaction_id.as_deref().unwrap_or("")
                    )
                })
                .collect(),
        })
    }

    async fn undo(&self, _: &mut TxCx, _: &ExpireRun, _: &Committed) -> Result<(), ToolError> {
        Ok(())
    }
}

/// `loyalty.expire`: the daily run of the store-point expiry, at store midnight. The job queue
/// schedules only watchers by themselves, so each run books the next day's run.
#[derive(Debug)]
pub struct LoyaltyExpires;

impl Job<Cx> for LoyaltyExpires {
    const META: JobMeta = JobMeta {
        kind: "loyalty.expire",
        summary: "Expires store points past their last day (promotion.yml loyalty.expiry), daily at store midnight.",
        watcher: None,
    };

    async fn run(&self, tx: &mut TxCx, _: &Value) -> Result<JobOutcome, ToolError> {
        let now = tx.clock.now();
        let as_of = store_date(&tx.time, now);
        let p = plan(tx, as_of).await?;
        let done = run(tx, &p).await?;
        let next_day = as_of + Days::new(1);
        let next_at = tx
            .time
            .parse(&format!("{next_day}T00:00"))
            .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))?;
        sqlx::query(
            "INSERT INTO platform.job (tenant_id, id, kind, run_at, payload)
             VALUES ($1, $2, $3, $4, '{}') ON CONFLICT (tenant_id, id) DO NOTHING",
        )
        .bind(tx.tenant.as_str())
        .bind(format!("job_loyalty_expire_{}", next_day.format("%Y%m%d")))
        .bind(Self::META.kind)
        .bind(next_at)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        let events = if done.is_empty() {
            Vec::new()
        } else {
            vec![DomainEvent {
                topic: "loyalty.expired".into(),
                object_id: run_id(as_of),
                payload: json!({"as_of": as_of.to_string(), "expired": done.len()}),
            }]
        };
        Ok(JobOutcome::Done {
            result: json!({"as_of": as_of.to_string(), "expired": done.len(),
                           "next_run_at": tx.time.format(next_at)}),
            events,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use acom_contract::schema::{closed_schema_for, violations};

    #[test]
    fn is_a_system_batch_with_a_daily_job() {
        assert_eq!(ExpireLoyalty::META.audience, Audience::Internal);
        assert_eq!(LoyaltyExpires::META.kind, "loyalty.expire");
        let s = closed_schema_for::<ExpireLoyaltyInput>();
        assert_eq!(violations(&s), Vec::<String>::new(), "{s:#}");
        assert_eq!(run_id("2026-11-07".parse().unwrap()), "job_expire_20261107");
    }
}
