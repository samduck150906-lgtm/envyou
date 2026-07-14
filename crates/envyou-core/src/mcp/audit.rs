//! Local, value-free audit log for MCP activity (spec §11).
//!
//! An [`AuditEvent`] records *that* an AI touched a variable and how the request
//! resolved — never the value itself. The type has no field that could hold a
//! secret, so "the audit log must never contain a secret value" is guaranteed by
//! construction, not by remembering to redact. The [`AuditSink`] is where a
//! runtime decides to persist events (e.g. to a local file the user can clear);
//! the core only produces them.

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::mcp::ApprovalOutcome;

/// Which value-touching tool an event is about. Listing projects / variable
/// names is not audited (it exposes no values).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuditTool {
    ReadValues,
    CreateValue,
    UpdateValue,
    DeleteValue,
}

/// How an MCP request resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

    /// Serialize this event as one JSON line (no trailing newline) stamped with
    /// `at`. Pure — the caller supplies the timestamp — so it is fully testable
    /// and independent of any clock.
    pub fn to_jsonl_line(&self, at: &str) -> String {
        let rec = AuditRecord {
            at: at.to_string(),
            event: self.clone(),
        };
        // An AuditRecord always serializes; fall back to an empty object rather
        // than panicking in a logging path.
        serde_json::to_string(&rec).unwrap_or_else(|_| "{}".to_string())
    }
}

/// A persisted audit entry: a timestamp plus a (flattened) [`AuditEvent`]. Still
/// value-free — it only adds `at`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditRecord {
    /// When the event was recorded (ISO-8601, supplied by the sink's clock).
    pub at: String,
    #[serde(flatten)]
    pub event: AuditEvent,
}

/// Where audit events go. A runtime implements this to persist events locally
/// (append to a file, insert into a table, …). The core never persists on its
/// own; it only hands finished, value-free events to the sink.
pub trait AuditSink {
    fn record(&self, event: &AuditEvent);
}

/// An [`AuditSink`] that appends each event as one JSON line to a local file
/// (JSONL). Best-effort: a failed write is swallowed rather than allowed to
/// break an MCP request, and nothing is ever written to stdout/stderr.
///
/// The file only ever contains [`AuditRecord`]s — timestamps, client, tool,
/// project id, variable *names*, count, and outcome — so it is safe to keep,
/// read back, or export as-is; it can never contain a secret value.
pub struct FileAuditSink {
    path: PathBuf,
    clock: Box<dyn Fn() -> String>,
}

impl FileAuditSink {
    /// Create a sink appending to `path`. `clock` supplies the per-event
    /// timestamp (e.g. an ISO-8601 `now`); injecting it keeps the sink testable.
    pub fn new(path: impl Into<PathBuf>, clock: impl Fn() -> String + 'static) -> Self {
        Self {
            path: path.into(),
            clock: Box::new(clock),
        }
    }
}

impl AuditSink for FileAuditSink {
    fn record(&self, event: &AuditEvent) {
        let mut line = event.to_jsonl_line(&(self.clock)());
        line.push('\n');
        if let Some(parent) = self.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        // Append the whole line in one write so concurrent `--mcp` processes
        // don't interleave partial lines. Failures are intentionally ignored:
        // auditing is best-effort and must never break the actual request.
        if let Ok(mut f) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

/// Read back a JSONL audit file. Missing file → empty list; malformed lines are
/// skipped rather than aborting the whole read. Never returns a value (the file
/// has none to begin with).
pub fn read_audit_jsonl(path: &Path) -> std::io::Result<Vec<AuditRecord>> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e),
    };
    let mut out = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Ok(rec) = serde_json::from_str::<AuditRecord>(line) {
            out.push(rec);
        }
    }
    Ok(out)
}

/// Delete the audit log entirely (the user's "clear all"). A missing file is a
/// no-op success.
pub fn clear_audit_log(path: &Path) -> std::io::Result<()> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
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

    fn sample(names: Vec<&str>, outcome: AuditOutcome) -> AuditEvent {
        AuditEvent::new(
            "Claude Desktop",
            AuditTool::ReadValues,
            "p1",
            names.into_iter().map(String::from).collect(),
            outcome,
        )
    }

    /// The serialized JSONL line must contain exactly the metadata keys and no
    /// others — there is structurally no field that could carry a value.
    #[test]
    fn jsonl_line_has_only_value_free_keys() {
        let line = sample(vec!["DATABASE_URL"], AuditOutcome::Approved)
            .to_jsonl_line("2026-07-14T00:00:00Z");
        let v: serde_json::Value = serde_json::from_str(&line).unwrap();
        let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(|k| k.as_str()).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            vec![
                "at",
                "client",
                "count",
                "outcome",
                "projectId",
                "tool",
                "variableNames"
            ]
        );
        assert_eq!(v["at"], "2026-07-14T00:00:00Z");
    }

    #[test]
    fn file_sink_appends_one_line_per_event_and_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        // Deterministic clock for the test.
        let sink = FileAuditSink::new(path.clone(), || "2026-07-14T12:00:00Z".to_string());
        sink.record(&sample(vec!["DATABASE_URL"], AuditOutcome::Approved));
        sink.record(&sample(vec!["API_KEY", "TOKEN"], AuditOutcome::Denied));

        let raw = std::fs::read_to_string(&path).unwrap();
        assert_eq!(raw.lines().count(), 2, "one JSON line per event");
        // Best-effort guard that a value never lands in the file.
        assert!(!raw.contains("postgres://"));

        let records = read_audit_jsonl(&path).unwrap();
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].at, "2026-07-14T12:00:00Z");
        assert_eq!(records[0].event.outcome, AuditOutcome::Approved);
        assert_eq!(records[1].event.variable_names, vec!["API_KEY", "TOKEN"]);
        assert_eq!(records[1].event.count, 2);
    }

    #[test]
    fn read_audit_jsonl_skips_blank_and_garbage_lines() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        let good = sample(vec!["X"], AuditOutcome::Completed).to_jsonl_line("t");
        std::fs::write(&path, format!("\n{good}\nnot json at all\n\n")).unwrap();
        let records = read_audit_jsonl(&path).unwrap();
        assert_eq!(records.len(), 1, "only the one valid line is returned");
    }

    #[test]
    fn read_missing_file_is_empty_and_clear_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("audit.jsonl");
        assert!(read_audit_jsonl(&path).unwrap().is_empty());
        // Clearing a non-existent log is a no-op success.
        clear_audit_log(&path).unwrap();

        FileAuditSink::new(path.clone(), || "t".to_string())
            .record(&sample(vec!["X"], AuditOutcome::Approved));
        assert!(path.exists());
        clear_audit_log(&path).unwrap();
        assert!(!path.exists(), "clear removes the log");
    }
}
