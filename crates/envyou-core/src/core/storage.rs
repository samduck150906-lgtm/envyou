//! Persistence of [`EnvYouLocalState`] to the encrypted `enc_state.json` file.
//!
//! All data lives only on the local machine (spec §1.2 "Zero Cloud").

use std::fs;
use std::path::{Path, PathBuf};

use directories::ProjectDirs;
use rand::RngCore;

use crate::core::crypto::{self, MasterKey};
use crate::core::model::EnvYouLocalState;
use crate::error::{Error, Result};

/// Filename for the encrypted state, as named in the spec (§7).
pub const STATE_FILE: &str = "enc_state.json";

/// Filename for the persisted random device secret used as the last-resort
/// key-derivation material (see [`machine_id`]).
pub const DEVICE_SECRET_FILE: &str = "device_secret";

/// How a [`Store`] derives its encryption key.
enum KeySource {
    /// Device-bound key (SHA-256 over machine material). No user password; the
    /// file is protected against copying to another machine only.
    Device(MasterKey),
    /// User master password (Argon2id). The password is held in memory for the
    /// lifetime of the `Store` and never written to disk.
    Password(String),
}

/// A handle to the encrypted store at a fixed path, guarded by a key source.
pub struct Store {
    path: PathBuf,
    key: KeySource,
}

impl Store {
    /// Construct a device-bound store from an explicit key (used in tests and
    /// by [`Store::open_default`]).
    pub fn new(path: impl Into<PathBuf>, key: MasterKey) -> Self {
        Self {
            path: path.into(),
            key: KeySource::Device(key),
        }
    }

    /// Construct a password-protected store. State is sealed with Argon2id +
    /// AES-256-GCM (v2 envelope). The `password` is kept only in memory.
    pub fn with_password(path: impl Into<PathBuf>, password: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            key: KeySource::Password(password.into()),
        }
    }

    /// Open the store at the default OS application-data location, deriving the
    /// master key from machine-specific material (device-bound, no password).
    pub fn open_default() -> Result<Self> {
        let dir = default_data_dir()?;
        fs::create_dir_all(&dir)?;
        let key = MasterKey::derive(machine_id().as_bytes());
        Ok(Self::new(dir.join(STATE_FILE), key))
    }

    /// Open the default-location store unlocked with a user master password.
    pub fn open_default_with_password(password: impl Into<String>) -> Result<Self> {
        let dir = default_data_dir()?;
        fs::create_dir_all(&dir)?;
        Ok(Self::with_password(dir.join(STATE_FILE), password))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Whether the on-disk vault at this path is password-protected (v2). Useful
    /// for the UI to decide between a "set password" and an "unlock" screen.
    pub fn is_locked_with_password(&self) -> Result<bool> {
        if !self.path.exists() {
            return Ok(false);
        }
        let envelope = fs::read_to_string(&self.path)?;
        crypto::is_password_protected(&envelope)
    }

    /// Load and decrypt state, returning [`EnvYouLocalState::default`] if the
    /// file does not yet exist.
    pub fn load(&self) -> Result<EnvYouLocalState> {
        if !self.path.exists() {
            return Ok(EnvYouLocalState::default());
        }
        let envelope = fs::read_to_string(&self.path)?;
        let plaintext = match &self.key {
            KeySource::Device(key) => {
                if crypto::is_password_protected(&envelope)? {
                    return Err(Error::Crypto(
                        "vault is password-protected; unlock with a master password".into(),
                    ));
                }
                crypto::decrypt(key, &envelope)?
            }
            KeySource::Password(pw) => crypto::decrypt_with_password(pw.as_bytes(), &envelope)?,
        };
        let state: EnvYouLocalState = serde_json::from_slice(&plaintext)?;
        Ok(state)
    }

    /// Encrypt and atomically persist state.
    pub fn save(&self, state: &EnvYouLocalState) -> Result<()> {
        let plaintext = serde_json::to_vec(state)?;
        let envelope = match &self.key {
            KeySource::Device(key) => crypto::encrypt(key, &plaintext)?,
            KeySource::Password(pw) => crypto::encrypt_with_password(pw.as_bytes(), &plaintext)?,
        };
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        // Write to a uniquely-named temp file then rename for atomicity / crash
        // safety. The temp name includes the PID and a random suffix so two
        // concurrent `save()` calls on the same path (e.g. the GUI and a
        // `--mcp` process both writing `enc_state.json`) never share a temp
        // file — with a fixed name, one writer's content could land in the
        // other's rename, silently discarding whichever write lost the race.
        let mut nonce = [0u8; 8];
        rand::thread_rng().fill_bytes(&mut nonce);
        let nonce_hex: String = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let tmp =
            self.path
                .with_extension(format!("json.tmp.{}.{}", std::process::id(), nonce_hex));
        fs::write(&tmp, envelope.as_bytes())?;
        fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    /// One-time migration: read this (device-bound) vault and re-seal it under a
    /// user master password, returning the new password-protected [`Store`].
    ///
    /// Reads with the current key source, so it works whether the vault is
    /// empty (fresh default state) or already populated — no data is lost.
    ///
    /// Takes `&self` (rather than consuming it) so that if this fails partway
    /// (e.g. the write in `save` errors), the caller still holds a live,
    /// still-valid `Store` instead of having already discarded it.
    pub fn migrate_to_password(&self, password: impl Into<String>) -> Result<Store> {
        let state = self.load()?;
        let migrated = Store::with_password(self.path.clone(), password);
        migrated.save(&state)?;
        Ok(migrated)
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
/// Tries the platform machine id, then the hostname, then a **persisted random
/// device secret** generated once and stored in the app data directory. The
/// previous implementation fell back to a hard-coded constant, which produced a
/// fully predictable (and therefore useless) encryption key on locked-down
/// systems; the random device secret keeps the file bound to *this* install
/// without that weakness.
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
    // Hostname, if the environment exposes one.
    if let Ok(h) = std::env::var("HOSTNAME").or_else(|_| std::env::var("COMPUTERNAME")) {
        if !h.trim().is_empty() {
            return h;
        }
    }
    // Last resort: a random secret persisted once per install. Only if even that
    // cannot be written do we use a constant (a truly locked-down/read-only
    // environment) — and that case is unavoidable without a user password.
    persisted_device_secret().unwrap_or_else(|| "envyou-default-machine".to_string())
}

/// Read (or lazily create) a random 32-byte device secret stored as hex in the
/// app data directory. Returns `None` if the data directory is unavailable or
/// unwritable.
fn persisted_device_secret() -> Option<String> {
    let dir = default_data_dir().ok()?;
    let path = dir.join(DEVICE_SECRET_FILE);
    if let Ok(existing) = fs::read_to_string(&path) {
        let t = existing.trim();
        if !t.is_empty() {
            return Some(t.to_string());
        }
    }
    fs::create_dir_all(&dir).ok()?;
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let secret: String = bytes.iter().map(|b| format!("{b:02x}")).collect();

    // Create exclusively (fails if the file already exists) so two processes
    // racing on first launch can't each "win" with a different secret — the
    // loser would otherwise derive a `MasterKey` from a secret that never
    // makes it to disk, and fail to decrypt its own save on the next launch.
    use std::io::Write;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
    {
        Ok(mut f) => {
            f.write_all(secret.as_bytes()).ok()?;
            Some(secret)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            // Another process created it first between our read and write —
            // use what it actually persisted, not our discarded secret.
            let existing = fs::read_to_string(&path).ok()?;
            let t = existing.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.to_string())
            }
        }
        Err(_) => None,
    }
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

    #[test]
    fn settings_changes_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join(STATE_FILE), MasterKey::derive(b"test"));

        let mut state = EnvYouLocalState::default();
        state.settings.always_on_top = false;
        state.settings.mask_sensitive_data = false;
        state.settings.global_hotkey = "Ctrl+Alt+V".into();
        store.save(&state).unwrap();

        let loaded = store.load().unwrap();
        assert_eq!(loaded.settings, state.settings);
        assert!(!loaded.settings.always_on_top);
        assert_eq!(loaded.settings.global_hotkey, "Ctrl+Alt+V");
    }

    #[test]
    fn password_store_round_trips_and_stays_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STATE_FILE);
        let store = Store::with_password(&path, "correct horse battery staple");

        let mut state = EnvYouLocalState::default();
        let mut p = ProjectItem::new("api", "#000080", "now");
        p.variables.push(EnvVariable {
            key: "SECRET".into(),
            value: "must-not-appear-on-disk".into(),
            comment: None,
            is_masked: true,
        });
        state.projects.push(p);
        store.save(&state).unwrap();

        let raw = fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("must-not-appear-on-disk"));
        assert!(
            raw.contains("argon2id"),
            "v2 envelope should record the kdf"
        );
        assert!(store.is_locked_with_password().unwrap());
        assert_eq!(store.load().unwrap(), state);
    }

    #[test]
    fn wrong_master_password_cannot_load() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STATE_FILE);
        Store::with_password(&path, "right")
            .save(&EnvYouLocalState::default())
            .unwrap();

        let wrong = Store::with_password(&path, "wrong");
        assert!(
            wrong.load().is_err(),
            "wrong password must not decrypt the vault"
        );
    }

    #[test]
    fn device_store_refuses_password_protected_vault() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STATE_FILE);
        Store::with_password(&path, "pw")
            .save(&EnvYouLocalState::default())
            .unwrap();

        // A device-bound Store must not silently succeed on a password vault;
        // it should signal that a password unlock is required.
        let device = Store::new(&path, MasterKey::derive(b"machine"));
        let err = device.load().unwrap_err();
        assert!(err.to_string().contains("password-protected"));
    }

    #[test]
    fn migration_from_device_to_password_preserves_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(STATE_FILE);

        // Start as a device-bound vault with real data.
        let device = Store::new(&path, MasterKey::derive(b"machine"));
        let mut state = EnvYouLocalState::default();
        state
            .projects
            .push(ProjectItem::new("api", "#000080", "now"));
        device.save(&state).unwrap();

        // Migrate to a master password; data must survive and now be v2.
        let secured = device.migrate_to_password("hunter2").unwrap();
        assert!(secured.is_locked_with_password().unwrap());
        assert_eq!(secured.load().unwrap(), state);

        // Re-opening with the same password works; a fresh device store does not.
        assert_eq!(
            Store::with_password(&path, "hunter2").load().unwrap(),
            state
        );
        assert!(Store::new(&path, MasterKey::derive(b"machine"))
            .load()
            .is_err());
    }
}
