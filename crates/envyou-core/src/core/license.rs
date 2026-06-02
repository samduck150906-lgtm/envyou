//! Offline license activation & verification (spec §6.3).
//!
//! Paddle issues a license key by email after a one-time purchase. Because the
//! app must work on air-gapped machines, we cannot phone home. Instead, on
//! activation we bind the key to the local hardware id and store an encrypted
//! token; verification re-derives that token and compares.
//!
//! NOTE: This MVP scheme proves possession of the key *on this machine*. A
//! production build would additionally verify a Paddle-signed key with an
//! embedded public key (Ed25519). The verification surface is isolated here so
//! that upgrade is a drop-in change.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// Domain-separation tag for the activation token.
const TOKEN_TAG: &[u8] = b"envyou::license::token::v1";

/// Minimal structural validation of a Paddle-style license key.
///
/// Accepts keys of the form `XXXX-XXXX-XXXX-XXXX` (alphanumeric groups). This
/// is deliberately lenient — the cryptographic binding in [`activate`] is what
/// actually matters for offline verification.
pub fn is_well_formed(license_key: &str) -> bool {
    let key = license_key.trim();
    let groups: Vec<&str> = key.split('-').collect();
    groups.len() == 4
        && groups
            .iter()
            .all(|g| g.len() == 4 && g.chars().all(|c| c.is_ascii_alphanumeric()))
}

/// Produce a hardware-bound activation token for a license key.
fn token_for(license_key: &str, hardware_id: &str) -> String {
    let mut h = Sha256::new();
    h.update(TOKEN_TAG);
    h.update(license_key.trim().as_bytes());
    h.update(b"::");
    h.update(hardware_id.as_bytes());
    B64.encode(h.finalize())
}

/// Activate a license on this machine. Returns the token to persist in
/// [`crate::core::model::License`]/the encrypted state.
pub fn activate(license_key: &str, hardware_id: &str) -> Result<String> {
    if !is_well_formed(license_key) {
        return Err(Error::License(
            "license key format is invalid (expected XXXX-XXXX-XXXX-XXXX)".into(),
        ));
    }
    Ok(token_for(license_key, hardware_id))
}

/// Verify a previously stored token against the current machine.
pub fn verify(license_key: &str, hardware_id: &str, stored_token: &str) -> bool {
    if !is_well_formed(license_key) {
        return false;
    }
    let expected = token_for(license_key, hardware_id);
    // Constant-time-ish compare.
    constant_time_eq(expected.as_bytes(), stored_token.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_validation() {
        assert!(is_well_formed("AB12-CD34-EF56-GH78"));
        assert!(!is_well_formed("too-short"));
        assert!(!is_well_formed("ABC-DEF-GHI-JKL"));
        assert!(!is_well_formed("AB12-CD34-EF56-GH7!"));
    }

    #[test]
    fn activate_then_verify() {
        let key = "AB12-CD34-EF56-GH78";
        let hw = "machine-xyz";
        let token = activate(key, hw).unwrap();
        assert!(verify(key, hw, &token));
    }

    #[test]
    fn token_is_machine_bound() {
        let key = "AB12-CD34-EF56-GH78";
        let token = activate(key, "machine-A").unwrap();
        // Same key, different machine => token must not validate.
        assert!(!verify(key, "machine-B", &token));
    }

    #[test]
    fn wrong_key_does_not_verify() {
        let hw = "m";
        let token = activate("AB12-CD34-EF56-GH78", hw).unwrap();
        assert!(!verify("ZZ99-ZZ99-ZZ99-ZZ99", hw, &token));
    }

    #[test]
    fn invalid_format_cannot_activate() {
        assert!(activate("nope", "m").is_err());
    }
}
