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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct License {
    #[serde(rename = "isPro")]
    pub is_pro: bool,
    #[serde(rename = "licenseKey")]
    pub license_key: Option<String>,
    #[serde(rename = "activatedAt")]
    pub activated_at: Option<String>,
}

impl Default for License {
    fn default() -> Self {
        Self {
            is_pro: false,
            license_key: None,
            activated_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Settings {
    #[serde(rename = "globalHotkey")]
    pub global_hotkey: String,
    #[serde(rename = "alwaysOnTop")]
    pub always_on_top: bool,
    #[serde(rename = "maskSensitiveData")]
    pub mask_sensitive_data: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            global_hotkey: "Ctrl+Shift+E".to_string(),
            always_on_top: true,
            mask_sensitive_data: true,
        }
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
    pub fn new(name: impl Into<String>, color_tag: impl Into<String>, created_at: impl Into<String>) -> Self {
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
    fn free_tier_caps_projects() {
        let mut s = EnvYouLocalState::default();
        for i in 0..FREE_MAX_PROJECTS {
            assert!(s.can_add_project());
            s.projects.push(ProjectItem::new(format!("p{i}"), "#008080", "now"));
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
