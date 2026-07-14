//! Data model for envyou's local state.
//!
//! Mirrors the `EnvYouLocalState` TypeScript interface from the product
//! specification (§7) so the Rust backend and the JS frontend agree on the
//! shape of `enc_state.json`.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Free-tier limits (spec §6.2). Enforced unless [`License::is_pro`] is true.
pub const FREE_MAX_PROJECTS: usize = 3;
pub const FREE_MAX_VARS_PER_PROJECT: usize = 10;

/// The single environment color available on the free tier; Pro unlocks the
/// full palette. The frontend's color picker locks every other swatch, and the
/// backend mirrors that here (see [`EnvYouLocalState::enforce_color`]) so custom
/// colors are a real Pro feature, not merely a hidden button. Must match the
/// frontend default swatch (`COLORS[0]` in `app.js`).
pub const FREE_DEFAULT_COLOR: &str = "#008080";

/// Current on-disk schema version.
pub const STATE_VERSION: &str = "1.0.0";

/// Root state object persisted (encrypted) to `enc_state.json`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EnvYouLocalState {
    pub version: String,
    pub license: License,
    pub settings: Settings,
    pub projects: Vec<ProjectItem>,
}

impl Default for EnvYouLocalState {
    fn default() -> Self {
        Self {
            version: STATE_VERSION.to_string(),
            license: License::default(),
            settings: Settings::default(),
            projects: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct License {
    #[serde(rename = "isPro")]
    pub is_pro: bool,
    #[serde(rename = "licenseKey")]
    pub license_key: Option<String>,
    #[serde(rename = "activatedAt")]
    pub activated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    #[serde(rename = "globalHotkey")]
    pub global_hotkey: String,
    #[serde(rename = "alwaysOnTop")]
    pub always_on_top: bool,
    #[serde(rename = "maskSensitiveData")]
    pub mask_sensitive_data: bool,
    /// Which capabilities the MCP server (Claude Desktop / Claude Code) is
    /// allowed to use. Defaulted via serde so a pre-existing `enc_state.json`
    /// written before this field existed still deserializes — it simply loads
    /// the safe [`McpAccess::default`] (reads on, writes/deletes off).
    #[serde(default)]
    pub mcp: McpAccess,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            global_hotkey: "Ctrl+Shift+E".to_string(),
            always_on_top: true,
            mask_sensitive_data: true,
            mcp: McpAccess::default(),
        }
    }
}

/// User-controlled gate on what an MCP client may do. Mutating capabilities are
/// **opt-in** (default `false`) so an AI can never create, change, or remove a
/// secret until the user has deliberately turned that on — the "human decides
/// what the AI can even attempt" layer that sits *above* the per-call approval
/// dialog.
///
/// The `enabled` master switch defaults to `false`: MCP access is off until the
/// user links a client and turns it on. Every field is `#[serde(default)]` so
/// partially-written or older settings round-trip to the safe value rather than
/// silently enabling something.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct McpAccess {
    /// Master switch. When `false`, the MCP server rejects every tool call
    /// regardless of the more specific flags below.
    #[serde(default)]
    pub enabled: bool,
    /// Allow `list_projects` (names + counts only, never values).
    #[serde(default = "default_true")]
    pub list_projects: bool,
    /// Allow `list_variable_names` (variable names only, never values).
    #[serde(default = "default_true")]
    pub list_variable_names: bool,
    /// Allow `read_env_variables` (returns approved values — the AI sees them).
    #[serde(default = "default_true")]
    pub read_values: bool,
    /// Allow `write_env_variable`. Opt-in: an AI cannot modify secrets until the
    /// user turns this on.
    #[serde(default)]
    pub write_values: bool,
    /// Allow deletion tools. Opt-in and independent of `write_values`.
    #[serde(default)]
    pub delete_values: bool,
    /// How long (seconds) an approval dialog waits for the user before the
    /// request is auto-denied. Clamp with [`McpAccess::timeout_secs`].
    #[serde(default = "default_approval_timeout")]
    pub approval_timeout_secs: u32,
}

fn default_true() -> bool {
    true
}

fn default_approval_timeout() -> u32 {
    60
}

impl Default for McpAccess {
    fn default() -> Self {
        Self {
            enabled: false,
            list_projects: true,
            list_variable_names: true,
            read_values: true,
            write_values: false,
            delete_values: false,
            approval_timeout_secs: default_approval_timeout(),
        }
    }
}

impl McpAccess {
    /// The approval timeout, clamped to a sane range (10s–600s) so a
    /// corrupted/hostile settings value can neither make approvals hang nor make
    /// the window unusably short.
    pub fn timeout_secs(&self) -> u32 {
        self.approval_timeout_secs.clamp(10, 600)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectItem {
    pub id: String,
    pub name: String,
    #[serde(rename = "colorTag")]
    pub color_tag: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    pub variables: Vec<EnvVariable>,
}

impl ProjectItem {
    /// Create a new project with a freshly generated UUID v4.
    pub fn new(
        name: impl Into<String>,
        color_tag: impl Into<String>,
        created_at: impl Into<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: name.into(),
            color_tag: color_tag.into(),
            created_at: created_at.into(),
            variables: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EnvVariable {
    pub key: String,
    pub value: String,
    pub comment: Option<String>,
    #[serde(rename = "isMasked")]
    pub is_masked: bool,
}

/// Lightweight project summary returned by the MCP `list_projects` tool
/// (never exposes variable values).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProjectSummary {
    pub id: String,
    pub name: String,
    #[serde(rename = "variableCount")]
    pub variable_count: usize,
}

impl From<&ProjectItem> for ProjectSummary {
    fn from(p: &ProjectItem) -> Self {
        Self {
            id: p.id.clone(),
            name: p.name.clone(),
            variable_count: p.variables.len(),
        }
    }
}

impl EnvYouLocalState {
    /// Whether a new project may be created under the current tier.
    pub fn can_add_project(&self) -> bool {
        self.license.is_pro || self.projects.len() < FREE_MAX_PROJECTS
    }

    /// Whether a new variable may be added to the given project under the
    /// current tier.
    pub fn can_add_variable(&self, project_id: &str) -> bool {
        if self.license.is_pro {
            return true;
        }
        match self.project(project_id) {
            Some(p) => p.variables.len() < FREE_MAX_VARS_PER_PROJECT,
            None => false,
        }
    }

    /// Whether a write (insert-or-update) of `key` into `project_id` is allowed
    /// under the current tier.
    ///
    /// Updating an **existing** key is always permitted; only a brand-new key
    /// counts against the free-tier per-project cap. Returns `false` for an
    /// unknown project (callers disambiguate not-found from cap-reached).
    ///
    /// This is the single source of truth shared by both write entry points —
    /// the GUI `upsert_variable` command and the MCP `write_env_variable` tool —
    /// so their free-tier policy can never drift apart.
    pub fn can_write_variable(&self, project_id: &str, key: &str) -> bool {
        let key_exists = self
            .project(project_id)
            .map(|p| p.variables.iter().any(|v| v.key == key))
            .unwrap_or(false);
        key_exists || self.can_add_variable(project_id)
    }

    /// The color a project is allowed to use under the current tier: Pro keeps
    /// any requested color; the free tier is forced back to
    /// [`FREE_DEFAULT_COLOR`]. Backend counterpart to the UI's Pro-locked color
    /// picker so a bypassed frontend can't set a custom color for free.
    pub fn enforce_color(&self, requested: impl Into<String>) -> String {
        let requested = requested.into();
        if self.license.is_pro {
            requested
        } else {
            FREE_DEFAULT_COLOR.to_string()
        }
    }

    pub fn project(&self, project_id: &str) -> Option<&ProjectItem> {
        self.projects.iter().find(|p| p.id == project_id)
    }

    pub fn project_mut(&mut self, project_id: &str) -> Option<&mut ProjectItem> {
        self.projects.iter_mut().find(|p| p.id == project_id)
    }

    pub fn summaries(&self) -> Vec<ProjectSummary> {
        self.projects.iter().map(ProjectSummary::from).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_free_tier() {
        let s = EnvYouLocalState::default();
        assert!(!s.license.is_pro);
        assert_eq!(s.settings.global_hotkey, "Ctrl+Shift+E");
        assert_eq!(s.version, STATE_VERSION);
    }

    #[test]
    fn free_tier_pins_color_to_default_pro_keeps_custom() {
        let mut s = EnvYouLocalState::default();
        assert!(!s.license.is_pro);
        // Free tier: any requested color collapses to the default swatch.
        assert_eq!(s.enforce_color("#FF0000"), FREE_DEFAULT_COLOR);
        assert_eq!(s.enforce_color(FREE_DEFAULT_COLOR), FREE_DEFAULT_COLOR);
        // Pro: the requested color is kept verbatim.
        s.license.is_pro = true;
        assert_eq!(s.enforce_color("#FF0000"), "#FF0000");
    }

    #[test]
    fn free_tier_caps_projects() {
        let mut s = EnvYouLocalState::default();
        for i in 0..FREE_MAX_PROJECTS {
            assert!(s.can_add_project());
            s.projects
                .push(ProjectItem::new(format!("p{i}"), "#008080", "now"));
        }
        assert!(!s.can_add_project());
        s.license.is_pro = true;
        assert!(s.can_add_project());
    }

    #[test]
    fn free_tier_caps_variables() {
        let mut s = EnvYouLocalState::default();
        let mut p = ProjectItem::new("p", "#008080", "now");
        let id = p.id.clone();
        for i in 0..FREE_MAX_VARS_PER_PROJECT {
            p.variables.push(EnvVariable {
                key: format!("K{i}"),
                value: "v".into(),
                comment: None,
                is_masked: false,
            });
        }
        s.projects.push(p);
        assert!(!s.can_add_variable(&id));
        s.license.is_pro = true;
        assert!(s.can_add_variable(&id));
    }

    /// A project at the free-tier variable cap must still allow *updating* an
    /// existing key — only brand-new keys are capped. This is the regression
    /// guard for the MCP write path that previously blocked all writes once the
    /// cap was hit.
    #[test]
    fn free_tier_allows_update_but_not_new_var_at_cap() {
        let mut s = EnvYouLocalState::default();
        let mut p = ProjectItem::new("p", "#008080", "now");
        let id = p.id.clone();
        for i in 0..FREE_MAX_VARS_PER_PROJECT {
            p.variables.push(EnvVariable {
                key: format!("K{i}"),
                value: "v".into(),
                comment: None,
                is_masked: false,
            });
        }
        s.projects.push(p);

        assert!(
            s.can_write_variable(&id, "K0"),
            "updating an existing key must be allowed even at the free-tier cap"
        );
        assert!(
            !s.can_write_variable(&id, "BRAND_NEW"),
            "adding a new key beyond the free-tier cap must be denied"
        );
    }

    #[test]
    fn can_write_variable_is_false_for_unknown_project() {
        let s = EnvYouLocalState::default();
        assert!(
            !s.can_write_variable("no-such-project", "K"),
            "an unknown project must not be writable"
        );
    }

    #[test]
    fn pro_tier_allows_new_var_beyond_cap() {
        let mut s = EnvYouLocalState::default();
        let mut p = ProjectItem::new("p", "#008080", "now");
        let id = p.id.clone();
        for i in 0..FREE_MAX_VARS_PER_PROJECT {
            p.variables.push(EnvVariable {
                key: format!("K{i}"),
                value: "v".into(),
                comment: None,
                is_masked: false,
            });
        }
        s.projects.push(p);
        s.license.is_pro = true;
        assert!(
            s.can_write_variable(&id, "BRAND_NEW"),
            "Pro tier must allow new keys beyond the free cap"
        );
    }

    #[test]
    fn mcp_access_defaults_are_fail_closed_for_mutations() {
        let a = McpAccess::default();
        // Master switch off until the user opts in.
        assert!(!a.enabled, "MCP must be off by default");
        // Read-only capabilities are on so a linked-and-enabled client is useful,
        // but mutation is opt-in.
        assert!(a.list_projects);
        assert!(a.list_variable_names);
        assert!(a.read_values);
        assert!(!a.write_values, "writes must be opt-in");
        assert!(!a.delete_values, "deletes must be opt-in");
    }

    /// A state file written before the `mcp` settings field existed must still
    /// load — deserializing to the safe default rather than failing or enabling
    /// anything.
    #[test]
    fn legacy_settings_without_mcp_field_deserialize_to_safe_default() {
        let legacy = r#"{
            "globalHotkey": "Ctrl+Shift+E",
            "alwaysOnTop": true,
            "maskSensitiveData": true
        }"#;
        let s: Settings = serde_json::from_str(legacy).unwrap();
        assert_eq!(s.mcp, McpAccess::default());
        assert!(!s.mcp.enabled);
        assert!(!s.mcp.write_values);
    }

    /// A partially-specified mcp object must fill the rest from safe defaults —
    /// e.g. enabling reads must not silently enable writes.
    #[test]
    fn partial_mcp_settings_fill_missing_fields_safely() {
        let json = r#"{
            "globalHotkey": "Ctrl+Shift+E",
            "alwaysOnTop": true,
            "maskSensitiveData": true,
            "mcp": { "enabled": true, "readValues": true }
        }"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert!(s.mcp.enabled);
        assert!(s.mcp.read_values);
        assert!(!s.mcp.write_values, "unspecified write flag must stay off");
        assert!(
            !s.mcp.delete_values,
            "unspecified delete flag must stay off"
        );
    }

    #[test]
    fn approval_timeout_is_clamped_to_sane_bounds() {
        let mut a = McpAccess::default();
        assert_eq!(a.timeout_secs(), 60);
        a.approval_timeout_secs = 0; // hostile/corrupt: too short
        assert_eq!(a.timeout_secs(), 10);
        a.approval_timeout_secs = 100_000; // absurdly long
        assert_eq!(a.timeout_secs(), 600);
        a.approval_timeout_secs = 45;
        assert_eq!(a.timeout_secs(), 45);
    }

    #[test]
    fn full_state_round_trips_with_mcp_settings() {
        let mut s = EnvYouLocalState::default();
        s.settings.mcp.enabled = true;
        s.settings.mcp.write_values = true;
        let json = serde_json::to_string(&s).unwrap();
        let back: EnvYouLocalState = serde_json::from_str(&json).unwrap();
        assert_eq!(s, back);
    }

    #[test]
    fn summary_hides_values() {
        let mut p = ProjectItem::new("p", "#008080", "now");
        p.variables.push(EnvVariable {
            key: "SECRET".into(),
            value: "super-secret".into(),
            comment: None,
            is_masked: true,
        });
        let sum = ProjectSummary::from(&p);
        assert_eq!(sum.variable_count, 1);
        // ProjectSummary intentionally has no value field.
        let json = serde_json::to_string(&sum).unwrap();
        assert!(!json.contains("super-secret"));
    }
}
