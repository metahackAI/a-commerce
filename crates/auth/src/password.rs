//! Password hashes (argon2id). Only the gateway checks passwords.

use argon2::Argon2;
use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};

/// Hashes a password into a PHC string (`$argon2id$...`).
pub fn hash(password: &str) -> Result<String, String> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

/// `true` when `password` matches the stored PHC string. Unreadable hashes never match.
pub fn verify(password: &str, phc: &str) -> bool {
    PasswordHash::new(phc)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hashes_verify_and_reject_the_wrong_password() {
        let h = hash("admin123").unwrap();
        assert!(h.starts_with("$argon2id$"));
        assert!(verify("admin123", &h));
        assert!(!verify("admin124", &h));
        assert!(!verify("admin123", "not a hash"));
    }
}
