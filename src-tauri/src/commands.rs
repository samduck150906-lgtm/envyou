//! Tauri commands invoked from the retro frontend. All operate on the
//! encrypted local [`Store`]; nothing ever leaves the machine.

use std::sync::Mutex;

use envyou_core::core::license;
use envyou_core::core::model::{EnvVariable, EnvYouLocalState, ProjectItem, Settings};
use envyou_core::core::storage::{machine_id, Store};
use tauri::State;

use crate::util::now_iso8601;

/// Tauri-managed application state.
pub struct AppState {
    pub store: Mutex<Store>,
}

type CmdResult<T> = Result<T, String>;

fn load(state: &State<AppState>) -> CmdResult<EnvYouLocalState> {
    let store = state.store.lock().map_err(|_| "state lock poisoned")?;
    store.load().map_err(|e| e.to_string())
}

fn persist(state: &State<AppState>, s: &EnvYouLocalState) -> CmdResult<()> {
    let store = state.store.lock().map_err(|_| "state lock poisoned")?;
    store.save(s).map_err(|e| e.to_string())
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
    let exists = s
        .project(&project_id)
        .map(|p| p.variables.iter().any(|v| v.key == key))
        .unwrap_or(false);
    // Only enforce the cap when adding a brand-new key.
    if !exists && !s.can_add_variable(&project_id) {
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
pub fn activate_license(state: State<AppState>, license_key: String) -> CmdResult<EnvYouLocalState> {
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
    let merged = claude_config::merge_config_str(existing.as_deref(), &exe)
        .map_err(|e| e.to_string())?;

    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, merged).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().to_string())
}
