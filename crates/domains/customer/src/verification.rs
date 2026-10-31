//! The identity check engine (`customer.identity_verification`): replaceable methods from the
//! policy (`identity_verification.methods`), allowed per purpose (`required_for`). The tool
//! `verify_identity` starts and finishes checks; sign-up, contact changes, business checks,
//! password resets and withdrawal re-authentication consume them.
//!
//! The one-time code is a secret: it is made at commit, stored only as a hash, and handed to the
//! delivery stub (email and SMS are not connected here, so it is recorded as not delivered). The
//! `verification_token` a finished check hands out is the check's own id (`ivf_…`); it is not a
//! secret by itself, because it only works once, before it expires, for the purpose and the
//! destination that were verified.

use acom_contract::ToolError;
use acom_policy::PolicySet;
use acom_store::TxCx;
use chrono::{DateTime, Duration, Utc};
use serde_json::{Value, json};

use crate::access::db;
use crate::secret;
use crate::{IDENTITY_NOT_VERIFIED, METHOD_NOT_ENABLED};

/// One method of `identity_verification.methods`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Method {
    pub id: String,
    /// `one_time_code`, `magic_link`, `registry_lookup`, `external_provider`.
    pub kind: String,
    /// `email` or `sms` for codes and links.
    pub channel: Option<String>,
}

impl Method {
    pub(crate) fn sends_code(&self) -> bool {
        matches!(self.kind.as_str(), "one_time_code" | "magic_link")
    }
}

/// The methods allowed for a purpose, in the policy's order (the first is the default).
pub(crate) fn allowed(policy: &PolicySet, purpose: &str) -> Vec<String> {
    policy
        .lookup(&format!(
            "customer.identity_verification.required_for.{purpose}"
        ))
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The method `id`, when the store turned it on for `purpose`; else `method_not_enabled`.
pub(crate) fn method(policy: &PolicySet, purpose: &str, id: &str) -> Result<Method, ToolError> {
    let allowed = allowed(policy, purpose);
    let found = policy
        .lookup("customer.identity_verification.methods")
        .and_then(Value::as_array)
        .and_then(|l| {
            l.iter()
                .find(|m| m.get("id").and_then(Value::as_str) == Some(id))
        });
    match found {
        Some(m) if allowed.iter().any(|a| a == id) => Ok(Method {
            id: id.to_owned(),
            kind: m
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("one_time_code")
                .to_owned(),
            channel: m.get("channel").and_then(Value::as_str).map(str::to_owned),
        }),
        _ => Err(ToolError::new(
            METHOD_NOT_ENABLED,
            format!("{id} is not a way this store checks identity for {purpose}."),
        )
        .detail(
            json!({"field": "method", "reason": "enum", "allowed": allowed,
                       "policy_key": format!("identity_verification.required_for.{purpose}")}),
        )
        .next("get_policy")),
    }
}

#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub(crate) struct CheckRow {
    pub id: String,
    pub purpose: String,
    pub method: String,
    pub destination: Option<String>,
    pub customer_id: Option<String>,
    pub subject: Option<Value>,
    pub code_hash: Option<String>,
    pub attempts: i32,
    pub action_status: String,
    pub requested_at: DateTime<Utc>,
    pub expires: DateTime<Utc>,
    pub verified_at: Option<DateTime<Utc>>,
    pub token_expires: Option<DateTime<Utc>>,
    pub token_used_at: Option<DateTime<Utc>>,
    pub version: i32,
}

const COLUMNS: &str = "id, purpose, method, destination, customer_id, subject, code_hash,
    attempts, action_status, requested_at, expires, verified_at, token_expires, token_used_at,
    version";

pub(crate) async fn check(tx: &mut TxCx, id: &str) -> Result<Option<CheckRow>, ToolError> {
    sqlx::query_as::<_, CheckRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM customer.identity_verification WHERE tenant_id = $1 AND id = $2"
    )))
    .bind(tx.tenant.as_str())
    .bind(id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// The newest check waiting for its code for this purpose, method and destination.
pub(crate) async fn waiting(
    tx: &mut TxCx,
    purpose: &str,
    method: &str,
    destination: &str,
) -> Result<Option<CheckRow>, ToolError> {
    sqlx::query_as::<_, CheckRow>(sqlx::AssertSqlSafe(format!(
        "SELECT {COLUMNS} FROM customer.identity_verification
         WHERE tenant_id = $1 AND purpose = $2 AND method = $3 AND destination = $4
           AND action_status = 'potential'
         ORDER BY requested_at DESC, id DESC LIMIT 1"
    )))
    .bind(tx.tenant.as_str())
    .bind(purpose)
    .bind(method)
    .bind(destination)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Codes sent to a destination since a time, and when the last one went.
pub(crate) async fn sends_since(
    tx: &mut TxCx,
    destination: &str,
    since: DateTime<Utc>,
) -> Result<(i64, Option<DateTime<Utc>>), ToolError> {
    sqlx::query_as(
        "SELECT count(*), max(requested_at) FROM customer.identity_verification
         WHERE tenant_id = $1 AND destination = $2 AND requested_at >= $3",
    )
    .bind(tx.tenant.as_str())
    .bind(destination)
    .bind(since)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// A new check to save.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub(crate) struct NewCheck {
    pub id: String,
    pub purpose: String,
    pub method: String,
    pub destination: Option<String>,
    pub customer_id: Option<String>,
    pub subject: Option<Value>,
    /// How long the code or link lives.
    pub ttl_seconds: i64,
    /// A registry lookup finishes at once; a token lives this long.
    pub verified_token_ttl_seconds: Option<i64>,
    /// The code length, for methods that send one.
    pub code_length: Option<usize>,
}

/// Saves a check. For code and link methods it makes the code (returned for the delivery stub
/// only, never stored or logged); a registry lookup is saved already verified.
pub(crate) async fn insert(tx: &mut TxCx, c: &NewCheck) -> Result<Option<String>, ToolError> {
    let now = tx.clock.now();
    let code = c.code_length.map(secret::code);
    let verified = c.verified_token_ttl_seconds.is_some();
    sqlx::query(
        "INSERT INTO customer.identity_verification (tenant_id, id, purpose, method, destination,
           customer_id, subject, code_hash, action_status, requested_at, expires, verified_at,
           token_expires)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
         ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(&c.id)
    .bind(&c.purpose)
    .bind(&c.method)
    .bind(c.destination.as_deref())
    .bind(c.customer_id.as_deref())
    .bind(c.subject.as_ref())
    .bind(code.as_deref().map(secret::hash))
    .bind(if verified { "completed" } else { "potential" })
    .bind(now)
    .bind(now + Duration::seconds(c.ttl_seconds))
    .bind(verified.then_some(now))
    .bind(
        c.verified_token_ttl_seconds
            .map(|s| now + Duration::seconds(s)),
    )
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(code)
}

/// Marks a check verified and starts its token's lifetime.
pub(crate) async fn mark_verified(
    tx: &mut TxCx,
    id: &str,
    token_ttl: Duration,
) -> Result<(), ToolError> {
    let now = tx.clock.now();
    sqlx::query(
        "UPDATE customer.identity_verification SET action_status = 'completed', verified_at = $3,
           token_expires = $4, updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND action_status = 'potential'",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(now)
    .bind(now + token_ttl)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// Counts one wrong code; at `max` the check fails. Returns the attempts so far.
pub(crate) async fn record_wrong_code(tx: &mut TxCx, id: &str, max: i32) -> Result<i32, ToolError> {
    sqlx::query_scalar(
        "UPDATE customer.identity_verification SET attempts = attempts + 1,
           action_status = CASE WHEN attempts + 1 >= $3 THEN 'failed' ELSE action_status END,
           updated_at = now(), version = version + 1
         WHERE tenant_id = $1 AND id = $2 RETURNING attempts",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(max)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Marks a token used (a token works once). No-op when it already is.
pub(crate) async fn use_token(tx: &mut TxCx, id: &str) -> Result<(), ToolError> {
    sqlx::query(
        "UPDATE customer.identity_verification SET token_used_at = $3, updated_at = now(),
           version = version + 1
         WHERE tenant_id = $1 AND id = $2 AND token_used_at IS NULL",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(tx.clock.now())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// The finished, unused, unexpired check a `verification_token` names, for this purpose and (when
/// given) this destination; else `identity_not_verified` naming why.
pub(crate) async fn usable(
    tx: &mut TxCx,
    token: &str,
    purpose: &str,
    destination: Option<&str>,
    fix: &'static str,
) -> Result<CheckRow, ToolError> {
    let refuse = |why: &str| {
        ToolError::new(
            IDENTITY_NOT_VERIFIED,
            format!("The verification_token cannot be used: {why}. Verify again."),
        )
        .detail(json!({"field": "verification_token", "reason": why, "purpose": purpose}))
        .next(fix)
    };
    let now = tx.clock.now();
    let Some(c) = check(tx, token).await? else {
        return Err(refuse("unknown token"));
    };
    if c.purpose != purpose {
        return Err(refuse("made for another purpose"));
    }
    if c.action_status != "completed" {
        return Err(refuse("the check did not succeed"));
    }
    if c.token_used_at.is_some() {
        return Err(refuse("already used"));
    }
    if c.token_expires.is_none_or(|t| t <= now) {
        return Err(refuse("expired"));
    }
    if let Some(d) = destination
        && c.destination.as_deref().map(secret::normalize) != Some(secret::normalize(d))
    {
        return Err(refuse("made for another email or phone"));
    }
    Ok(c)
}

/// `a***@example.com` or `+1 *** *** 0142`: how a destination is shown.
pub(crate) fn masked(destination: &str) -> String {
    if destination.contains('@') {
        crate::wording::mask_email(destination)
    } else {
        let digits: String = destination.chars().filter(char::is_ascii_digit).collect();
        let tail: String = digits
            .chars()
            .rev()
            .take(4)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();
        format!("*** {tail}")
    }
}

/// What a delivery stub says: nothing left the store.
pub(crate) fn not_delivered(channel: &str, to: &str) -> Value {
    json!({"channel": channel, "to": masked(to), "delivered": false,
           "note": "Email and SMS are not connected here: the message was recorded, not delivered."})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_destinations() {
        assert_eq!(masked("alex.kim@example.com"), "a***@example.com");
        assert_eq!(masked("+1 718 555 0142"), "*** 0142");
    }
}
