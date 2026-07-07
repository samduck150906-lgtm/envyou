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
    store.load().map_err(|e| e.to_string())
}

fn persist(state: &State<AppState>, s: &EnvYouLocalState) -> CmdResult<()> {
    let guard = state.store.lock().map_err(|_| "state lock poisoned")?;
    let store = guard.as_ref().ok_or_else(vault_locked_err)?;
    store.save(s).map_err(|e| e.to_string())
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
        crypto::is_password_protected(&raw)
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
    let data = store
        .load()
        .map_err(|_| "incorrect master password".to_string())?;
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
    let current = guard.take().ok_or_else(vault_locked_err)?;
    let migrated = current
        .migrate_to_password(password)
        .map_err(|e| e.to_string())?;
    let data = migrated.load().map_err(|e| e.to_string())?;
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
    let mut s = load(&state)?;
    if !s.can_add_project() {
        return Err("Free tier allows up to 3 projects. Upgrade to Pro for unlimited.".into());
    }
    s.projects
        .push(ProjectItem::new(name, color_tag, now_iso8601()));
    persist(&state, &s)?;
    Ok(s)
}

#[tauri::command]
pub fn delete_project(state: State<AppState>, project_id: String) -> CmdResult<EnvYouLocalState> {
    let mut s = load(&state)?;
    let before = s.projects.len();
    s.projects.retain(|p| p.id != project_id);
    if s.projects.len() == before {
        return Err(format!("project not found: {project_id}"));
    }
    persist(&state, &s)?;
    Ok(s)
}

#[tauri::command]
pub fn rename_project(
    state: State<AppState>,
    project_id: String,
    name: String,
    color_tag: String,
) -> CmdResult<EnvYouLocalState> {
    let mut s = load(&state)?;
    let p = s
        .project_mut(&project_id)
        .ok_or_else(|| format!("project not found: {project_id}"))?;
    p.name = name;
    p.color_tag = color_tag;
    persist(&state, &s)?;
    Ok(s)
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
    let mut s = load(&state)?;
    // Only enforce the cap when adding a brand-new key; existing keys may always
    // be updated. Shares `can_write_variable` with the MCP path so both enforce
    // an identical free-tier policy.
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
    persist(&state, &s)?;
    Ok(s)
}

#[tauri::command]
pub fn delete_variable(
    state: State<AppState>,
    project_id: String,
    key: String,
) -> CmdResult<EnvYouLocalState> {
    let mut s = load(&state)?;
    let p = s
        .project_mut(&project_id)
        .ok_or_else(|| format!("project not found: {project_id}"))?;
    p.variables.retain(|v| v.key != key);
    persist(&state, &s)?;
    Ok(s)
}

#[tauri::command]
pub fn save_settings(state: State<AppState>, settings: Settings) -> CmdResult<EnvYouLocalState> {
    let mut s = load(&state)?;
    s.settings = settings;
    persist(&state, &s)?;
    Ok(s)
}

#[tauri::command]
pub fn activate_license(
    state: State<AppState>,
    license_key: String,
) -> CmdResult<EnvYouLocalState> {
    // Validate + machine-bind the key offline (spec §6.3).
    license::activate(&license_key, &machine_id()).map_err(|e| e.to_string())?;
    let mut s = load(&state)?;
    s.license.is_pro = true;
    s.license.license_key = Some(license_key.trim().to_string());
    s.license.activated_at = Some(now_iso8601());
    persist(&state, &s)?;
    Ok(s)
}

/// Write the `envyou` MCP server entry into Claude Desktop's config, merging
/// non-destructively (spec §5). Returns the path written.
#[tauri::command]
pub fn link_claude_desktop() -> CmdResult<String> {
    use envyou_core::core::claude_config;

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
