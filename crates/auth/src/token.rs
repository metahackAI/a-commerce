//! The internal token: the gateway signs (Ed25519), commerce and agent verify.

use std::collections::BTreeSet;

use acom_core::TenantId;
use acom_core::enums::ActorKind;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};

use crate::{Principal, Surface};

const ISSUER: &str = "acom-gateway";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claims {
    pub iss: String,
    pub aud: Vec<String>,
    pub sub: String,
    pub kind: ActorKind,
    pub tenant: TenantId,
    pub surface: Surface,
    pub scopes: BTreeSet<String>,
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub iat: u64,
    pub exp: u64,
}

impl Claims {
    pub fn principal(&self) -> Principal {
        Principal {
            subject: self.sub.clone(),
            kind: self.kind,
            tenant: self.tenant.clone(),
            surface: self.surface,
            scopes: self.scopes.clone(),
            name: self.name.clone(),
            session_id: self.session_id.clone(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TokenError {
    #[error("cannot read key: {0}")]
    Key(String),
    #[error("token rejected: {0}")]
    Invalid(String),
}

/// Signs internal tokens. Only the gateway has one.
pub struct Signer {
    key: EncodingKey,
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Signer")
    }
}

impl Signer {
    pub fn from_pem(pem: &[u8]) -> Result<Self, TokenError> {
        EncodingKey::from_ed_pem(pem)
            .map(|key| Self { key })
            .map_err(|e| TokenError::Key(e.to_string()))
    }

    pub fn from_pem_file(path: &str) -> Result<Self, TokenError> {
        let pem = std::fs::read(path).map_err(|e| TokenError::Key(format!("{path}: {e}")))?;
        Self::from_pem(&pem)
    }

    /// A token for `principal`, valid for `ttl_seconds`, addressed to commerce and agent.
    pub fn sign(
        &self,
        principal: &Principal,
        request_id: &str,
        now: u64,
        ttl_seconds: u64,
    ) -> Result<String, TokenError> {
        let claims = Claims {
            iss: ISSUER.to_owned(),
            aud: vec!["commerce".to_owned(), "agent".to_owned()],
            sub: principal.subject.clone(),
            kind: principal.kind,
            tenant: principal.tenant.clone(),
            surface: principal.surface,
            scopes: principal.scopes.clone(),
            request_id: request_id.to_owned(),
            name: principal.name.clone(),
            session_id: principal.session_id.clone(),
            iat: now,
            exp: now + ttl_seconds,
        };
        jsonwebtoken::encode(&Header::new(Algorithm::EdDSA), &claims, &self.key)
            .map_err(|e| TokenError::Invalid(e.to_string()))
    }
}

/// Verifies internal tokens for one audience (`commerce` or `agent`).
pub struct Verifier {
    key: DecodingKey,
    validation: Validation,
}

impl std::fmt::Debug for Verifier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Verifier")
    }
}

impl Verifier {
    pub fn from_pem(pem: &[u8], audience: &str) -> Result<Self, TokenError> {
        let key = DecodingKey::from_ed_pem(pem).map_err(|e| TokenError::Key(e.to_string()))?;
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_audience(&[audience]);
        validation.set_issuer(&[ISSUER]);
        validation.leeway = 5;
        Ok(Self { key, validation })
    }

    pub fn from_pem_file(path: &str, audience: &str) -> Result<Self, TokenError> {
        let pem = std::fs::read(path).map_err(|e| TokenError::Key(format!("{path}: {e}")))?;
        Self::from_pem(&pem, audience)
    }

    pub fn verify(&self, token: &str) -> Result<Claims, TokenError> {
        jsonwebtoken::decode::<Claims>(token, &self.key, &self.validation)
            .map(|d| d.claims)
            .map_err(|e| TokenError::Invalid(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scopes;

    /// A throwaway Ed25519 key pair, generated with openssl for this test run.
    fn openssl_pair() -> Option<(Vec<u8>, Vec<u8>)> {
        let dir = std::env::temp_dir().join(format!("acom-token-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).ok()?;
        let private = dir.join("k.pem");
        let public = dir.join("p.pem");
        let ok = std::process::Command::new("openssl")
            .args(["genpkey", "-algorithm", "ed25519", "-out"])
            .arg(&private)
            .status()
            .ok()?
            .success()
            && std::process::Command::new("openssl")
                .args(["pkey", "-pubout", "-in"])
                .arg(&private)
                .arg("-out")
                .arg(&public)
                .status()
                .ok()?
                .success();
        let pair = ok.then(|| (std::fs::read(&private).ok(), std::fs::read(&public).ok()));
        std::fs::remove_dir_all(&dir).ok();
        match pair {
            Some((Some(a), Some(b))) => Some((a, b)),
            _ => None,
        }
    }

    #[test]
    fn keys_round_trip_and_audience_is_checked() {
        let Some((private, public)) = openssl_pair() else {
            eprintln!("openssl not available; skipping");
            return;
        };
        let signer = Signer::from_pem(&private).unwrap();
        let principal = Principal {
            subject: "adm_admin".into(),
            kind: ActorKind::Operator,
            tenant: TenantId::parse("store_northwind").unwrap(),
            surface: Surface::Bo,
            scopes: scopes::admin(),
            name: Some("Admin".into()),
            session_id: Some("ses_1".into()),
        };
        let now = 1_800_000_000;
        let token = signer.sign(&principal, "req_1", now, 300).unwrap();

        let commerce = Verifier::from_pem(&public, "commerce").unwrap();
        // Validate against a fixed clock: jsonwebtoken checks exp against the system time,
        // so sign relative to now in real use; here only the signature and audience matter.
        let mut v = commerce;
        v.validation.validate_exp = false;
        let claims = v.verify(&token).unwrap();
        assert_eq!(claims.principal(), principal);

        let mut other = Verifier::from_pem(&public, "billing").unwrap();
        other.validation.validate_exp = false;
        assert!(other.verify(&token).is_err());
    }
}
