//! Offline license activation & verification.
//!
//! # Two layers: a short *code* and a signed *certificate*
//!
//! Buyers see a short, human-friendly **license code** (e.g.
//! `ENVY-K7M4-9Q2P-D8X6-R3TA`) — see [`generate_license_code`]. That code is
//! only a lookup key into the license database; it is *not* a token the app can
//! verify by itself. When the buyer activates, the app exchanges the code (plus
//! their email) at the activation server, which returns the real
//! **certificate**: a compact, self-contained **Ed25519-signed token**
//!
//! ```text
//! <payload>.<signature>
//! ```
//!
//! where both parts are URL-safe base64 (no padding). `payload` is a JSON
//! [`LicenseClaims`] document; `signature` is a 64-byte Ed25519 signature over
//! the exact `payload` bytes as transmitted. The app verifies this certificate
//! fully offline against the embedded public key and stores it — after the first
//! activation Pro works air-gapped, and every load re-verifies the stored
//! certificate rather than trusting a persisted boolean.
//!
//! The license server (the Paddle webhook, or a manual issuer) holds the
//! **private** signing key, mints one certificate per purchase, and stores it in
//! the DB keyed by the short code. The app embeds only the corresponding
//! **public** key ([`LICENSE_PUBLIC_KEY_B64`]). Because the signing key never
//! ships, the app can *verify* but never *mint* a certificate, so a valid Pro
//! certificate cannot be manufactured on the client. This also fixes the older
//! "email the whole token" scheme, where line-wrapping a long token in an email
//! client produced an `invalid license signature` on paste (verification now
//! strips all whitespace, and the visible artifact is the short code anyway).
//!
//! See `docs/LICENSE_SYSTEM.md` and `docs/ACTIVATION_FLOW.md` for the full flow.
//!
//! # ⚠️ Key management (read before shipping)
//!
//! * The signing **private key MUST NEVER be committed to this repository** or
//!   bundled into the app. Generate it once, offline, and keep it only in the
//!   issuer's secret store (Railway `ENVYOU_SIGNING_KEY_B64`). See
//!   `docs/LICENSE_SYSTEM.md` → *Key management* for the recipe.
//! * [`LICENSE_PUBLIC_KEY_B64`] below is set to the production public key. To
//!   force the build closed (reject every activation), reset it to
//!   [`UNCONFIGURED_PUBLIC_KEY_B64`] or empty — shipping un-activatable is safer
//!   than shipping forgeable. `license_tool checkkey` proves the shipped public
//!   key matches the webhook's signing seed before you release.

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
/// This is the production public key; its private half lives only in the
/// license issuer's secret store (Railway `ENVYOU_SIGNING_KEY_B64`), never in
/// this repo. To rotate, generate a new keypair with the offline `license_tool`
/// and replace this value, then run `license_tool checkkey` to confirm the new
/// seed matches. Set it back to [`UNCONFIGURED_PUBLIC_KEY_B64`] (or empty) to
/// force the build closed.
pub const LICENSE_PUBLIC_KEY_B64: &str = "EAEkl5eCcd6cnEXb3Ij7DlVIu6BE2/6wxiR/kNM2qEo=";

/// Sentinel meaning "no license key configured yet". While
/// [`LICENSE_PUBLIC_KEY_B64`] equals this (or is empty), the build **fails
/// closed** and rejects every activation. It is intentionally not valid base64
/// so it can never collide with a real 32-byte key.
const UNCONFIGURED_PUBLIC_KEY_B64: &str = "REPLACE_WITH_YOUR_ED25519_PUBLIC_KEY_B64";

/// Whether this build ships a real (non-placeholder, non-empty) license public
/// key. The offline `license_tool checkkey` command and the app can surface this
/// to avoid shipping a build that either rejects every real license or would
/// accept a forgeable one.
pub fn is_license_key_configured() -> bool {
    is_configured_key(LICENSE_PUBLIC_KEY_B64)
}

/// Value-level check used by [`is_license_key_configured`] and
/// [`verifying_key_configured`]; a standalone fn so tests can exercise the
/// fail-closed logic without mutating the shipped constant.
fn is_configured_key(key_b64: &str) -> bool {
    let k = key_b64.trim();
    !k.is_empty() && k != UNCONFIGURED_PUBLIC_KEY_B64
}

/// The signed claims carried by a license certificate (the internal
/// `<payload>.<signature>` token the app stores and verifies offline). Users
/// never see this — they see the short [`generate_license_code`] code, which the
/// activation server exchanges for this certificate.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct LicenseClaims {
    /// Product scope — must equal [`PRODUCT`].
    pub product: String,
    /// Plan name, e.g. `"pro"` or `"pro-lifetime"`. Drives [`grants_pro`].
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
    /// Opaque license id (the DB row id). v2+.
    #[serde(rename = "licenseId", default, skip_serializing_if = "Option::is_none")]
    pub license_id: Option<String>,
    /// SHA-256 (hex) of the buyer's normalized email — binds the certificate to
    /// the buyer without embedding the raw email. v2+.
    #[serde(rename = "emailHash", default, skip_serializing_if = "Option::is_none")]
    pub email_hash: Option<String>,
    /// SHA-256 (hex) of the license code — binds the certificate to its code. v2+.
    #[serde(rename = "codeHash", default, skip_serializing_if = "Option::is_none")]
    pub code_hash: Option<String>,
    /// Certificate schema version (2 = short-code + server activation). Absent on
    /// legacy v1 tokens.
    #[serde(
        rename = "schemaVersion",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub schema_version: Option<u32>,
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

/// Decode the embedded verification key, failing closed when unconfigured.
fn embedded_verifying_key() -> Result<VerifyingKey> {
    verifying_key_configured(LICENSE_PUBLIC_KEY_B64)
}

/// Decode `key_b64` into a verifying key, but fail closed when it is the
/// unconfigured sentinel or empty (detected by value, not by hoping it fails to
/// decode). Parameterised so the fail-closed path is unit-testable.
fn verifying_key_configured(key_b64: &str) -> Result<VerifyingKey> {
    if !is_configured_key(key_b64) {
        return Err(Error::License(
            "license public key is not configured in this build (still the placeholder); \
             set LICENSE_PUBLIC_KEY_B64 to your production public key"
                .into(),
        ));
    }
    verifying_key_from_b64(key_b64).map_err(|_| {
        Error::License(
            "configured LICENSE_PUBLIC_KEY_B64 is not a valid 32-byte Ed25519 public key".into(),
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
    // Strip ALL whitespace (not just the ends) so a certificate that got
    // line-wrapped by an email client still parses.
    let license: String = license.chars().filter(|c| !c.is_whitespace()).collect();
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

/// Whether these (already signature-verified) claims grant the Pro tier.
///
/// Only Pro plans unlock Pro — a validly-signed token for some other plan
/// (free, trial, …) must NOT flip the app into Pro. The Paddle webhook issues
/// `"pro-lifetime"` for the one-time license and `"pro"` for the annual plan.
pub fn grants_pro(claims: &LicenseClaims) -> bool {
    matches!(claims.plan.as_str(), "pro" | "pro-lifetime")
}

/// Whether a stored license token currently entitles this machine to Pro:
/// signature valid against the configured key, correct product, hardware/expiry
/// satisfied, **and** a Pro-tier plan. This is the single source of truth the
/// app should consult on every load instead of trusting a persisted boolean —
/// so editing the local state file to flip `isPro` grants nothing.
pub fn is_pro_active(license_key: Option<&str>, hardware_id: &str) -> bool {
    match license_key {
        Some(k) if !k.trim().is_empty() => verify_license(k, hardware_id)
            .map(|claims| grants_pro(&claims))
            .unwrap_or(false),
        _ => false,
    }
}

// ============================================================================
// Short license codes (what buyers see) + normalization helpers.
//
// A license *code* (e.g. `ENVY-K7M4-9Q2P-D8X6-R3TA`) is a human-friendly lookup
// key stored in the license DB. It is NOT a signed token — the activation server
// exchanges a valid code (+ email) for the signed certificate above, which the
// app then verifies offline. Keeping these separate lets the visible key be
// short and pretty while verification stays cryptographic.
// ============================================================================

/// The visible prefix on every license code.
pub const LICENSE_CODE_PREFIX: &str = "ENVY";

/// Normalize a buyer email for storage/comparison: drop all whitespace and
/// lowercase. Deliberately conservative — no Gmail dot/plus tricks — so a
/// legitimately distinct address is never merged.
pub fn normalize_email(email: &str) -> String {
    email
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .to_lowercase()
}

/// Canonicalize a user-entered license code to the stored form
/// `ENVY-XXXX-XXXX-XXXX-…`: uppercase, drop everything that isn't a letter or
/// digit, then regroup in fours. Tolerant of missing/extra hyphens, spaces, and
/// case, and idempotent, so lookups match regardless of how the buyer typed it.
pub fn normalize_license_code(input: &str) -> String {
    let cleaned: String = input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_uppercase())
        .collect();
    let body = cleaned
        .strip_prefix(LICENSE_CODE_PREFIX)
        .unwrap_or(&cleaned);
    let mut out = String::from(LICENSE_CODE_PREFIX);
    for (i, ch) in body.chars().enumerate() {
        if i % 4 == 0 {
            out.push('-');
        }
        out.push(ch);
    }
    out
}

/// Lowercase-hex SHA-256 of the input — used to bind a certificate to its
/// email/code without embedding the raw values.
pub fn sha256_hex(input: &str) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(input.as_bytes());
    let mut s = String::with_capacity(64);
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// Alphabet for license codes: A–Z and 2–9 with the ambiguous glyphs
/// (I, L, O, 0, 1) removed so codes are safe to read aloud and retype.
#[cfg(feature = "issuer")]
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789";

/// Number of random body characters (excludes the `ENVY` prefix). 16 symbols
/// over a 30-char alphabet ≈ 78 bits of entropy; the DB `unique` constraint is
/// the ultimate collision guard.
#[cfg(feature = "issuer")]
const CODE_BODY_LEN: usize = 16;

/// Generate a fresh license code, e.g. `ENVY-K7M4-9Q2P-D8X6-R3TA`, using OS
/// randomness with rejection sampling (no modulo bias). Issuer-only — the app
/// never generates codes.
#[cfg(feature = "issuer")]
pub fn generate_license_code() -> String {
    use rand::RngCore;
    let mut rng = rand::rngs::OsRng;
    let n = CODE_ALPHABET.len() as u32;
    // Reject the top of the u32 range that doesn't divide evenly, so every
    // symbol is equally likely.
    let limit = u32::MAX - (u32::MAX % n);
    let mut body = String::with_capacity(CODE_BODY_LEN);
    while body.len() < CODE_BODY_LEN {
        let r = rng.next_u32();
        if r >= limit {
            continue;
        }
        body.push(CODE_ALPHABET[(r % n) as usize] as char);
    }
    let mut out = String::from(LICENSE_CODE_PREFIX);
    for (i, ch) in body.chars().enumerate() {
        if i % 4 == 0 {
            out.push('-');
        }
        out.push(ch);
    }
    out
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
            ..Default::default()
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
    fn unconfigured_key_fails_closed() {
        // The placeholder sentinel and empty are "unconfigured"; any real key is
        // "configured". The check is by value so a decodable-but-placeholder key
        // can't slip through.
        assert!(!is_configured_key(UNCONFIGURED_PUBLIC_KEY_B64));
        assert!(!is_configured_key(""));
        assert!(!is_configured_key("   "));
        assert!(is_configured_key(LICENSE_PUBLIC_KEY_B64));

        // An unconfigured key rejects verification with the *not-configured*
        // error — not a mere signature mismatch — so it can never accidentally
        // accept a license.
        let err = verifying_key_configured(UNCONFIGURED_PUBLIC_KEY_B64)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("not configured"),
            "expected the fail-closed 'not configured' error, got: {err}"
        );
        // The real configured key decodes fine through the same path.
        assert!(verifying_key_configured(LICENSE_PUBLIC_KEY_B64).is_ok());
    }

    #[test]
    fn shipped_build_has_a_real_license_key() {
        // Guards against ever shipping the placeholder (which would reject every
        // real license) or a compromised/empty key.
        assert!(
            is_license_key_configured(),
            "shipped build must set LICENSE_PUBLIC_KEY_B64 to a real production key"
        );
    }

    #[test]
    fn grants_pro_only_for_pro_plans() {
        let mut c = pro_claims();
        c.plan = "pro".into();
        assert!(grants_pro(&c));
        c.plan = "pro-lifetime".into();
        assert!(grants_pro(&c));
        c.plan = "free".into();
        assert!(!grants_pro(&c));
        c.plan = "trial".into();
        assert!(!grants_pro(&c));
    }

    #[test]
    fn is_pro_active_rejects_missing_or_blank_key() {
        assert!(!is_pro_active(None, "machine-A"));
        assert!(!is_pro_active(Some(""), "machine-A"));
        assert!(!is_pro_active(Some("   "), "machine-A"));
        assert!(!is_pro_active(Some("garbage.token"), "machine-A"));
    }

    #[test]
    fn non_pro_plan_does_not_grant_pro_even_when_signed() {
        // A validly-signed token for a non-pro plan must verify cryptographically
        // but must NOT grant Pro (checked via grants_pro, key-agnostic here).
        let sk = test_signing_key();
        let vk = sk.verifying_key();
        let mut claims = pro_claims();
        claims.plan = "free".into();
        let lic = issue(&sk, &claims);
        let verified = verify_license_with_key(&lic, "machine-A", &vk).unwrap();
        assert!(!grants_pro(&verified));
    }

    #[test]
    fn normalize_email_lowercases_and_strips_whitespace() {
        assert_eq!(
            normalize_email("  Foo.Bar@Example.COM \n"),
            "foo.bar@example.com"
        );
        assert_eq!(normalize_email("a b\t@ c.com"), "ab@c.com");
        // conservative: does NOT strip Gmail dots / plus tags
        assert_eq!(normalize_email("a.b+tag@gmail.com"), "a.b+tag@gmail.com");
    }

    #[test]
    fn normalize_code_is_tolerant_and_idempotent() {
        let canon = "ENVY-K7M4-9Q2P-D8X6-R3TA";
        assert_eq!(normalize_license_code("envy k7m4 9q2p d8x6 r3ta"), canon);
        assert_eq!(normalize_license_code("ENVYK7M49Q2PD8X6R3TA"), canon);
        assert_eq!(normalize_license_code("k7m4-9q2p-d8x6-r3ta"), canon); // no prefix typed
        assert_eq!(normalize_license_code(" envy_k7m4_9q2p_d8x6_r3ta "), canon);
        assert_eq!(normalize_license_code(canon), canon); // idempotent
    }

    #[test]
    fn sha256_hex_matches_known_vector() {
        // SHA-256("") — a well-known constant.
        assert_eq!(
            sha256_hex(""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn certificate_verifies_despite_email_line_wrapping() {
        let sk = test_signing_key();
        let vk = sk.verifying_key();
        let lic = issue(&sk, &pro_claims());
        // Simulate an email client wrapping the long certificate across lines.
        let mid = lic.len() / 2;
        let wrapped = format!("{}\r\n  {}", &lic[..mid], &lic[mid..]);
        let claims = verify_license_with_key(&wrapped, "machine-A", &vk).unwrap();
        assert_eq!(claims.plan, "pro");
    }

    #[cfg(feature = "issuer")]
    #[test]
    fn generated_code_shape_alphabet_and_uniqueness() {
        let code = generate_license_code();
        assert!(code.starts_with("ENVY-"), "code: {code}");
        assert_eq!(code.len(), 24, "ENVY + 4×4 groups + hyphens = 24: {code}");
        let body: String = code
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .skip(4)
            .collect();
        assert_eq!(body.len(), CODE_BODY_LEN);
        for c in body.chars() {
            assert!(
                CODE_ALPHABET.contains(&(c as u8)),
                "unexpected char {c} in {code}"
            );
        }
        // A generated code normalizes back to itself.
        assert_eq!(normalize_license_code(&code), code);
        // Two codes are astronomically unlikely to collide.
        assert_ne!(generate_license_code(), generate_license_code());
    }

    #[cfg(feature = "issuer")]
    #[test]
    fn certificate_v2_fields_roundtrip_and_grant_pro() {
        use ed25519_dalek::SigningKey;
        let seed = [21u8; 32];
        let vk = SigningKey::from_bytes(&seed).verifying_key();
        let claims = LicenseClaims {
            product: PRODUCT.into(),
            plan: "pro-lifetime".into(),
            hardware_id: None,
            issued_at: "2026-07-09T00:00:00Z".into(),
            expires_at: None,
            features: vec!["unlimited_projects".into(), "mcp".into()],
            license_id: Some("11111111-1111-1111-1111-111111111111".into()),
            email_hash: Some(sha256_hex(&normalize_email("Buyer@Example.com"))),
            code_hash: Some(sha256_hex("ENVY-K7M4-9Q2P-D8X6-R3TA")),
            schema_version: Some(2),
        };
        let cert = issue_license(&seed, &claims).unwrap();
        let verified = verify_license_with_key(&cert, "any-machine", &vk).unwrap();
        assert_eq!(verified, claims);
        assert!(grants_pro(&verified));
        assert_eq!(verified.schema_version, Some(2));
    }
}
