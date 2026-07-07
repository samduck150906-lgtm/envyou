//! Offline license activation & verification (spec §6.3).
//!
//! # Model
//!
//! A license is a compact, self-contained **Ed25519-signed token**:
//!
//! ```text
//! <payload>.<signature>
//! ```
//!
//! where both parts are URL-safe base64 (no padding). `payload` is a JSON
//! [`LicenseClaims`] document; `signature` is a 64-byte Ed25519 signature over
//! the exact `payload` bytes as transmitted.
//!
//! The license server (Paddle / Lemon Squeezy webhook, or a manual issuer)
//! holds the **private** signing key and mints a signed token per purchase. The
//! app embeds only the corresponding **public** key ([`LICENSE_PUBLIC_KEY_B64`])
//! and verifies signatures fully offline — no network, works air-gapped.
//!
//! This replaces the earlier MVP scheme (`SHA256(key + hardware_id)`), which
//! could be forged locally because the app both produced and checked the token.
//! With asymmetric signatures the app can *verify* but never *mint* a license,
//! so a valid Pro token cannot be manufactured on the client.
//!
//! # ⚠️ Key management (read before shipping)
//!
//! * The signing **private key MUST NEVER be committed to this repository** or
//!   bundled into the app. Generate it once, offline, and keep it in your
//!   payment provider's secret store / a hardware token. See `README.md`
//!   → *License model* for the generation recipe.
//! * [`LICENSE_PUBLIC_KEY_B64`] below ships as an unconfigured placeholder, so
//!   the build **fails closed**: every activation is rejected until the product
//!   owner pastes their real public key. This is intentional — it is safer to
//!   ship with Pro un-activatable than with Pro forgeable.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64URL, Engine};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};

/// The product identifier a valid license must be scoped to.
pub const PRODUCT: &str = "envyou";

/// Base64 (standard) of the 32-byte Ed25519 **public** key used to verify
/// licenses.
///
/// Ships as an unconfigured placeholder so the build fails closed. Replace this
/// with your real public key (see `README.md` → *License model*) before
/// enabling paid activation. NEVER put the matching private key anywhere in this
/// repo.
pub const LICENSE_PUBLIC_KEY_B64: &str = "nsJ4J+OMAg5kjuvCVNcsMdld5i8A+2ZqPyKTq0sCV6Y=";

/// The signed claims carried by a license token.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LicenseClaims {
    /// Product scope — must equal [`PRODUCT`].
    pub product: String,
    /// Plan name, e.g. `"pro"`.
    pub plan: String,
    /// Optional hardware binding. When present, the license is only valid on a
    /// machine whose id matches (see [`crate::core::storage::machine_id`]).
    #[serde(
        rename = "hardwareId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub hardware_id: Option<String>,
    /// ISO-8601 issue time (informational).
    #[serde(rename = "issuedAt")]
    pub issued_at: String,
    /// Optional ISO-8601 expiry. When present and in the past, the license is
    /// rejected. Omit for a perpetual / lifetime license.
    #[serde(rename = "expiresAt", default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<String>,
    /// Feature flags unlocked by this license (e.g. `["unlimited_projects"]`).
    #[serde(default)]
    pub features: Vec<String>,
}

impl LicenseClaims {
    /// Whether a given feature flag is granted.
    pub fn has_feature(&self, feature: &str) -> bool {
        self.features.iter().any(|f| f == feature)
    }
}

/// Cheap structural check: does this look like `<payload>.<signature>` with two
/// non-empty base64url parts? The cryptographic signature check in
/// [`verify_license`] is what actually authorizes Pro.
pub fn is_well_formed(license: &str) -> bool {
    let parts: Vec<&str> = license.trim().split('.').collect();
    parts.len() == 2
        && !parts[0].is_empty()
        && !parts[1].is_empty()
        && B64URL.decode(parts[0]).is_ok()
        && B64URL
            .decode(parts[1])
            .map(|s| s.len() == 64)
            .unwrap_or(false)
}

/// Decode the embedded verification key. Returns an error while the placeholder
/// is still in place, so production activation fails closed.
fn embedded_verifying_key() -> Result<VerifyingKey> {
    verifying_key_from_b64(LICENSE_PUBLIC_KEY_B64).map_err(|_| {
        Error::License(
            "license public key is not configured in this build (set LICENSE_PUBLIC_KEY_B64)"
                .into(),
        )
    })
}

fn verifying_key_from_b64(b64: &str) -> std::result::Result<VerifyingKey, ()> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(b64.trim())
        .map_err(|_| ())?;
    let arr: [u8; 32] = bytes.as_slice().try_into().map_err(|_| ())?;
    VerifyingKey::from_bytes(&arr).map_err(|_| ())
}

/// Verify a signed license against the embedded public key and the current
/// machine, returning the validated claims on success.
///
/// Checks, in order: signature validity, product scope, hardware binding (if
/// any), and expiry (if any).
pub fn verify_license(license: &str, hardware_id: &str) -> Result<LicenseClaims> {
    let key = embedded_verifying_key()?;
    verify_license_with_key(license, hardware_id, &key)
}

/// Core verification, parameterised over the verifying key so tests can supply a
/// throwaway key without touching the shipped public key.
fn verify_license_with_key(
    license: &str,
    hardware_id: &str,
    key: &VerifyingKey,
) -> Result<LicenseClaims> {
    let license = license.trim();
    let (payload_b64, sig_b64) = license.split_once('.').ok_or_else(|| {
        Error::License("malformed license (expected <payload>.<signature>)".into())
    })?;

    let sig_bytes = B64URL
        .decode(sig_b64)
        .map_err(|_| Error::License("bad signature encoding".into()))?;
    let sig_arr: [u8; 64] = sig_bytes
        .as_slice()
        .try_into()
        .map_err(|_| Error::License("signature must be 64 bytes".into()))?;
    let signature = Signature::from_bytes(&sig_arr);

    // Signature is over the exact transmitted payload bytes.
    key.verify(payload_b64.as_bytes(), &signature)
        .map_err(|_| Error::License("invalid license signature".into()))?;

    let payload = B64URL
        .decode(payload_b64)
        .map_err(|_| Error::License("bad payload encoding".into()))?;
    let claims: LicenseClaims = serde_json::from_slice(&payload)
        .map_err(|_| Error::License("malformed license payload".into()))?;

    if claims.product != PRODUCT {
        return Err(Error::License(format!(
            "license is for a different product: {}",
            claims.product
        )));
    }
    if let Some(bound) = &claims.hardware_id {
        if bound != hardware_id {
            return Err(Error::License(
                "license is bound to a different machine".into(),
            ));
        }
    }
    if let Some(exp) = &claims.expires_at {
        if is_expired(exp) {
            return Err(Error::License(format!("license expired on {exp}")));
        }
    }
    Ok(claims)
}

/// Activate a license on this machine: verify it and return the token string to
/// persist verbatim. Errors if the license is invalid, wrong product, bound to
/// another machine, or expired.
pub fn activate(license: &str, hardware_id: &str) -> Result<String> {
    verify_license(license, hardware_id)?;
    Ok(license.trim().to_string())
}

/// Re-verify a previously stored license token against the current machine.
/// Returns `true` only for a currently-valid license.
pub fn verify(license: &str, hardware_id: &str) -> bool {
    verify_license(license, hardware_id).is_ok()
}

/// Mint a signed license token (`<payload>.<signature>`) from claims and a
/// 32-byte Ed25519 **private** signing key.
///
/// Gated behind the `issuer` feature so the shipped desktop app — which never
/// enables it — cannot sign licenses, only verify them. Used by the offline
/// `license_tool` example. Producing a token here uses the exact same payload
/// encoding [`verify_license`] checks, so any token this emits verifies against
/// the matching public key.
#[cfg(feature = "issuer")]
pub fn issue_license(signing_key_bytes: &[u8; 32], claims: &LicenseClaims) -> Result<String> {
    use ed25519_dalek::{Signer, SigningKey};
    let sk = SigningKey::from_bytes(signing_key_bytes);
    let payload = serde_json::to_vec(claims)?;
    let payload_b64 = B64URL.encode(payload);
    let sig = sk.sign(payload_b64.as_bytes());
    Ok(format!("{payload_b64}.{}", B64URL.encode(sig.to_bytes())))
}

/// Whether an ISO-8601 UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`) is in the past.
/// Such timestamps sort lexicographically, so a string compare against "now"
/// is correct without a date-time dependency.
fn is_expired(expires_at: &str) -> bool {
    expires_at < now_iso8601().as_str()
}

/// Current UTC time as `YYYY-MM-DDTHH:MM:SSZ` using only `std`.
fn now_iso8601() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hour, min, sec) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{hour:02}:{min:02}:{sec:02}Z")
}

/// Days since 1970-01-01 → (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    /// Deterministic throwaway signing key for tests.
    fn test_signing_key() -> SigningKey {
        SigningKey::from_bytes(&[7u8; 32])
    }

    /// Mint a signed license the way the (offline) license server would.
    fn issue(key: &SigningKey, claims: &LicenseClaims) -> String {
        let payload = serde_json::to_vec(claims).unwrap();
        let payload_b64 = B64URL.encode(payload);
        let sig = key.sign(payload_b64.as_bytes());
        format!("{payload_b64}.{}", B64URL.encode(sig.to_bytes()))
    }

    fn pro_claims() -> LicenseClaims {
        LicenseClaims {
            product: PRODUCT.into(),
            plan: "pro".into(),
            hardware_id: Some("machine-A".into()),
            issued_at: "2026-01-01T00:00:00Z".into(),
            expires_at: None,
            features: vec!["unlimited_projects".into()],
        }
    }

    #[test]
    fn valid_license_verifies_on_bound_machine() {
        let sk = test_signing_key();
        let vk = sk.verifying_key();
        let lic = issue(&sk, &pro_claims());
        let claims = verify_license_with_key(&lic, "machine-A", &vk).unwrap();
        assert_eq!(claims.plan, "pro");
        assert!(claims.has_feature("unlimited_projects"));
    }

    #[test]
    fn tampered_payload_is_rejected() {
        let sk = test_signing_key();
        let vk = sk.verifying_key();
        let lic = issue(&sk, &pro_claims());
        // Flip a character in the payload so the signature no longer matches.
        let (payload, sig) = lic.split_once('.').unwrap();
        let mut bad_payload = payload.to_string();
        let last = bad_payload.pop().unwrap();
        bad_payload.push(if last == 'A' { 'B' } else { 'A' });
        let tampered = format!("{bad_payload}.{sig}");
        assert!(verify_license_with_key(&tampered, "machine-A", &vk).is_err());
    }

    #[test]
    fn signature_from_a_different_key_is_rejected() {
        let attacker = SigningKey::from_bytes(&[9u8; 32]);
        let real_vk = test_signing_key().verifying_key();
        let lic = issue(&attacker, &pro_claims());
        assert!(
            verify_license_with_key(&lic, "machine-A", &real_vk).is_err(),
            "a license signed by a non-trusted key must not verify"
        );
    }

    #[test]
    fn wrong_hardware_id_is_rejected() {
        let sk = test_signing_key();
        let vk = sk.verifying_key();
        let lic = issue(&sk, &pro_claims());
        assert!(verify_license_with_key(&lic, "machine-B", &vk).is_err());
    }

    #[test]
    fn expired_license_is_rejected() {
        let sk = test_signing_key();
        let vk = sk.verifying_key();
        let mut claims = pro_claims();
        claims.expires_at = Some("2000-01-01T00:00:00Z".into()); // long past
        let lic = issue(&sk, &claims);
        let err = verify_license_with_key(&lic, "machine-A", &vk).unwrap_err();
        assert!(err.to_string().contains("expired"));
    }

    #[test]
    fn future_expiry_is_accepted() {
        let sk = test_signing_key();
        let vk = sk.verifying_key();
        let mut claims = pro_claims();
        claims.expires_at = Some("2999-01-01T00:00:00Z".into());
        let lic = issue(&sk, &claims);
        assert!(verify_license_with_key(&lic, "machine-A", &vk).is_ok());
    }

    #[test]
    fn wrong_product_is_rejected() {
        let sk = test_signing_key();
        let vk = sk.verifying_key();
        let mut claims = pro_claims();
        claims.product = "someone-elses-app".into();
        let lic = issue(&sk, &claims);
        assert!(verify_license_with_key(&lic, "machine-A", &vk).is_err());
    }

    #[test]
    fn unbound_license_works_on_any_machine() {
        let sk = test_signing_key();
        let vk = sk.verifying_key();
        let mut claims = pro_claims();
        claims.hardware_id = None; // floating license
        let lic = issue(&sk, &claims);
        assert!(verify_license_with_key(&lic, "any-machine", &vk).is_ok());
    }

    #[test]
    fn well_formed_rejects_garbage() {
        assert!(!is_well_formed("not-a-license"));
        assert!(!is_well_formed("only-one-part"));
        assert!(!is_well_formed(""));
    }

    // Guards that the offline issuer (license_tool) emits tokens the app accepts.
    #[cfg(feature = "issuer")]
    #[test]
    fn issue_license_produces_app_verifiable_token() {
        use ed25519_dalek::SigningKey;
        let seed = [11u8; 32];
        let vk = SigningKey::from_bytes(&seed).verifying_key();
        let claims = pro_claims();
        let token = issue_license(&seed, &claims).unwrap();
        let verified = verify_license_with_key(&token, "machine-A", &vk).unwrap();
        assert_eq!(verified, claims);
    }

    #[test]
    fn well_formed_accepts_issued_token_shape() {
        let sk = test_signing_key();
        let lic = issue(&sk, &pro_claims());
        assert!(is_well_formed(&lic));
    }

    #[test]
    fn build_fails_closed_until_public_key_configured() {
        // With the placeholder public key in place, the public entry points must
        // reject everything rather than accept an unverifiable license.
        let sk = test_signing_key();
        let lic = issue(&sk, &pro_claims());
        assert!(activate(&lic, "machine-A").is_err());
        assert!(!verify(&lic, "machine-A"));
    }
}
