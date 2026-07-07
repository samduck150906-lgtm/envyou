//! AES-256-GCM encryption for the local state file (spec §2.1 Storage Engine).
//!
//! The on-disk format is a small JSON envelope so the file is still a valid
//! `.json` document (as named in the spec) while its payload stays encrypted:
//!
//! ```json
//! { "v": 1, "alg": "AES-256-GCM", "nonce": "<base64>", "ciphertext": "<base64>" }
//! ```

use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::Argon2;
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// Application-wide salt mixed into key derivation. Not a secret; it only
/// domain-separates envyou keys from other AES-256-GCM usage.
const KEY_SALT: &[u8] = b"envyou::aes256gcm::v1";

/// KDF identifier stored in a v2 envelope when the key was derived from a
/// user master password via Argon2id.
pub const KDF_ARGON2ID: &str = "argon2id";

/// Length of the random per-file salt for the Argon2id KDF.
pub const ARGON2_SALT_LEN: usize = 16;

/// A 256-bit symmetric key.
#[derive(Clone)]
pub struct MasterKey([u8; 32]);

impl MasterKey {
    /// Build a key directly from 32 raw bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Derive a key from machine-bound secret material using SHA-256 over a
    /// fixed salt. This is the **device-bound** key path (no user password); it
    /// protects the on-disk file against casual copying to another machine but
    /// is only as strong as the machine secret. For password-protected vaults
    /// use [`MasterKey::derive_argon2id`] instead.
    pub fn derive(secret_material: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(KEY_SALT);
        hasher.update(secret_material);
        let digest = hasher.finalize();
        let mut key = [0u8; 32];
        key.copy_from_slice(&digest);
        Self(key)
    }

    /// Derive a key from a user **master password** using Argon2id with the
    /// given random per-vault `salt`. Argon2id is memory-hard, so it resists
    /// GPU/ASIC brute force in a way SHA-256 cannot — this is the path a
    /// password-protected vault should use.
    ///
    /// The `salt` must be stored alongside the ciphertext (it is not secret) so
    /// the same key can be re-derived on unlock; see [`encrypt_with_password`].
    pub fn derive_argon2id(password: &[u8], salt: &[u8]) -> Result<Self> {
        let mut key = [0u8; 32];
        Argon2::default()
            .hash_password_into(password, salt, &mut key)
            .map_err(|e| Error::Crypto(format!("argon2id key derivation failed: {e}")))?;
        Ok(Self(key))
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new_from_slice(&self.0).expect("32-byte key is always valid for AES-256")
    }
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    v: u8,
    alg: String,
    /// Present only in v2 password-protected envelopes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    kdf: Option<String>,
    /// Base64 of the Argon2id salt; present only in v2 password envelopes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    salt: Option<String>,
    nonce: String,
    ciphertext: String,
}

/// Whether this envelope is password-protected (v2 / Argon2id). Lets the storage
/// layer decide whether it needs a master password before unlocking, without
/// decrypting first.
pub fn is_password_protected(envelope_json: &str) -> bool {
    serde_json::from_str::<Envelope>(envelope_json)
        .ok()
        .and_then(|e| e.kdf)
        .as_deref()
        == Some(KDF_ARGON2ID)
}

fn seal(
    key: &MasterKey,
    plaintext: &[u8],
    kdf: Option<String>,
    salt: Option<String>,
) -> Result<String> {
    let cipher = key.cipher();
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| Error::Crypto("encryption failed".into()))?;

    let env = Envelope {
        v: if kdf.is_some() { 2 } else { 1 },
        alg: "AES-256-GCM".into(),
        kdf,
        salt,
        nonce: B64.encode(nonce_bytes),
        ciphertext: B64.encode(ciphertext),
    };
    serde_json::to_string_pretty(&env).map_err(Error::from)
}

fn open(key: &MasterKey, env: &Envelope) -> Result<Vec<u8>> {
    if env.alg != "AES-256-GCM" {
        return Err(Error::Crypto(format!("unsupported algorithm: {}", env.alg)));
    }
    let nonce_bytes = B64
        .decode(env.nonce.as_bytes())
        .map_err(|_| Error::Crypto("bad nonce encoding".into()))?;
    if nonce_bytes.len() != 12 {
        return Err(Error::Crypto("invalid nonce length".into()));
    }
    let ciphertext = B64
        .decode(env.ciphertext.as_bytes())
        .map_err(|_| Error::Crypto("bad ciphertext encoding".into()))?;

    let cipher = key.cipher();
    let nonce = Nonce::from_slice(&nonce_bytes);
    cipher
        .decrypt(nonce, ciphertext.as_ref())
        .map_err(|_| Error::Crypto("decryption failed (wrong key or tampered data)".into()))
}

fn parse_envelope(envelope_json: &str) -> Result<Envelope> {
    serde_json::from_str(envelope_json)
        .map_err(|_| Error::Crypto("malformed encrypted envelope".into()))
}

/// Encrypt `plaintext` with a device-bound key and return the v1 JSON envelope.
pub fn encrypt(key: &MasterKey, plaintext: &[u8]) -> Result<String> {
    seal(key, plaintext, None, None)
}

/// Decrypt a JSON envelope produced by [`encrypt`] (v1 device-bound key).
pub fn decrypt(key: &MasterKey, envelope_json: &str) -> Result<Vec<u8>> {
    let env = parse_envelope(envelope_json)?;
    open(key, &env)
}

/// Encrypt `plaintext` under a **master password**: generate a fresh random
/// Argon2id salt, derive the key, and embed the salt + KDF id in a v2 envelope
/// so the vault can be unlocked later with the same password.
pub fn encrypt_with_password(password: &[u8], plaintext: &[u8]) -> Result<String> {
    let mut salt = [0u8; ARGON2_SALT_LEN];
    rand::thread_rng().fill_bytes(&mut salt);
    let key = MasterKey::derive_argon2id(password, &salt)?;
    seal(
        &key,
        plaintext,
        Some(KDF_ARGON2ID.to_string()),
        Some(B64.encode(salt)),
    )
}

/// Decrypt a v2 password-protected envelope. Re-derives the Argon2id key from
/// `password` and the embedded salt; a wrong password fails authentication.
pub fn decrypt_with_password(password: &[u8], envelope_json: &str) -> Result<Vec<u8>> {
    let env = parse_envelope(envelope_json)?;
    let kdf = env
        .kdf
        .as_deref()
        .ok_or_else(|| Error::Crypto("envelope is not password-protected".into()))?;
    if kdf != KDF_ARGON2ID {
        return Err(Error::Crypto(format!("unsupported kdf: {kdf}")));
    }
    let salt = env
        .salt
        .as_deref()
        .ok_or_else(|| Error::Crypto("password envelope missing salt".into()))?;
    let salt = B64
        .decode(salt.as_bytes())
        .map_err(|_| Error::Crypto("bad salt encoding".into()))?;
    let key = MasterKey::derive_argon2id(password, &salt)?;
    open(&key, &env)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let key = MasterKey::derive(b"machine-id-1234");
        let msg = b"DATABASE_URL=postgres://localhost/db";
        let env = encrypt(&key, msg).unwrap();
        // Envelope must not leak plaintext.
        assert!(!env.contains("postgres"));
        let out = decrypt(&key, &env).unwrap();
        assert_eq!(out, msg);
    }

    #[test]
    fn wrong_key_fails() {
        let k1 = MasterKey::derive(b"machine-A");
        let k2 = MasterKey::derive(b"machine-B");
        let env = encrypt(&k1, b"secret").unwrap();
        assert!(decrypt(&k2, &env).is_err());
    }

    #[test]
    fn tampered_ciphertext_fails() {
        let key = MasterKey::derive(b"m");
        let env = encrypt(&key, b"secret").unwrap();
        let mut v: serde_json::Value = serde_json::from_str(&env).unwrap();
        v["ciphertext"] = serde_json::Value::String(B64.encode(b"tampered-bytes-here!!"));
        assert!(decrypt(&key, &v.to_string()).is_err());
    }

    #[test]
    fn nonces_differ_between_encryptions() {
        let key = MasterKey::derive(b"m");
        let a = encrypt(&key, b"same").unwrap();
        let b = encrypt(&key, b"same").unwrap();
        assert_ne!(a, b, "GCM nonce must be random per-encryption");
    }

    #[test]
    fn password_round_trip() {
        let msg = b"STRIPE_KEY=sk_live_shouldstayhidden";
        let env = encrypt_with_password(b"correct horse battery staple", msg).unwrap();
        assert!(!env.contains("sk_live"), "envelope must not leak plaintext");
        assert!(is_password_protected(&env));
        let out = decrypt_with_password(b"correct horse battery staple", &env).unwrap();
        assert_eq!(out, msg);
    }

    #[test]
    fn wrong_password_fails() {
        let env = encrypt_with_password(b"right-password", b"secret").unwrap();
        assert!(
            decrypt_with_password(b"wrong-password", &env).is_err(),
            "a wrong master password must not decrypt the vault"
        );
    }

    #[test]
    fn password_salt_is_random_per_encryption() {
        let a = encrypt_with_password(b"pw", b"same").unwrap();
        let b = encrypt_with_password(b"pw", b"same").unwrap();
        // Different random salts (and nonces) => different envelopes for the
        // same password + plaintext.
        assert_ne!(a, b);
        // Both still decrypt with the same password.
        assert_eq!(decrypt_with_password(b"pw", &a).unwrap(), b"same");
        assert_eq!(decrypt_with_password(b"pw", &b).unwrap(), b"same");
    }

    #[test]
    fn v1_envelope_is_not_flagged_password_protected() {
        let key = MasterKey::derive(b"machine");
        let env = encrypt(&key, b"data").unwrap();
        assert!(!is_password_protected(&env));
        // And a device-bound (v1) envelope cannot be opened via the password path.
        assert!(decrypt_with_password(b"anything", &env).is_err());
    }

    #[test]
    fn argon2_derivation_is_deterministic_for_same_salt() {
        let salt = [3u8; ARGON2_SALT_LEN];
        let k1 = MasterKey::derive_argon2id(b"pw", &salt).unwrap();
        let k2 = MasterKey::derive_argon2id(b"pw", &salt).unwrap();
        // Same password + salt must yield an identical key (needed for unlock).
        let e = encrypt(&k1, b"x").unwrap();
        assert_eq!(decrypt(&k2, &e).unwrap(), b"x");
    }
}
