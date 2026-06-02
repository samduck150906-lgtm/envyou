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
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

/// Application-wide salt mixed into key derivation. Not a secret; it only
/// domain-separates envyou keys from other AES-256-GCM usage.
const KEY_SALT: &[u8] = b"envyou::aes256gcm::v1";

/// A 256-bit symmetric key.
#[derive(Clone)]
pub struct MasterKey([u8; 32]);

impl MasterKey {
    /// Build a key directly from 32 raw bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Derive a key from arbitrary secret material (e.g. machine id +
    /// passphrase) using SHA-256 over a fixed salt. This keeps the MVP free of
    /// a heavy KDF dependency while still binding the key to per-machine
    /// material; a future Pro build can swap in Argon2 transparently.
    pub fn derive(secret_material: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(KEY_SALT);
        hasher.update(secret_material);
        let digest = hasher.finalize();
        let mut key = [0u8; 32];
        key.copy_from_slice(&digest);
        Self(key)
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new_from_slice(&self.0).expect("32-byte key is always valid for AES-256")
    }
}

#[derive(Serialize, Deserialize)]
struct Envelope {
    v: u8,
    alg: String,
    nonce: String,
    ciphertext: String,
}

/// Encrypt `plaintext` and return the JSON envelope string.
pub fn encrypt(key: &MasterKey, plaintext: &[u8]) -> Result<String> {
    let cipher = key.cipher();
    let mut nonce_bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|_| Error::Crypto("encryption failed".into()))?;

    let env = Envelope {
        v: 1,
        alg: "AES-256-GCM".into(),
        nonce: B64.encode(nonce_bytes),
        ciphertext: B64.encode(ciphertext),
    };
    serde_json::to_string_pretty(&env).map_err(Error::from)
}

/// Decrypt a JSON envelope produced by [`encrypt`].
pub fn decrypt(key: &MasterKey, envelope_json: &str) -> Result<Vec<u8>> {
    let env: Envelope = serde_json::from_str(envelope_json)
        .map_err(|_| Error::Crypto("malformed encrypted envelope".into()))?;
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
}
