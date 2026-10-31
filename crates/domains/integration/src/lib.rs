//! The `integration` domain: adapters between the core domains and outside channels, carriers
//! and payment providers; bulk imports and inbound events. Business rules stay in the core
//! domains. Specified in `docs/api/integration.md`; rules in `policies/integration.yml`.
//!
//! Layout (docs/backend/README.md §2.2): one folder per resource of the spec (`product_import`,
//! `order`, `claim`, `channel`, `event`), one file per operation named after its tool.
//!
//! The adapter never writes another domain's tables: new products are catalog drafts made by
//! catalog's own draft path, orders and payments move through sales, parcels, claims and refunds
//! through fulfillment (each domain's `service::for_integration`). No outside system is
//! connected in this store: lookups read the copies the channels last sent, a connection test
//! checks the address and credential only for form, and an outbound sync is recorded, not sent.
//! Every such answer says so.

pub mod service;

mod channel;
mod claim;
mod event;
mod model;
mod order;
mod product_import;
mod repo;
mod rules;

#[cfg(test)]
pub(crate) mod testing;

use acom_contract::{ErrorClass, ErrorCode, RegistryBuilder, ToolError};
use acom_core::Money;
use acom_store::Cx;
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub use channel::{ListChannels, RegisterChannel};
pub use claim::{LookupExternalClaim, SyncClaimStatus};
pub use event::{ApplyPendingEvent, ReceiveCarrierEvent, ReceivePaymentEvent};
pub use order::{LookupExternalOrder, SyncOrderStatus};
pub use product_import::{GetImportStatus, ImportProducts};

/// An outside category word that matches no standard category (`import.category_mapping`).
pub const CATEGORY_UNMAPPED: ErrorCode = ErrorCode::new("category_unmapped", ErrorClass::Domain);
/// An outside status word the channel's mapping does not name (`status_mapping.unmapped`).
pub const STATUS_UNMAPPED: ErrorCode = ErrorCode::new("status_unmapped", ErrorClass::Domain);
/// A seller feed from a seller who is not active.
pub const SELLER_NOT_ACTIVE: ErrorCode =
    ErrorCode::new("seller_not_active", ErrorClass::InvalidState);
/// A price under its floor (`price_floor`).
pub const BELOW_PRICE_FLOOR: ErrorCode =
    ErrorCode::new("below_price_floor", ErrorClass::PolicyViolation);
/// More rows than `import.max_items_per_batch`.
pub const BATCH_TOO_LARGE: ErrorCode = ErrorCode::new("batch_too_large", ErrorClass::LimitExceeded);
/// No import with that id.
pub const IMPORT_NOT_FOUND: ErrorCode = ErrorCode::new("import_not_found", ErrorClass::NotFound);
/// No channel with that id (tools an operator calls).
pub const CHANNEL_NOT_REGISTERED: ErrorCode =
    ErrorCode::new("channel_not_registered", ErrorClass::NotFound);
/// No channel with that id (events an outside system sends).
pub const UNKNOWN_CHANNEL: ErrorCode = ErrorCode::new("unknown_channel", ErrorClass::NotFound);
/// The channel cannot be read now.
pub const EXTERNAL_LOOKUP_FAILED: ErrorCode =
    ErrorCode::new("external_lookup_failed", ErrorClass::UpstreamUnavailable);
/// A status move the owning domain does not allow.
pub const INVALID_TRANSITION: ErrorCode =
    ErrorCode::new("invalid_transition", ErrorClass::InvalidState);
/// An outside order the store has no order for.
pub const ORDER_NOT_MAPPED: ErrorCode = ErrorCode::new("order_not_mapped", ErrorClass::NotFound);
/// An outside claim the store has no claim for.
pub const CLAIM_NOT_MAPPED: ErrorCode = ErrorCode::new("claim_not_mapped", ErrorClass::NotFound);
/// The connection test failed (`channel.connection_test_required`).
pub const CONNECTION_FAILED: ErrorCode =
    ErrorCode::new("connection_failed", ErrorClass::UpstreamUnavailable);
/// A status mapping that leaves out a part `status_mapping.must_cover` names.
pub const MAPPING_INCOMPLETE: ErrorCode =
    ErrorCode::new("mapping_incomplete", ErrorClass::InvalidInput);
/// A channel with the same name and address exists.
pub const DUPLICATE_CHANNEL: ErrorCode =
    ErrorCode::new("duplicate_channel", ErrorClass::AlreadyExists);
/// An event whose signature does not match the channel's signing secret.
pub const INVALID_SIGNATURE: ErrorCode =
    ErrorCode::new("invalid_signature", ErrorClass::Unauthenticated);
/// A payment event whose amount is not the payment's or the refund's.
pub const AMOUNT_MISMATCH: ErrorCode = ErrorCode::new("amount_mismatch", ErrorClass::Domain);
/// A feed row without a value `import.required_fields` asks for.
pub const MISSING_REQUIRED_FIELD: ErrorCode =
    ErrorCode::new("missing_required_field", ErrorClass::InvalidInput);
/// A feed row whose SKU another product has.
pub const DUPLICATE_SKU: ErrorCode = ErrorCode::new("duplicate_sku", ErrorClass::AlreadyExists);

/// Adds every operation of this domain to the registry, and the job that applies events whose
/// parcel, payment or refund was not recorded yet when they came.
pub fn register(b: &mut RegistryBuilder<Cx>) {
    b.write(ImportProducts)
        .read(GetImportStatus)
        .read(LookupExternalOrder)
        .write(SyncOrderStatus)
        .read(LookupExternalClaim)
        .write(SyncClaimStatus)
        .read(ListChannels)
        .write(RegisterChannel)
        .write(ReceiveCarrierEvent)
        .write(ReceivePaymentEvent)
        .job(ApplyPendingEvent);
}

/// Refuses an id without its prefix (`chn_…`). The schema says so too; this keeps a direct
/// call honest.
pub(crate) fn check_id(field: &str, prefix: &str, value: &str) -> Result<(), ToolError> {
    let rest = value.strip_prefix(prefix).and_then(|r| r.strip_prefix('_'));
    let ok = rest.is_some_and(|r| {
        !r.is_empty()
            && r.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    });
    if ok {
        Ok(())
    } else {
        Err(
            ToolError::invalid_input(format!("`{field}` must look like {prefix}_…."))
                .detail(json!({"field": field, "reason": "format"})),
        )
    }
}

/// A required text: trimmed, not empty.
pub(crate) fn required_text(field: &str, value: &str) -> Result<String, ToolError> {
    let v = value.trim();
    if v.is_empty() {
        return Err(ToolError::invalid_input(format!("`{field}` is empty."))
            .detail(json!({"field": field, "reason": "missing"})));
    }
    Ok(v.to_owned())
}

/// A time an input names, read in the store's time zone when it has no offset.
pub(crate) fn parse_time(cx: &Cx, field: &str, text: &str) -> Result<DateTime<Utc>, ToolError> {
    cx.time.parse(text).map_err(|e| {
        ToolError::invalid_input(format!("`{field}`: {e}"))
            .detail(json!({"field": field, "reason": "format"}))
    })
}

/// `$58.00`, `€10.00`.
pub(crate) fn money_text(m: &Money) -> String {
    let digits = m.currency.minor_units() as usize;
    let value = m.rounded().value;
    match m.currency.as_str() {
        "USD" => format!("${value:.digits$}"),
        "EUR" => format!("€{value:.digits$}"),
        other => format!("{value:.digits$} {other}"),
    }
}

/// SHA-256 of a text, as 64 lowercase hex characters.
pub(crate) fn sha256_hex(text: &str) -> String {
    hex(&Sha256::digest(text.as_bytes()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// HMAC-SHA256 (RFC 2104) of `message` under `key`, as lowercase hex.
pub(crate) fn hmac_sha256_hex(key: &[u8], message: &[u8]) -> String {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    inner.update(k.map(|b| b ^ 0x36));
    inner.update(message);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(k.map(|b| b ^ 0x5c));
    outer.update(inner);
    hex(&outer.finalize())
}

/// Compares two texts in time that does not depend on where they differ.
pub(crate) fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0u8, |acc, (x, y)| acc | (x ^ y))
            == 0
}

/// A new secret: `prefix` and 64 hex characters from the operating system's generator.
pub(crate) fn new_secret(prefix: &str) -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    format!("{prefix}{}", hex(&bytes))
}

/// A new id with a prefix and a lowercase ULID (`whk_01jc…`).
pub(crate) fn new_id(prefix: &str) -> String {
    format!("{prefix}_{}", acom_core::id::new_suffix())
}

/// What an answer says about an outside system that is not connected.
pub(crate) fn not_connected(channel_id: &str, what: &str) -> Value {
    json!({"channel_id": channel_id, "connected": false,
           "note": format!("{channel_id} is not connected in this store: {what}")})
}

/// A list answer with its total and, when there is more, the next cursor.
pub(crate) fn list(items: Vec<Value>, total: i64, page: &acom_store::Page) -> Value {
    let returned = items.len();
    let mut out = json!({"items": items, "total": total});
    if let Some(c) = page.next_cursor(returned, total) {
        out["next_cursor"] = json!(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_matches_the_rfc_4231_vectors() {
        // RFC 4231 test case 2.
        assert_eq!(
            hmac_sha256_hex(b"Jefe", b"what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
        // Test case 6: a key longer than the block is hashed first.
        assert_eq!(
            hmac_sha256_hex(
                &[0xaa; 131],
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            ),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn ids_need_their_prefix() {
        assert!(check_id("channel_id", "chn", "chn_carrier_ups").is_ok());
        assert!(check_id("channel_id", "chn", "carrier_ups").is_err());
        assert!(check_id("channel_id", "chn", "chn_").is_err());
        assert!(check_id("channel_id", "chn", "chn_UPS").is_err());
    }

    #[test]
    fn secrets_are_random_and_compared_whole() {
        let a = new_secret("whsec_");
        assert!(a.starts_with("whsec_") && a.len() == 6 + 64);
        assert_ne!(a, new_secret("whsec_"));
        assert!(same_secret("abc", "abc"));
        assert!(!same_secret("abc", "abd"));
        assert!(!same_secret("abc", "abcd"));
        assert_eq!(sha256_hex("abc").len(), 64);
    }

    #[test]
    fn every_operation_registers_with_a_closed_schema() {
        let mut b = RegistryBuilder::default();
        register(&mut b);
        let r = b.build().unwrap_or_else(|e| panic!("{e:#?}"));
        assert_eq!(r.len(), 10);
        assert_eq!(
            r.jobs().map(|j| j.kind).collect::<Vec<_>>(),
            ["integration.apply_event"]
        );
    }
}
