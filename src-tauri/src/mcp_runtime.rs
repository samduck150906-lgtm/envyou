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
        if !state.can_add_variable(project_id) {
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
