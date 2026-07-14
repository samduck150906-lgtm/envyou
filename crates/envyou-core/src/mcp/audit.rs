//! Local, value-free audit log for MCP activity (spec §11).
//!
//! An [`AuditEvent`] records *that* an AI touched a variable and how the request
//! resolved — never the value itself. The type has no field that could hold a
//! secret, so "the audit log must never contain a secret value" is guaranteed by
//! construction, not by remembering to redact. The [`AuditSink`] is where a
//! runtime decides to persist events (e.g. to a local file the user can clear);
//! the core only produces them.

use serde::Serialize;

use crate::mcp::ApprovalOutcome;

/// Which value-touching tool an event is about. Listing projects / variable
/// names is not audited (it exposes no values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditTool {
    ReadValues,
    CreateValue,
    UpdateValue,
    DeleteValue,
}

/// How an MCP request resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditOutcome {
    /// User approved everything requested.
    Approved,
    /// User approved some of the requested variables (reads only).
    PartiallyApproved,
    /// User explicitly declined.
    Denied,
    /// No answer within the approval timeout.
    Timeout,
    /// The approval UI could not be shown / the app was unreachable.
    Error,
    /// Approved and the store change succeeded.
    Completed,
    /// Approved but the store change failed (e.g. free-tier cap, project gone).
    Failed,
}

impl From<&ApprovalOutcome> for AuditOutcome {
    fn from(o: &ApprovalOutcome) -> Self {
        match o {
            ApprovalOutcome::Approved { .. } => AuditOutcome::Approved,
            ApprovalOutcome::Denied => AuditOutcome::Denied,
            ApprovalOutcome::Timeout => AuditOutcome::Timeout,
            ApprovalOutcome::Error => AuditOutcome::Error,
        }
    }
}

/// One audited MCP request. **Contains no variable value** — only which client
/// asked, which tool, which project, the variable *names* involved, how many,
/// and the outcome. A timestamp is intentionally omitted here: the sink stamps
/// events when it persists them (the pure core has no clock).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEvent {
    /// The MCP client (e.g. "Claude Desktop"), from the handshake.
    pub client: String,
    pub tool: AuditTool,
    pub project_id: String,
    /// Names of the variables the request concerned — never their values.
    pub variable_names: Vec<String>,
    pub count: usize,
    pub outcome: AuditOutcome,
}

impl AuditEvent {
    pub fn new(
        client: impl Into<String>,
        tool: AuditTool,
        project_id: impl Into<String>,
        variable_names: Vec<String>,
        outcome: AuditOutcome,
    ) -> Self {
        let count = variable_names.len();
        Self {
            client: client.into(),
            tool,
            project_id: project_id.into(),
            variable_names,
            count,
            outcome,
        }
    }
}

/// Where audit events go. A runtime implements this to persist events locally
/// (append to a file, insert into a table, …). The core never persists; it only
/// hands finished, value-free events to the sink.
pub trait AuditSink {
    fn record(&self, event: &AuditEvent);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_carries_names_not_values_and_serializes_without_them() {
        let ev = AuditEvent::new(
            "Claude Desktop",
            AuditTool::ReadValues,
            "p1",
            vec!["DATABASE_URL".into(), "API_KEY".into()],
            AuditOutcome::PartiallyApproved,
        );
        assert_eq!(ev.count, 2);
        let json = serde_json::to_string(&ev).unwrap();
        // Names and metadata are present…
        assert!(json.contains("DATABASE_URL"));
        assert!(json.contains("partially_approved"));
        assert!(json.contains("read_values"));
        // …and there is simply no field that could carry a value. Guard against a
        // value-shaped string ever slipping in.
        assert!(!json.contains("postgres://"));
        assert!(!json.contains("sk_live"));
    }

    #[test]
    fn outcome_maps_from_approval_outcome() {
        assert_eq!(
            AuditOutcome::from(&ApprovalOutcome::Denied),
            AuditOutcome::Denied
        );
        assert_eq!(
            AuditOutcome::from(&ApprovalOutcome::Timeout),
            AuditOutcome::Timeout
        );
        assert_eq!(
            AuditOutcome::from(&ApprovalOutcome::Error),
            AuditOutcome::Error
        );
        assert_eq!(
            AuditOutcome::from(&ApprovalOutcome::approved_all(["X".into()])),
            AuditOutcome::Approved
        );
    }
}
