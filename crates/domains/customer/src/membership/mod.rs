//! `list_tiers`, `get_tier`, `get_member_tier`. Tiers are data the store defines
//! (`customer.member_program_tier`), never code.

use acom_contract::ToolError;
use acom_store::TxCx;
use serde_json::{Value, json};

use crate::progress::{cents, threshold_money};
use crate::{TIER_NOT_FOUND, repo};

mod get_member_tier;
mod get_tier;
mod list_tiers;

pub use get_member_tier::*;
pub use get_tier::*;
pub use list_tiers::*;

fn tier_not_found(id: &str) -> ToolError {
    ToolError::new(TIER_NOT_FOUND, format!("There is no tier {id}."))
        .detail(json!({"entity": "tier", "id": id}))
        .next("list_tiers")
}

/// `{value, unit_text}` of a tier's points multiplier, or null.
fn points_json(t: &repo::TierRow) -> Value {
    match (
        &t.membership_points_earned_value,
        &t.membership_points_earned_unit_text,
    ) {
        (Some(v), unit) => json!({"value": v, "unit_text": unit}),
        _ => Value::Null,
    }
}

async fn member_count(tx: &mut TxCx, program: &str, tier: &str) -> Result<i64, ToolError> {
    Ok(repo::member_counts(tx, Some(program))
        .await?
        .into_iter()
        .find(|(t, _)| t == tier)
        .map_or(0, |(_, n)| n))
}

/// `Spend at least 300.00 USD in 6 months`, one line per requirement.
pub(crate) fn requirement_text(req: &Value) -> String {
    let metric = req
        .get("metric")
        .and_then(Value::as_str)
        .unwrap_or("custom");
    let window = req.get("window").map(|w| {
        let length = w.get("length").and_then(Value::as_u64).unwrap_or(0);
        let unit = w.get("unit").and_then(Value::as_str).unwrap_or("month");
        match w.get("type").and_then(Value::as_str) {
            Some("lifetime") => " in total".to_owned(),
            _ => format!(" in {length} {unit}{}", if length == 1 { "" } else { "s" }),
        }
    });
    let window = window.unwrap_or_default();
    match metric {
        "spend" => match threshold_money(req, "threshold") {
            Some(m) => {
                let m = cents(m.value, m.currency);
                format!("Spend at least {} {}{window}", m.value, m.currency)
            }
            None => format!("Spend{window}"),
        },
        "order_count" => format!(
            "At least {} delivered orders{window}",
            req.get("threshold").and_then(Value::as_i64).unwrap_or(0)
        ),
        "invite" => "Invited".to_owned(),
        "manual" => "Set by an operator".to_owned(),
        other => other.replace('_', " "),
    }
}

#[cfg(test)]
mod tests {
    use acom_contract::ReadOp;

    use super::*;

    #[test]
    fn describes_requirements_in_a_line() {
        let gold = json!({"metric": "spend", "threshold": {"value": 300.00, "currency": "USD"},
                          "window": {"type": "rolling", "length": 6, "unit": "month"}});
        assert_eq!(
            requirement_text(&gold),
            "Spend at least 300.00 USD in 6 months"
        );
        assert_eq!(requirement_text(&json!({"metric": "invite"})), "Invited");
    }

    /// Integration: needs a seeded database (`DATABASE_URL`). Run with `-- --ignored`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn reads_tiers_and_memberships() {
        let mut tx = crate::testdb::tx("adm_sj").await;
        let tiers = repo::tiers(&mut tx, Some("prg_rewards")).await.unwrap();
        assert_eq!(
            tiers.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
            ["vip", "gold", "member"]
        );
        let alex = repo::memberships(&mut tx, "cus_alex").await.unwrap();
        assert_eq!(alex[0].tier_id, "member");
        let cx = crate::testdb::cx("cus_alex").await;
        let out = GetMemberTier
            .read(
                &cx,
                GetMemberTierInput {
                    customer_id: "me".into(),
                    program_id: None,
                },
            )
            .await
            .unwrap();
        assert_eq!(out["tier_id"], "member");
        assert_eq!(out["source"], "entry");
    }
}
