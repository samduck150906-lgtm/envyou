//! Persistence of [`EnvYouLocalState`] to the encrypted `enc_state.json` file.
//!
//! All data lives only on the local machine (spec §1.2 "Zero Cloud").

use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;

use crate::core::crypto::{self, MasterKey};
use crate::core::model::EnvYouLocalState;
use crate::error::{Error, Result};

/// Filename for the encrypted state, as named in the spec (§7).
pub const STATE_FILE: &str = "enc_state.json";

/// A handle to the encrypted store at a fixed path, guarded by a master key.
pub struct Store {
    path: PathBuf,
    key: MasterKey,
}

impl Store {
    pub fn new(path: impl Into<PathBuf>, key: MasterKey) -> Self {
        Self {
            path: path.into(),
            key,
        }
    }

    /// Open the store at the default OS application-data location, deriving the
    /// master key from machine-specific material.
    pub fn open_default() -> Result<Self> {
        let dir = default_data_dir()?;
        fs::create_dir_all(&dir)?;
        let key = MasterKey::derive(machine_id().as_bytes());
        Ok(Self::new(dir.join(STATE_FILE), key))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Load and decrypt state, returning [`EnvYouLocalState::default`] if the
    /// file does not yet exist.
    pub fn load(&self) -> Result<EnvYouLocalState> {
        if !self.path.exists() {
            return Ok(EnvYouLocalState::default());
        }
        let envelope = fs::read_to_string(&self.path)?;
        let plaintext = crypto::decrypt(&self.key, &envelope)?;
        let state: EnvYouLocalState = serde_json::from_slice(&plaintext)?;
        Ok(state)
    }

    /// Encrypt and atomically persist state.
    pub fn save(&self, state: &EnvYouLocalState) -> Result<()> {
        let plaintext = serde_json::to_vec(state)?;
        let envelope = crypto::encrypt(&self.key, &plaintext)?;
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Write to a temp file then rename for atomicity / crash safety.
        let tmp = self.path.with_extension("json.tmp");
        fs::write(&tmp, envelope.as_bytes())?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }
}

/// Default per-OS data directory for envyou.
pub fn default_data_dir() -> Result<PathBuf> {
    ProjectDirs::from("com", "envyou", "envyou")
        .map(|d| d.data_dir().to_path_buf())
        .ok_or_else(|| Error::Config("could not resolve OS data directory".into()))
}

/// Best-effort stable machine identifier used as key-derivation material.
///
/// Tries the platform machine id, then the hostname, then a constant fallback
/// (so the app remains usable even on locked-down systems — at the cost of a
/// non-machine-bound key in that edge case).
pub fn machine_id() -> String {
    // Linux / many Unixes
    for p in ["/etc/machine-id", "/var/lib/dbus/machine-id"] {
        if let Ok(s) = fs::read_to_string(p) {
            let t = s.trim();
            if !t.is_empty() {
                return t.to_string();
            }
        }
    }
    // Fallback: hostname env, then a constant.
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "envyou-default-machine".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{EnvVariable, ProjectItem};

    #[test]
    fn save_then_load_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join(STATE_FILE), MasterKey::derive(b"test"));

        let mut state = EnvYouLocalState::default();
        let mut p = ProjectItem::new("api", "#000080", "2026-06-02");
        p.variables.push(EnvVariable {
            key: "DATABASE_URL".into(),
            value: "postgres://localhost/db".into(),
            comment: Some("primary db".into()),
            is_masked: true,
        });
        state.projects.push(p);

        store.save(&state).unwrap();
        let loaded = store.load().unwrap();
        assert_eq!(state, loaded);
    }

    #[test]
    fn on_disk_file_is_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join(STATE_FILE), MasterKey::derive(b"test"));
        let mut state = EnvYouLocalState::default();
        let mut p = ProjectItem::new("api", "#000080", "2026-06-02");
        p.variables.push(EnvVariable {
            key: "SECRET".into(),
            value: "plaintext-should-not-appear".into(),
            comment: None,
            is_masked: true,
        });
        state.projects.push(p);
        store.save(&state).unwrap();

        let raw = fs::read_to_string(store.path()).unwrap();
        assert!(!raw.contains("plaintext-should-not-appear"));
        assert!(raw.contains("AES-256-GCM"));
    }

    #[test]
    fn missing_file_yields_default() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join(STATE_FILE), MasterKey::derive(b"k"));
        assert_eq!(store.load().unwrap(), EnvYouLocalState::default());
    }
}
