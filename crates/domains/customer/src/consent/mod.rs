//! `consent`: what a member agreed to (`customer.consent_record`). Records are only added: a
//! change of mind is a new record, and the newest record of each consent type is the member's
//! current answer. Marketing sends read it through `service::for_messaging` (opt-in gate).

use acom_contract::ToolError;
use acom_store::TxCx;
use chrono::{DateTime, Utc};

use crate::access::db;

mod get_consent;
mod set_consent;

pub use get_consent::*;
pub use set_consent::*;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub(crate) struct ConsentRow {
    pub id: String,
    pub consent_type: String,
    pub granted: bool,
    pub recorded_at: DateTime<Utc>,
}

/// The newest record of each consent type of a member, in the order of the type list.
pub(crate) async fn current(
    tx: &mut TxCx,
    customer_id: &str,
) -> Result<Vec<ConsentRow>, ToolError> {
    sqlx::query_as::<_, ConsentRow>(
        "SELECT DISTINCT ON (consent_type) id, consent_type, granted, recorded_at
         FROM customer.consent_record WHERE tenant_id = $1 AND customer_id = $2
         ORDER BY consent_type, recorded_at DESC, id DESC",
    )
    .bind(tx.tenant.as_str())
    .bind(customer_id)
    .fetch_all(&mut *tx.tx)
    .await
    .map_err(db)
}

/// Adds one record (a repeated run with the same id adds nothing).
pub(crate) async fn record(
    tx: &mut TxCx,
    id: &str,
    customer_id: &str,
    consent_type: &str,
    granted: bool,
    instrument: &str,
) -> Result<(), ToolError> {
    sqlx::query(
        "INSERT INTO customer.consent_record (tenant_id, id, customer_id, consent_type, granted,
           recorded_at, agent_id, instrument)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8) ON CONFLICT (tenant_id, id) DO NOTHING",
    )
    .bind(tx.tenant.as_str())
    .bind(id)
    .bind(customer_id)
    .bind(consent_type)
    .bind(granted)
    .bind(tx.clock.now())
    .bind(&tx.principal.subject)
    .bind(instrument)
    .execute(&mut *tx.tx)
    .await
    .map_err(db)?;
    Ok(())
}

/// A new consent record id.
pub(crate) fn new_id() -> String {
    format!("cns_{}", acom_core::id::new_suffix())
}

#[cfg(test)]
mod tests {
    use acom_contract::{ReadOp, WriteOp};
    use acom_core::enums::ConsentType;

    use super::*;

    /// Integration: needs a seeded database (`DATABASE_URL`). Run with `-- --ignored`.
    #[tokio::test]
    #[ignore = "integration: needs a PostgreSQL database"]
    async fn adds_records_and_keeps_the_mandatory_ones() {
        let cx = crate::testdb::cx("cus_alex").await;
        let now = GetConsent
            .read(
                &cx,
                GetConsentInput {
                    customer_id: "me".into(),
                },
            )
            .await
            .unwrap();
        let email = now["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|c| c["consent_type"] == "marketing_email")
            .cloned()
            .unwrap();
        assert_eq!(email["granted"], true);
        assert_eq!(email["id"], "cns_412");
        let e = SetConsent
            .preview(
                &cx,
                SetConsentInput {
                    customer_id: "me".into(),
                    consent_type: ConsentType::Terms,
                    granted: false,
                },
            )
            .await
            .unwrap_err();
        assert_eq!(e.code, "cannot_revoke_mandatory");
        let planned = SetConsent
            .preview(
                &cx,
                SetConsentInput {
                    customer_id: "me".into(),
                    consent_type: ConsentType::MarketingEmail,
                    granted: false,
                },
            )
            .await
            .unwrap();
        assert_eq!(
            planned.changes[0].to,
            serde_json::json!({"marketing_email": false})
        );
        let mut tx = cx.begin().await.unwrap();
        let done = SetConsent.commit(&mut tx, &planned.plan).await.unwrap();
        let rows = current(&mut tx, "cus_alex").await.unwrap();
        let email = rows
            .iter()
            .find(|r| r.consent_type == "marketing_email")
            .unwrap();
        assert_eq!(
            (email.id.as_str(), email.granted),
            (done.id.as_str(), false)
        );
        // Rolled back on drop.
    }
}
