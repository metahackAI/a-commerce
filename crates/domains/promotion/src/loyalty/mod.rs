//! The `loyalty` resource (docs/api/promotion.md#loyalty): store points as a ledger
//! (`promotion.loyalty_transaction`). Entries are only added; the balance is what the credits
//! (accrual, grant, restore) have left after the debits (spend, expire, reversal). Spending takes
//! the points that expire first, so the ledger also says what is left of each credit and when it
//! expires. What the operations share: the ledger rows, that arithmetic, and the expiry run
//! behind `expire_loyalty` and the daily `loyalty.expire` job.

pub(crate) mod expire_loyalty;
pub(crate) mod get_loyalty_balance;
pub(crate) mod grant_loyalty;
pub(crate) mod list_loyalty_transactions;
pub(crate) mod spend_loyalty;

use std::collections::BTreeMap;

use acom_contract::{ErrorClass, ErrorCode, ToolError};
pub(crate) use acom_core::enums::LoyaltyTransactionType;
use acom_core::{Currency, Money};
use acom_store::{Cx, TxCx};
use chrono::{DateTime, NaiveDate, Utc};
use rust_decimal::Decimal;
use serde_json::Value;

use crate::repo::db;

pub const OVER_LIMIT: ErrorCode = ErrorCode::new("over_limit", ErrorClass::LimitExceeded);
pub const INSUFFICIENT_BALANCE: ErrorCode =
    ErrorCode::new("insufficient_balance", ErrorClass::Domain);

/// The ledger's currency: the one the loyalty policy states its amounts in.
fn currency(cx: &Cx) -> Result<String, ToolError> {
    cx.policy
        .lookup("promotion.loyalty.spend.min_order.currency")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| {
            ToolError::new(
                ErrorCode::INTERNAL,
                "The policy value `promotion.loyalty.spend.min_order` is missing.",
            )
        })
}

/// One ledger entry.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub(crate) struct Entry {
    pub id: String,
    pub customer_id: String,
    pub transaction_type: String,
    pub amount_value: Decimal,
    pub amount_currency: String,
    pub order_id: Option<String>,
    pub source_transaction_id: Option<String>,
    pub expires: Option<NaiveDate>,
    pub description: Option<String>,
    pub start_time: DateTime<Utc>,
}

impl Entry {
    /// Accrual, grant and restore add points.
    pub(crate) fn is_credit(&self) -> bool {
        matches!(
            self.transaction_type.as_str(),
            "accrual" | "grant" | "restore"
        )
    }
}

const ENTRY_SELECT: &str = "SELECT id, customer_id, transaction_type, amount_value,
        amount_currency::text AS amount_currency, order_id, source_transaction_id, expires,
        description, start_time
     FROM promotion.loyalty_transaction WHERE tenant_id = $1";

/// A member's whole ledger, oldest first.
pub(crate) async fn ledger(tx: &mut TxCx, customer_id: &str) -> Result<Vec<Entry>, ToolError> {
    sqlx::query_as::<_, Entry>(sqlx::AssertSqlSafe(format!(
        "{ENTRY_SELECT} AND customer_id = $2 ORDER BY start_time, id"
    )))
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The ledgers of every member holding a credit that expired before `as_of` and has no expiry
/// entry yet, oldest first.
pub(crate) async fn ledgers_with_expired_credit(
    tx: &mut TxCx,
    as_of: NaiveDate,
) -> Result<Vec<Entry>, ToolError> {
    sqlx::query_as::<_, Entry>(sqlx::AssertSqlSafe(format!(
        "{ENTRY_SELECT} AND customer_id IN (
           SELECT c.customer_id FROM promotion.loyalty_transaction c
           WHERE c.tenant_id = $1 AND c.transaction_type IN ('accrual', 'grant', 'restore')
             AND c.expires < $2
             AND NOT EXISTS (SELECT 1 FROM promotion.loyalty_transaction x
                             WHERE x.tenant_id = c.tenant_id AND x.source_transaction_id = c.id
                               AND x.transaction_type = 'expire'))
         ORDER BY customer_id, start_time, id"
    )))
    .bind(tx.tenant.as_str())
    .bind(as_of)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// One entry, by id.
pub(crate) async fn entry(tx: &mut TxCx, id: &str) -> Result<Option<Entry>, ToolError> {
    sqlx::query_as::<_, Entry>(sqlx::AssertSqlSafe(format!("{ENTRY_SELECT} AND id = $2")))
        .bind(tx.tenant.as_str())
        .bind(id)
        .fetch_optional(&mut *tx.tx)
        .await
        .map_err(db)
}

/// A member's spend on an order, if any.
pub(crate) async fn spend_on(
    tx: &mut TxCx,
    customer_id: &str,
    order_id: &str,
) -> Result<Option<Entry>, ToolError> {
    sqlx::query_as::<_, Entry>(sqlx::AssertSqlSafe(format!(
        "{ENTRY_SELECT} AND customer_id = $2 AND order_id = $3 AND transaction_type = 'spend'"
    )))
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .bind(order_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The entry that expires or reverses `source`, if any.
pub(crate) async fn offset_of(
    tx: &mut TxCx,
    source: &str,
    kind: &str,
) -> Result<Option<Entry>, ToolError> {
    sqlx::query_as::<_, Entry>(sqlx::AssertSqlSafe(format!(
        "{ENTRY_SELECT} AND source_transaction_id = $2 AND transaction_type = $3"
    )))
    .bind(tx.tenant.as_str())
    .bind(source)
    .bind(kind)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A ledger entry to add.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct NewEntry {
    pub id: String,
    pub customer_id: String,
    pub transaction_type: LoyaltyTransactionType,
    pub amount: Money,
    pub order_id: Option<String>,
    pub source_transaction_id: Option<String>,
    pub expires: Option<NaiveDate>,
    pub description: Option<String>,
    pub agent_id: Option<String>,
}

/// Adds an entry; nothing happens when its id is already there.
pub(crate) async fn insert(tx: &mut TxCx, e: &NewEntry) -> Result<(), ToolError> {
    let now = tx.clock.now();
    sqlx::query(
        "INSERT INTO promotion.loyalty_transaction (tenant_id, id, customer_id, transaction_type,
           amount_value, amount_currency, order_id, source_transaction_id, expires, description,
           agent_id, start_time)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(&e.id)
    .bind(&e.customer_id)
    .bind(e.transaction_type.as_str())
    .bind(e.amount.value)
    .bind(e.amount.currency.as_str())
    .bind(&e.order_id)
    .bind(&e.source_transaction_id)
    .bind(e.expires)
    .bind(&e.description)
    .bind(&e.agent_id)
    .bind(now)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// What is left of one credit.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Left {
    pub id: String,
    pub remaining: Decimal,
    pub expires: Option<NaiveDate>,
}

/// What each credit has left after the debits, in ledger order. An expiry or reversal takes its
/// own credit; a spend takes the credits made before it that expire first.
pub(crate) fn remaining(entries: &[Entry]) -> Vec<Left> {
    let mut left: Vec<(Left, DateTime<Utc>)> = entries
        .iter()
        .filter(|e| e.is_credit())
        .map(|e| {
            (
                Left {
                    id: e.id.clone(),
                    remaining: e.amount_value,
                    expires: e.expires,
                },
                e.start_time,
            )
        })
        .collect();
    for e in entries.iter().filter(|e| !e.is_credit()) {
        if let Some(source) = &e.source_transaction_id
            && let Some((l, _)) = left.iter_mut().find(|(l, _)| &l.id == source)
        {
            let take = e.amount_value.min(l.remaining);
            l.remaining -= take;
            continue;
        }
        let mut due = e.amount_value;
        let mut order: Vec<usize> = (0..left.len())
            .filter(|i| left[*i].1 <= e.start_time && left[*i].0.remaining > Decimal::ZERO)
            .collect();
        order.sort_by_key(|i| (left[*i].0.expires.unwrap_or(NaiveDate::MAX), left[*i].1));
        for i in order {
            if due <= Decimal::ZERO {
                break;
            }
            let take = due.min(left[i].0.remaining);
            left[i].0.remaining -= take;
            due -= take;
        }
    }
    left.into_iter().map(|(l, _)| l).collect()
}

/// The points that can be spent on `today`: what is left of credits not yet expired.
pub(crate) fn balance(entries: &[Entry], today: NaiveDate) -> Decimal {
    remaining(entries)
        .iter()
        .filter(|l| l.expires.is_none_or(|d| d >= today))
        .map(|l| l.remaining)
        .sum()
}

/// What expires within `days` of `today` (both included), by day.
pub(crate) fn expiring(
    entries: &[Entry],
    today: NaiveDate,
    days: i64,
) -> BTreeMap<NaiveDate, Decimal> {
    let until = today + chrono::Duration::days(days);
    let mut out = BTreeMap::new();
    for l in remaining(entries) {
        if let Some(d) = l.expires
            && d >= today
            && d <= until
            && l.remaining > Decimal::ZERO
        {
            *out.entry(d).or_insert(Decimal::ZERO) += l.remaining;
        }
    }
    out
}

/// The credits that expired before `as_of` with points left and no expiry entry yet:
/// `(customer, credit, points left, currency)`.
pub(crate) fn to_expire(
    entries: &[Entry],
    as_of: NaiveDate,
) -> Vec<(String, String, Decimal, String)> {
    let mut out = Vec::new();
    let mut by_customer: BTreeMap<&str, Vec<Entry>> = BTreeMap::new();
    for e in entries {
        by_customer
            .entry(e.customer_id.as_str())
            .or_default()
            .push(e.clone());
    }
    for (customer, list) in by_customer {
        let expired: Vec<&String> = list
            .iter()
            .filter(|e| e.transaction_type == "expire")
            .filter_map(|e| e.source_transaction_id.as_ref())
            .collect();
        for l in remaining(&list) {
            if l.expires.is_some_and(|d| d < as_of)
                && l.remaining > Decimal::ZERO
                && !expired.contains(&&l.id)
                && let Some(c) = list.iter().find(|e| e.id == l.id)
            {
                out.push((
                    customer.to_owned(),
                    l.id.clone(),
                    l.remaining,
                    c.amount_currency.trim().to_owned(),
                ));
            }
        }
    }
    out
}

/// Money from a stored amount.
pub(crate) fn stored_money(value: Decimal, currency: &str) -> Result<Money, ToolError> {
    Currency::parse(currency.trim())
        .map(|c| Money::new(value, c))
        .map_err(|e| ToolError::new(ErrorCode::INTERNAL, e.to_string()))
}

/// The store date of `at`.
pub(crate) fn store_date(cx_time: &acom_core::StoreTime, at: DateTime<Utc>) -> NaiveDate {
    at.with_timezone(&cx_time.zone).date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::d;

    fn e(
        id: &str,
        kind: &str,
        amount: &str,
        at: &str,
        expires: Option<&str>,
        source: Option<&str>,
    ) -> Entry {
        Entry {
            id: id.into(),
            customer_id: "cus_jamie".into(),
            transaction_type: kind.into(),
            amount_value: d(amount),
            amount_currency: "USD".into(),
            order_id: None,
            source_transaction_id: source.map(str::to_owned),
            expires: expires.map(|s| s.parse().unwrap()),
            description: None,
            start_time: DateTime::parse_from_rfc3339(at)
                .unwrap()
                .with_timezone(&Utc),
        }
    }

    fn date(s: &str) -> NaiveDate {
        s.parse().unwrap()
    }

    #[test]
    fn spends_take_the_points_that_expire_first() {
        let ledger = vec![
            e(
                "lyt_welcome",
                "grant",
                "5.00",
                "2025-11-14T10:00:00-05:00",
                Some("2026-11-14"),
                None,
            ),
            e(
                "lyt_a",
                "accrual",
                "3.12",
                "2026-01-23T10:00:00-05:00",
                Some("2027-01-23"),
                None,
            ),
            e(
                "lyt_spend",
                "spend",
                "3.00",
                "2026-03-09T20:41:00-04:00",
                None,
                None,
            ),
        ];
        let left = remaining(&ledger);
        assert_eq!(left[0].remaining, d("2.00"));
        assert_eq!(left[1].remaining, d("3.12"));
        assert_eq!(balance(&ledger, date("2026-11-06")), d("5.12"));
        assert_eq!(
            expiring(&ledger, date("2026-11-06"), 10)
                .into_iter()
                .collect::<Vec<_>>(),
            vec![(date("2026-11-14"), d("2.00"))]
        );
        // After its day the welcome grant no longer counts, and the expiry run takes it.
        assert_eq!(balance(&ledger, date("2026-11-15")), d("3.12"));
        let due = to_expire(&ledger, date("2026-11-15"));
        assert_eq!(
            due,
            vec![(
                "cus_jamie".into(),
                "lyt_welcome".into(),
                d("2.00"),
                "USD".into()
            )]
        );
        let mut done = ledger.clone();
        done.push(e(
            "lyt_x",
            "expire",
            "2.00",
            "2026-11-15T00:00:00-05:00",
            None,
            Some("lyt_welcome"),
        ));
        assert!(to_expire(&done, date("2026-11-16")).is_empty());
        assert_eq!(balance(&done, date("2026-11-15")), d("3.12"));
    }

    #[test]
    fn a_reversal_takes_its_own_grant() {
        let ledger = vec![
            e(
                "lyt_a",
                "accrual",
                "1.00",
                "2026-01-01T10:00:00-05:00",
                Some("2026-12-01"),
                None,
            ),
            e(
                "lyt_g",
                "grant",
                "5.00",
                "2026-02-01T10:00:00-05:00",
                Some("2027-02-01"),
                None,
            ),
            e(
                "lyt_r",
                "reversal",
                "5.00",
                "2026-02-02T10:00:00-05:00",
                None,
                Some("lyt_g"),
            ),
        ];
        assert_eq!(balance(&ledger, date("2026-03-01")), d("1.00"));
    }
}
