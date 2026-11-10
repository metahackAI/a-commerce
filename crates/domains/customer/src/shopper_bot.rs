//! What Ailism Customer Bot needs of the members (docs/api/platform.md#customer_bot): which
//! members a run shops as, and a new member when a session signs up. Called by the commerce
//! service's internal endpoints, only for the system and only in a demo store; never tools.
//!
//! A new account here is what `register_member` makes (the person, the entry tier, the mandatory
//! consents) plus the home address and card a shopper adds after joining. The identity check is
//! the one step it skips: the bot has no inbox to read a code from, and a demo store's members are
//! made up.

use acom_contract::{ErrorCode, ToolError};
use acom_core::enums::TierChangeCause;
use acom_store::{StoreError, TxCx};
use serde_json::{Value, json};

use crate::person::{entry_tiers, identifier_taken, membership_number};
use crate::{consent, repo, secret};

fn db(e: sqlx::Error) -> ToolError {
    ToolError::from(StoreError::from(e))
}

fn invalid(field: &str, message: &str) -> ToolError {
    ToolError::new(ErrorCode::INVALID_INPUT, message.to_owned())
        .detail(json!({"field": field, "reason": "invalid"}))
}

/// Up to `n` active individual members with an address to ship to, drawn at random, leaving out
/// `never` (the scenes' cast and members already shopping).
pub async fn pick_members(
    tx: &mut TxCx,
    n: usize,
    never: &[String],
) -> Result<Vec<String>, ToolError> {
    if n == 0 {
        return Ok(Vec::new());
    }
    sqlx::query_scalar(
        "SELECT p.id FROM customer.person p
         WHERE p.tenant_id = $1 AND p.member_state = 'active' AND p.member_type = 'individual'
           AND NOT (p.id = ANY($2))
           AND EXISTS (SELECT 1 FROM customer.postal_address a
                        WHERE a.tenant_id = p.tenant_id AND a.customer_id = p.id)
         ORDER BY random() LIMIT $3",
    )
    .bind(tx.tenant.as_str())
    .bind(never)
    .bind(i64::try_from(n).unwrap_or(1))
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

fn text(v: &Value, key: &str, most: usize) -> String {
    v.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
        .chars()
        .take(most)
        .collect()
}

/// Signs up a new individual member from the bot's made-up identity:
/// `{name, email, telephone?, address: {street_address, address_locality, address_region,
/// postal_code}}`. Returns `{id, address_id, tier_id}`.
pub async fn sign_up(tx: &mut TxCx, body: &Value) -> Result<Value, ToolError> {
    let name = text(body, "name", 120);
    let email = text(body, "email", 200).to_lowercase();
    if name.is_empty() {
        return Err(invalid("name", "A member has a name."));
    }
    if !email.contains('@') || email.contains(' ') {
        return Err(invalid("email", "The email is not an email address."));
    }
    let telephone = Some(text(body, "telephone", 30)).filter(|t| !t.is_empty());
    let address = body.get("address").cloned().unwrap_or(Value::Null);
    let street = text(&address, "street_address", 200);
    let city = text(&address, "address_locality", 100);
    let region = text(&address, "address_region", 50);
    let postal = text(&address, "postal_code", 20);
    if street.is_empty() || city.is_empty() || postal.is_empty() {
        return Err(invalid(
            "address",
            "A new shopper needs a street, a city and a postal code to ship to.",
        ));
    }
    if identifier_taken(tx, "email", &secret::normalize(&email)).await? {
        return Err(
            ToolError::new(crate::DUPLICATE_MEMBER, "A member already uses this email.")
                .detail(json!({"entity": "person", "field": "email"})),
        );
    }
    let now = tx.clock.now();
    let id = format!("cus_{}", acom_core::id::new_suffix());
    // A saved card makes checkout's default payment work, as for every seeded member.
    let card = format!(
        "{:04}",
        acom_core::id::new_suffix()
            .bytes()
            .map(u32::from)
            .sum::<u32>()
            % 10_000
    );
    let (given, family) = match name.split_once(' ') {
        Some((g, f)) => (Some(g.to_owned()), Some(f.to_owned())),
        None => (Some(name.clone()), None),
    };
    sqlx::query(
        "INSERT INTO customer.person (tenant_id, id, name, given_name, family_name, email,
           telephone, member_state, member_type, card_last4, registered_at, email_verified_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 'active', 'individual', $8, $9, $9)",
    )
    .bind(tx.tenant.as_str())
    .bind(&id)
    .bind(&name)
    .bind(given.as_deref())
    .bind(family.as_deref())
    .bind(&email)
    .bind(telephone.as_deref())
    .bind(&card)
    .bind(now)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    let agent = tx.principal.subject.clone();
    let mut tier = None;
    for (program, t) in entry_tiers(tx).await? {
        sqlx::query(
            "INSERT INTO customer.program_membership (tenant_id, customer_id, member_program_id,
               tier_id, membership_number, tier_since)
             VALUES ($1, $2, $3, $4, $5, $6) ON CONFLICT DO NOTHING",
        )
        .bind(tx.tenant.as_str())
        .bind(&id)
        .bind(&program)
        .bind(&t)
        .bind(membership_number())
        .bind(now)
        .execute(&mut *tx.tx)
        .await
        .map_err(db)?;
        repo::add_history(
            tx,
            &repo::TierChange {
                customer_id: id.clone(),
                program_id: program.clone(),
                from_tier_id: None,
                to_tier_id: Some(t.clone()),
                cause: TierChangeCause::Entry.as_str().into(),
                evaluation_id: None,
                snapshot: None,
                agent_id: agent.clone(),
                reason: None,
                expires: None,
            },
        )
        .await?;
        tier.get_or_insert(t);
    }
    let mandatory: Vec<String> = tx
        .policy
        .raw("customer")
        .and_then(|c| c.get("consent"))
        .and_then(|c| c.get("mandatory"))
        .and_then(Value::as_array)
        .map(|l| {
            l.iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default();
    for kind in &mandatory {
        consent::record(tx, &consent::new_id(), &id, kind, true, "sign_up").await?;
    }
    let address_id = format!("adr_{}", acom_core::id::new_suffix());
    sqlx::query(
        "INSERT INTO customer.postal_address (tenant_id, id, customer_id, label, name,
           street_address, address_locality, address_region, postal_code, address_country,
           telephone, address_usage, is_primary)
         VALUES ($1, $2, $3, 'Home', $4, $5, $6, $7, $8, 'US', $9,
                 '{shipping,billing,return}', true)",
    )
    .bind(tx.tenant.as_str())
    .bind(&address_id)
    .bind(&id)
    .bind(&name)
    .bind(&street)
    .bind(&city)
    .bind(Some(region).filter(|r| !r.is_empty()))
    .bind(&postal)
    .bind(telephone.as_deref())
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    acom_changeset::log_action(
        tx,
        acom_changeset::LogEntry {
            object: Some(("person", &id)),
            result: Some(json!({"state": "active", "via": "customer_bot", "tier_id": tier})),
            ..acom_changeset::LogEntry::new(
                acom_contract::ActionType::CREATE.0,
                acom_core::enums::ActionStatus::Completed,
                "register_member",
            )
        },
    )
    .await?;
    Ok(json!({"ok": true, "id": id, "address_id": address_id, "tier_id": tier}))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use crate::testdb::tx;

    #[tokio::test]
    #[ignore = "integration: needs the seeded demo database and MIGRATION_DATABASE_URL"]
    async fn a_new_shopper_joins_with_an_address_and_never_the_cast_is_drawn() {
        let mut tx = tx("sys_customer_bot").await;
        let never = vec!["cus_jamie".to_owned(), "cus_alex".to_owned()];
        let drawn = super::pick_members(&mut tx, 5, &never).await.unwrap();
        assert_eq!(drawn.len(), 5);
        assert!(drawn.iter().all(|d| !never.contains(d)));
        let body = json!({"name": "Maya Carter", "email": "maya.carter.test@shoppers.example",
                          "telephone": "212-555-0134",
                          "address": {"street_address": "18 Oak St", "address_locality": "Portland",
                                      "address_region": "OR", "postal_code": "97205"}});
        let made = super::sign_up(&mut tx, &body).await.unwrap();
        let id = made["id"].as_str().unwrap().to_owned();
        assert!(made["tier_id"].is_string(), "{made}");
        let (state, card): (String, Option<String>) = sqlx::query_as(
            "SELECT member_state, card_last4 FROM customer.person WHERE tenant_id = $1 AND id = $2",
        )
        .bind(tx.tenant.as_str())
        .bind(&id)
        .fetch_one(&mut *tx.tx)
        .await
        .unwrap();
        assert_eq!(state, "active");
        assert_eq!(card.unwrap().len(), 4);
        let again = super::sign_up(&mut tx, &body).await.unwrap_err();
        assert_eq!(again.code, "duplicate_member");
        // Nothing stays: the transaction is dropped.
    }
}
