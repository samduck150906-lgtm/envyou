//! Tauri commands invoked from the retro frontend. All operate on the
//! encrypted local [`Store`]; nothing ever leaves the machine.

use std::sync::Mutex;

use envyou_core::core::model::{EnvVariable, EnvYouLocalState, ProjectItem, Settings};
use envyou_core::core::storage::{machine_id, Store};
use envyou_core::core::{crypto, license, storage};
use serde::Serialize;
use tauri::State;

use crate::util::now_iso8601;

/// Tauri-managed application state.
///
/// The store is `None` when the vault is **locked** — i.e. the on-disk file is
/// password-protected and the correct master password has not been entered this
/// session. All data commands refuse to run until it is unlocked.
pub struct AppState {
    pub store: Mutex<Option<Store>>,
}

type CmdResult<T> = Result<T, String>;

fn load(state: &State<AppState>) -> CmdResult<EnvYouLocalState> {
    let guard = state.store.lock().map_err(|_| "state lock poisoned")?;
    let store = guard.as_ref().ok_or_else(vault_locked_err)?;
    Ok(with_verified_pro(store.load().map_err(|e| e.to_string())?))
}

/// Recompute `is_pro` from the **signed** license on every load, rather than
/// trusting the persisted `isPro` boolean. A locally edited state file, an
/// expired token, or a validly-signed non-Pro token therefore grants nothing —
/// the free/Pro boundary rests on an Ed25519 signature, not a JSON flag.
fn with_verified_pro(mut s: EnvYouLocalState) -> EnvYouLocalState {
    s.license.is_pro = license::is_pro_active(s.license.license_key.as_deref(), &machine_id());
    s
}

/// Load, let `f` mutate the state, then persist — all under a single lock
/// acquisition. `load`/`persist` used to be called back-to-back, each taking
/// and releasing the mutex independently; two commands issued close together
/// (Tauri dispatches each on its own thread) could both load the same
/// on-disk state and then persist in sequence, silently discarding one of
/// the two changes. Holding the lock for the whole read-modify-write closes
/// that window.
fn with_state<F>(state: &State<AppState>, f: F) -> CmdResult<EnvYouLocalState>
where
    F: FnOnce(&mut EnvYouLocalState) -> CmdResult<()>,
{
    let guard = state.store.lock().map_err(|_| "state lock poisoned")?;
    let store = guard.as_ref().ok_or_else(vault_locked_err)?;
    let mut s = with_verified_pro(store.load().map_err(|e| e.to_string())?);
    f(&mut s)?;
    store.save(&s).map_err(|e| e.to_string())?;
    Ok(s)
}

fn vault_locked_err() -> String {
    "vault is locked; unlock it with your master password".to_string()
}

/// Absolute path of the encrypted state file at the default location.
fn state_file_path() -> CmdResult<std::path::PathBuf> {
    Ok(storage::default_data_dir()
        .map_err(|e| e.to_string())?
        .join(storage::STATE_FILE))
}

/// Snapshot of the vault's lock state, for the frontend to decide whether to
/// show an unlock screen on launch.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VaultStatus {
    /// Whether an encrypted state file exists on disk yet.
    pub exists: bool,
    /// Whether that file is protected by a master password (Argon2id / v2).
    pub password_protected: bool,
    /// Whether the store is currently unlocked and usable this session.
    pub unlocked: bool,
}

#[tauri::command]
pub fn vault_status(state: State<AppState>) -> CmdResult<VaultStatus> {
    let path = state_file_path()?;
    let exists = path.exists();
    let password_protected = if exists {
        let raw = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        crypto::is_password_protected(&raw).map_err(|e| e.to_string())?
    } else {
        false
    };
    let unlocked = state
        .store
        .lock()
        .map_err(|_| "state lock poisoned")?
        .is_some();
    Ok(VaultStatus {
        exists,
        password_protected,
        unlocked,
    })
}

/// Unlock a password-protected vault for this session. Returns the decrypted
/// state on success; an incorrect password yields an error and leaves the vault
/// locked.
#[tauri::command]
pub fn unlock_vault(state: State<AppState>, password: String) -> CmdResult<EnvYouLocalState> {
    let store = Store::open_default_with_password(password).map_err(|e| e.to_string())?;
    let data = with_verified_pro(
        store
            .load()
            .map_err(|_| "incorrect master password".to_string())?,
    );
    *state.store.lock().map_err(|_| "state lock poisoned")? = Some(store);
    Ok(data)
}

/// Set (or change) the master password: re-seal the currently-unlocked vault
/// under Argon2id with the given password. Requires the vault to be unlocked.
#[tauri::command]
pub fn set_master_password(
    state: State<AppState>,
    password: String,
) -> CmdResult<EnvYouLocalState> {
    if password.trim().chars().count() < 8 {
        return Err("master password must be at least 8 characters".into());
    }
    let mut guard = state.store.lock().map_err(|_| "state lock poisoned")?;
    let current = guard.as_ref().ok_or_else(vault_locked_err)?;
    // Borrow, don't take: if migration fails partway (e.g. a disk write
    // error), `current` is still valid and the vault must not end up
    // permanently "locked" for a session that was fine a moment ago.
    let migrated = current
        .migrate_to_password(password)
        .map_err(|e| e.to_string())?;
    let data = with_verified_pro(migrated.load().map_err(|e| e.to_string())?);
    *guard = Some(migrated);
    Ok(data)
}

#[tauri::command]
pub fn get_state(state: State<AppState>) -> CmdResult<EnvYouLocalState> {
    load(&state)
}

#[tauri::command]
pub fn create_project(
    state: State<AppState>,
    name: String,
    color_tag: String,
) -> CmdResult<EnvYouLocalState> {
    with_state(&state, |s| {
        if !s.can_add_project() {
            return Err("Free tier allows up to 3 projects. Upgrade to Pro for unlimited.".into());
        }
        // Custom env colors are a Pro feature: the free tier is pinned to the
        // default swatch regardless of what the frontend sent (mirrors the
        // UI's locked picker).
        let color_tag = s.enforce_color(color_tag);
        s.projects
            .push(ProjectItem::new(name, color_tag, now_iso8601()));
        Ok(())
    })
}

#[tauri::command]
pub fn delete_project(state: State<AppState>, project_id: String) -> CmdResult<EnvYouLocalState> {
    with_state(&state, |s| {
        let before = s.projects.len();
        s.projects.retain(|p| p.id != project_id);
        if s.projects.len() == before {
            return Err(format!("project not found: {project_id}"));
        }
        Ok(())
    })
}

#[tauri::command]
pub fn rename_project(
    state: State<AppState>,
    project_id: String,
    name: String,
    color_tag: String,
) -> CmdResult<EnvYouLocalState> {
    with_state(&state, |s| {
        // Compute the tier-allowed color before the mutable project borrow;
        // the free tier is pinned to the default swatch (custom colors are
        // Pro-only).
        let color_tag = s.enforce_color(color_tag);
        let p = s
            .project_mut(&project_id)
            .ok_or_else(|| format!("project not found: {project_id}"))?;
        p.name = name;
        p.color_tag = color_tag;
        Ok(())
    })
}

#[tauri::command]
pub fn upsert_variable(
    state: State<AppState>,
    project_id: String,
    key: String,
    value: String,
    comment: Option<String>,
    is_masked: bool,
) -> CmdResult<EnvYouLocalState> {
    with_state(&state, |s| {
        // Only enforce the cap when adding a brand-new key; existing keys may
        // always be updated. Shares `can_write_variable` with the MCP path so
        // both enforce an identical free-tier policy.
        if !s.can_write_variable(&project_id, &key) {
            if s.project(&project_id).is_none() {
                return Err(format!("project not found: {project_id}"));
            }
            return Err("Free tier allows up to 10 variables per project. Upgrade to Pro.".into());
        }
        let p = s
            .project_mut(&project_id)
            .ok_or_else(|| format!("project not found: {project_id}"))?;
        match p.variables.iter_mut().find(|v| v.key == key) {
            Some(v) => {
                v.value = value;
                v.comment = comment;
                v.is_masked = is_masked;
            }
            None => p.variables.push(EnvVariable {
                key,
                value,
                comment,
                is_masked,
            }),
        }
        Ok(())
    })
}

#[tauri::command]
pub fn delete_variable(
    state: State<AppState>,
    project_id: String,
    key: String,
) -> CmdResult<EnvYouLocalState> {
    with_state(&state, |s| {
        let p = s
            .project_mut(&project_id)
            .ok_or_else(|| format!("project not found: {project_id}"))?;
        p.variables.retain(|v| v.key != key);
        Ok(())
    })
}

#[tauri::command]
pub fn save_settings(state: State<AppState>, settings: Settings) -> CmdResult<EnvYouLocalState> {
    with_state(&state, |s| {
        s.settings = settings;
        Ok(())
    })
}

/// Supabase project that hosts the license store + activation RPC. Both the URL
/// and the anon key are **public** (the anon key only exposes the
/// `activate_license` RPC, which validates a code + email and returns the
/// already-signed certificate). No secret ever ships in the app.
const SUPABASE_URL: &str = "https://dfslueqzfmvtpdencasw.supabase.co";
const SUPABASE_ANON_KEY: &str = "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJzdXBhYmFzZSIsInJlZiI6ImRmc2x1ZXF6Zm12dHBkZW5jYXN3Iiwicm9sZSI6ImFub24iLCJpYXQiOjE3ODM1ODMyMjcsImV4cCI6MjA5OTE1OTIyN30.nCyPRp_qBwYBtLHaawafheBaVSpi8Fi8gVhGmgJKjm0";

fn activation_generic_err() -> String {
    "We couldn't activate this license. Please check your license email and code.".to_string()
}

/// Activate Pro online: exchange (email, license code) at the Supabase
/// activation RPC for a signed certificate, verify that certificate **offline**
/// against the embedded public key, and store it. Afterwards Pro persists
/// offline — every load re-verifies the stored certificate (see [`load`]).
#[tauri::command]
pub fn activate_pro(
    state: State<AppState>,
    email: String,
    code: String,
) -> CmdResult<EnvYouLocalState> {
    let email = email.trim().to_string();
    let code_norm = license::normalize_license_code(&code);
    if email.is_empty() || !email.contains('@') || code_norm.len() < 8 {
        return Err("Please enter your license email and code.".into());
    }

    // Ask the activation server (public anon key) to exchange code+email for the
    // signed certificate. The RPC returns HTTP 200 with a JSON result even for
    // logical failures (ok:false), so non-200 means a network/auth problem.
    let url = format!("{SUPABASE_URL}/rest/v1/rpc/activate_license");
    let resp = ureq::post(&url)
        .set("apikey", SUPABASE_ANON_KEY)
        .set("Authorization", &format!("Bearer {SUPABASE_ANON_KEY}"))
        .set("Content-Type", "application/json")
        .timeout(std::time::Duration::from_secs(20))
        .send_json(serde_json::json!({ "p_license_code": code_norm, "p_email": email }));

    let body: serde_json::Value = match resp {
        Ok(r) => r.into_json().map_err(|_| activation_generic_err())?,
        Err(ureq::Error::Status(_, r)) => r.into_json().map_err(|_| activation_generic_err())?,
        Err(_) => return Err(
            "Couldn't reach the activation server. Check your internet connection and try again."
                .into(),
        ),
    };

    if body.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        // Surface the server's friendly message; log only the machine-readable code.
        eprintln!(
            "activation_error server_code={}",
            body.get("code")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown")
        );
        return Err(body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or(&activation_generic_err())
            .to_string());
    }

    let cert = body
        .get("signed_certificate")
        .and_then(|v| v.as_str())
        .ok_or_else(activation_generic_err)?;

    store_certificate(&state, cert).map_err(|e| {
        eprintln!("activation_error=certificate_verify_failed detail={e}");
        "We activated your license but couldn't verify it on this device. Please update envyou to the latest version, or contact ceo@eternalsix.com.".to_string()
    })
}

/// Advanced path: activate directly from a pasted signed certificate (support /
/// offline re-activation). The main path is [`activate_pro`].
#[tauri::command]
pub fn activate_certificate(
    state: State<AppState>,
    certificate: String,
) -> CmdResult<EnvYouLocalState> {
    store_certificate(&state, &certificate)
        .map_err(|_| "This certificate is not valid on this device.".to_string())
}

/// Verify a signed certificate offline against the embedded public key, require
/// a Pro plan, and persist it. Shared by the online and paste paths.
fn store_certificate(state: &State<AppState>, certificate: &str) -> CmdResult<EnvYouLocalState> {
    let claims = license::verify_license(certificate, &machine_id()).map_err(|e| e.to_string())?;
    if !license::grants_pro(&claims) {
        return Err("this certificate does not include the Pro plan".into());
    }
    with_state(state, |s| {
        s.license.is_pro = true;
        s.license.license_key = Some(certificate.trim().to_string());
        s.license.activated_at = Some(now_iso8601());
        Ok(())
    })
}

/// Write the `envyou` MCP server entry into Claude Desktop's config, merging
/// non-destructively (spec §5). Returns the path written.
#[tauri::command]
pub fn link_claude_desktop(state: State<AppState>) -> CmdResult<String> {
    use envyou_core::core::claude_config;

    // Claude Desktop (MCP) linking is a Pro feature. Gate it on the SIGNED
    // license — the same source of truth as every other Pro check — so a
    // bypassed frontend button can't wire up the integration for free.
    if !load(&state)?.license.is_pro {
        return Err("Claude Desktop (MCP) linking is a Pro feature. Upgrade to Pro.".into());
    }

    let path = claude_config::config_path().ok_or_else(|| {
        "Claude Desktop config path is not available on this OS (macOS/Windows only).".to_string()
    })?;

    let exe = std::env::current_exe()
        .map_err(|e| format!("could not resolve current executable: {e}"))?
        .to_string_lossy()
        .to_string();

    let existing = std::fs::read_to_string(&path).ok();
    let merged =
        claude_config::merge_config_str(existing.as_deref(), &exe).map_err(|e| e.to_string())?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, merged).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}
