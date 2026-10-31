//! One-time secrets (verification codes, reset tokens) and one-way fingerprints.
//!
//! A secret is made once, handed to whoever must deliver it, and stored only as its SHA-256
//! hash. Secrets never go to the action log, a change set, a preview, or any output other than
//! the one step that hands them out (a `verification_token` to the caller that proved the code).
//! Passwords are not secrets of this kind: they are hashed with argon2 (`acom_auth::password`).

use rand::RngCore;
use sha2::{Digest, Sha256};

/// 32 random bytes as 64 hex characters: a reset link token or a verification token.
pub(crate) fn token() -> String {
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A one-time code of `len` digits (`identity_verification.code.length`).
pub(crate) fn code(len: usize) -> String {
    let mut rng = rand::rng();
    (0..len.clamp(4, 12))
        .map(|_| char::from(b'0' + u8::try_from(rng.next_u32() % 10).unwrap_or(0)))
        .collect()
}

/// SHA-256 of a secret, as hex.
pub(crate) fn hash(secret: &str) -> String {
    Sha256::digest(secret.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// The one-way fingerprint of an identifier (email, phone, sign-in id): the hash of the trimmed,
/// lowercased text, with spaces, dashes and parentheses taken out of phone numbers.
pub(crate) fn fingerprint(identifier: &str) -> String {
    hash(&normalize(identifier))
}

/// How identifiers are compared: trimmed and lowercased; phone numbers without separators.
pub(crate) fn normalize(identifier: &str) -> String {
    let t = identifier.trim().to_lowercase();
    let phone_like = t
        .chars()
        .all(|c| c.is_ascii_digit() || " +-().".contains(c));
    if phone_like {
        t.chars()
            .filter(|c| c.is_ascii_digit() || *c == '+')
            .collect()
    } else {
        t
    }
}

/// Whether `secret` hashes to `stored`, comparing in constant time.
pub(crate) fn matches(stored: &str, secret: &str) -> bool {
    let given = hash(secret);
    if given.len() != stored.len() {
        return false;
    }
    given
        .bytes()
        .zip(stored.bytes())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_and_codes_have_their_shape() {
        let t = token();
        assert_eq!(t.len(), 64);
        assert!(t.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(t, token());
        let c = code(6);
        assert_eq!(c.len(), 6);
        assert!(c.bytes().all(|b| b.is_ascii_digit()));
    }

    #[test]
    fn hashes_compare_and_identifiers_normalize() {
        let h = hash("482913");
        assert!(matches(&h, "482913"));
        assert!(!matches(&h, "482914"));
        assert_eq!(
            fingerprint(" Alex.Kim@Example.com "),
            fingerprint("alex.kim@example.com")
        );
        assert_eq!(normalize("+1 (718) 555-0142"), "+17185550142");
    }
}
