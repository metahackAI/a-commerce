//! What `messaging` reads from customer: whether a shopper exists, their name (masked in
//! previews), which channels they can be reached on, and, for a campaign, who is in its audience
//! and whether each of them agreed to marketing on its channel (`consent.marketing_gate`).

use acom_contract::ToolError;
use acom_store::{StoreError, TxCx};

/// How a shopper can be reached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contact {
    pub id: String,
    pub name: String,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    /// `None` when the shopper gave no email.
    pub email: Option<String>,
    /// E.164; `None` when the shopper gave no phone.
    pub telephone: Option<String>,
    /// `active`, `dormant`, `blocked`, `pending_withdrawal` or `withdrawn`.
    pub member_state: String,
}

/// One shopper's contact details, or `None` when the store has no such shopper.
pub async fn contact(tx: &mut TxCx, customer_id: &str) -> Result<Option<Contact>, ToolError> {
    #[allow(clippy::type_complexity)]
    let row: Option<(
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    )> = sqlx::query_as(
        "SELECT id, name, given_name, family_name, email, telephone, member_state
         FROM customer.person WHERE tenant_id = $1 AND id = $2",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_optional(&mut *tx.tx)
    .await
    .map_err(|e| ToolError::from(StoreError::from(e)))?;
    Ok(row.map(|r| Contact {
        id: r.0,
        name: r.1,
        given_name: r.2,
        family_name: r.3,
        email: r.4,
        telephone: r.5,
        member_state: r.6,
    }))
}

/// Who a campaign goes to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Audience<'a> {
    /// The members of one tier.
    Tier(&'a str),
    /// These shoppers.
    Customers(&'a [String]),
    /// Every shopper of the store.
    Everyone,
}

/// One shopper of a campaign's audience, with whether they may get marketing on its channel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reach {
    pub contact: Contact,
    /// The newest `marketing_<channel>` consent record grants it; no record means no.
    pub consented: bool,
}

/// Whether the store has a tier with this id.
pub async fn tier_exists(tx: &mut TxCx, tier: &str) -> Result<bool, ToolError> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM customer.member_program_tier WHERE tenant_id = $1 AND id = $2)",
    )
    .bind(tx.tenant.as_str())
    .bind(tier)
    .fetch_one(&mut *tx.tx)
    .await
    .map_err(|e| ToolError::from(StoreError::from(e)))
}

/// The shoppers of an audience in id order, each with their marketing consent for `channel`
/// (`email`, `sms`, `push`, `in_app`; the consent type is `marketing_<channel>`). A listed id
/// the store does not have is left out.
pub async fn audience(
    tx: &mut TxCx,
    who: Audience<'_>,
    channel: &str,
) -> Result<Vec<Reach>, ToolError> {
    let (tier, ids): (Option<&str>, Option<&[String]>) = match who {
        Audience::Tier(t) => (Some(t), None),
        Audience::Customers(list) => (None, Some(list)),
        Audience::Everyone => (None, None),
    };
    #[allow(clippy::type_complexity)]
    let rows: Vec<(
        String,
        String,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
        bool,
    )> = sqlx::query_as(
        "SELECT p.id, p.name, p.given_name, p.family_name, p.email, p.telephone, p.member_state,
                COALESCE(c.granted, false)
         FROM customer.person p
         LEFT JOIN LATERAL (
           SELECT r.granted FROM customer.consent_record r
           WHERE r.tenant_id = p.tenant_id AND r.customer_id = p.id AND r.consent_type = $2
           ORDER BY r.recorded_at DESC, r.id DESC LIMIT 1
         ) c ON true
         WHERE p.tenant_id = $1
           AND ($3::text IS NULL OR EXISTS (
                 SELECT 1 FROM customer.program_membership m
                 WHERE m.tenant_id = p.tenant_id AND m.customer_id = p.id AND m.tier_id = $3))
           AND ($4::text[] IS NULL OR p.id = ANY($4))
         ORDER BY p.id",
    )
    .bind(tx.tenant.as_str())
    .bind(format!("marketing_{channel}"))
    .bind(tier)
    .bind(ids)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(|e| ToolError::from(StoreError::from(e)))?;
    Ok(rows
        .into_iter()
        .map(|r| Reach {
            contact: Contact {
                id: r.0,
                name: r.1,
                given_name: r.2,
                family_name: r.3,
                email: r.4,
                telephone: r.5,
                member_state: r.6,
            },
            consented: r.7,
        })
        .collect())
}
