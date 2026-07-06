//! `envyou --mcp` runtime: wires envyou-core's MCP server to the encrypted
//! local store and a native approval dialog, then serves over STDIO.

use std::io::{self, BufReader};

use envyou_core::core::model::{EnvVariable, ProjectSummary};
use envyou_core::core::storage::Store;
use envyou_core::mcp::{
    serve_stdio, ApprovalDecision, ApprovalGate, ApprovalRequest, EnvStore, McpServer,
};

/// Adapter exposing the encrypted [`Store`] to the MCP server. Each call loads
/// the latest state from disk so the GUI and the MCP process stay in sync.
struct StoreAdapter {
    store: Store,
}

impl EnvStore for StoreAdapter {
    fn list_projects(&self) -> Vec<ProjectSummary> {
        self.store
            .load()
            .map(|s| s.summaries())
            .unwrap_or_default()
    }

    fn read_env_variables(&self, project_id: &str) -> Result<Vec<EnvVariable>, String> {
        let state = self.store.load().map_err(|e| e.to_string())?;
        state
            .project(project_id)
            .map(|p| p.variables.clone())
            .ok_or_else(|| format!("project not found: {project_id}"))
    }

    fn write_env_variable(&self, project_id: &str, key: &str, value: &str) -> Result<(), String> {
        let mut state = self.store.load().map_err(|e| e.to_string())?;
        // Updating an existing key is always allowed; only a brand-new key is
        // subject to the free-tier cap. `can_write_variable` is the same policy
        // predicate the GUI path uses, so both entry points stay in lock-step.
        if !state.can_write_variable(project_id, key) {
            // Either the project is full on the free tier, or it doesn't exist.
            if state.project(project_id).is_none() {
                return Err(format!("project not found: {project_id}"));
            }
            return Err("free-tier variable limit reached (upgrade to Pro)".into());
        }
        let project = state
            .project_mut(project_id)
            .ok_or_else(|| format!("project not found: {project_id}"))?;
        match project.variables.iter_mut().find(|v| v.key == key) {
            Some(existing) => existing.value = value.to_string(),
            None => project.variables.push(EnvVariable {
                key: key.to_string(),
                value: value.to_string(),
                comment: None,
                is_masked: true,
            }),
        }
        self.store.save(&state).map_err(|e| e.to_string())
    }
}

/// Native blocking confirmation dialog implementing the Human-in-the-Loop gate
/// (spec §4.1). Even in headless `--mcp` mode this surfaces a physical OS modal
/// the user must click before any secret leaves the machine.
struct NativeApprovalGate;

impl ApprovalGate for NativeApprovalGate {
    fn request(&self, req: &ApprovalRequest) -> ApprovalDecision {
        let (title, body) = match req {
            ApprovalRequest::Read { project_id } => (
                "envyou — AI read request",
                format!(
                    "Claude is requesting to READ all environment variables\nfor project:\n\n  {project_id}\n\nAllow this?"
                ),
            ),
            ApprovalRequest::Write { project_id, key } => (
                "envyou — AI write request",
                format!(
                    "Claude is requesting to WRITE an environment variable:\n\n  {key}\n\ninto project:\n\n  {project_id}\n\nAllow this?"
                ),
            ),
        };

        let confirmed = rfd::MessageDialog::new()
            .set_level(rfd::MessageLevel::Warning)
            .set_title(title)
            .set_description(&body)
            .set_buttons(rfd::MessageButtons::YesNo)
            .show();

        if matches!(confirmed, rfd::MessageDialogResult::Yes) {
            ApprovalDecision::Approved
        } else {
            ApprovalDecision::Denied
        }
    }
}

/// Build the server and pump STDIO until EOF.
pub fn run_stdio() -> io::Result<()> {
    let store = Store::open_default()
        .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))?;
    let server = McpServer::new(StoreAdapter { store }, NativeApprovalGate);

    let stdin = io::stdin();
    let stdout = io::stdout();
    serve_stdio(&server, BufReader::new(stdin.lock()), stdout.lock())
}

#[cfg(test)]
mod tests {
    use super::*;
    use envyou_core::core::crypto::MasterKey;
    use envyou_core::core::model::{EnvVariable, EnvYouLocalState, ProjectItem, FREE_MAX_VARS_PER_PROJECT};
    use envyou_core::core::storage::{Store, STATE_FILE};

    /// Build a free-tier store whose single project is already at the variable
    /// cap, and return the adapter plus that project's id.
    fn full_free_tier_adapter() -> (StoreAdapter, String, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path().join(STATE_FILE), MasterKey::derive(b"test"));

        let mut state = EnvYouLocalState::default();
        let mut p = ProjectItem::new("api", "#000080", "now");
        let id = p.id.clone();
        for i in 0..FREE_MAX_VARS_PER_PROJECT {
            p.variables.push(EnvVariable {
                key: format!("K{i}"),
                value: format!("v{i}"),
                comment: None,
                is_masked: true,
            });
        }
        state.projects.push(p);
        store.save(&state).unwrap();

        (StoreAdapter { store }, id, dir)
    }

    #[test]
    fn mcp_write_updates_existing_var_at_free_cap() {
        let (adapter, id, _dir) = full_free_tier_adapter();
        // Updating an existing key must succeed even though the project is full.
        adapter
            .write_env_variable(&id, "K0", "updated-value")
            .expect("updating an existing variable must be allowed at the free-tier cap");

        let vars = adapter.read_env_variables(&id).unwrap();
        let k0 = vars.iter().find(|v| v.key == "K0").unwrap();
        assert_eq!(k0.value, "updated-value");
        assert_eq!(vars.len(), FREE_MAX_VARS_PER_PROJECT, "no new variable should be added");
    }

    #[test]
    fn mcp_write_rejects_new_var_at_free_cap_without_leaking_secrets() {
        let (adapter, id, _dir) = full_free_tier_adapter();
        let err = adapter
            .write_env_variable(&id, "BRAND_NEW", "super-secret-value")
            .expect_err("adding a new variable beyond the free cap must fail");

        // The error must be about the cap and must never echo the secret value.
        assert!(err.to_lowercase().contains("limit"), "expected a limit message, got: {err}");
        assert!(!err.contains("super-secret-value"), "error message leaked the secret value");

        // And the rejected variable must not have been persisted.
        let vars = adapter.read_env_variables(&id).unwrap();
        assert!(vars.iter().all(|v| v.key != "BRAND_NEW"));
        assert_eq!(vars.len(), FREE_MAX_VARS_PER_PROJECT);
    }

    #[test]
    fn mcp_write_to_unknown_project_reports_not_found() {
        let (adapter, _id, _dir) = full_free_tier_adapter();
        let err = adapter
            .write_env_variable("no-such-project", "K", "V")
            .expect_err("writing to an unknown project must fail");
        assert!(err.contains("not found"), "expected not-found, got: {err}");
    }
}
