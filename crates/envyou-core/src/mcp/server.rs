//! JSON-RPC 2.0 / MCP request handling.
//!
//! Security model (spec §4 + hardening):
//! * **Policy first.** [`McpPolicy`] decides which tools an AI may even attempt.
//!   Mutating tools are opt-in; a disabled tool is refused *before* any approval
//!   dialog is shown.
//! * **Human-in-the-loop, fail-closed.** Value reads and every write block on
//!   [`ApprovalGate`]. Anything that is not an explicit `Approved` decision —
//!   denial, timeout, or an approval UI that could not be shown — results in no
//!   data leaving. There is no fail-open path.
//! * **Least data.** Reads are scoped to explicitly named variables (no "read
//!   everything" default, no wildcards); only the values the user approves are
//!   returned. Values never appear in error messages.

use std::cell::RefCell;
use std::io::{BufRead, Write};

use serde_json::{json, Value};

use crate::core::model::{EnvVariable, McpAccess, ProjectSummary};

/// Protocol version advertised during `initialize`.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// Which MCP capabilities are permitted. Built from the user's persisted
/// [`McpAccess`] settings; the server consults it before running any tool.
///
/// Defaults are fail-closed for mutation: [`McpPolicy::default`] disables the
/// whole server. Use [`McpPolicy::from`] to derive an active policy from the
/// user's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpPolicy {
    /// Master switch — when false, every tool call is refused.
    pub enabled: bool,
    pub list_projects: bool,
    pub list_variable_names: bool,
    pub read_values: bool,
    pub write_values: bool,
    pub delete_values: bool,
}

impl Default for McpPolicy {
    /// Everything off — the safe base. A real server derives its policy from the
    /// user's [`McpAccess`] settings via [`McpPolicy::from`].
    fn default() -> Self {
        Self {
            enabled: false,
            list_projects: false,
            list_variable_names: false,
            read_values: false,
            write_values: false,
            delete_values: false,
        }
    }
}

impl From<&McpAccess> for McpPolicy {
    fn from(a: &McpAccess) -> Self {
        Self {
            enabled: a.enabled,
            list_projects: a.list_projects,
            list_variable_names: a.list_variable_names,
            read_values: a.read_values,
            write_values: a.write_values,
            delete_values: a.delete_values,
        }
    }
}

/// The concrete thing an approval request is asking the user to allow. Carries
/// exactly what the UI must show — never a secret value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalAction {
    /// Read the plaintext values of these named variables. The user sees every
    /// name and the count, and may approve a subset.
    ReadValues { names: Vec<String> },
    /// Create or update a single variable. `creating` distinguishes a brand-new
    /// key from an update so the UI can warn appropriately. The value itself is
    /// deliberately *not* included.
    WriteValue { key: String, creating: bool },
}

/// A fully-described approval request handed to the [`ApprovalGate`]. Everything
/// the approval UI needs to let the user make an informed decision (spec §4.1),
/// and nothing more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalRequest {
    /// Which client is asking (e.g. "Claude Desktop", "Claude Code"), taken from
    /// the MCP `initialize` handshake; "Unknown MCP client" if unstated.
    pub client: String,
    pub project_id: String,
    /// Human-readable project name, if the store could resolve one.
    pub project_name: String,
    pub action: ApprovalAction,
    /// Optional free-text reason/purpose the AI supplied. Advisory only.
    pub reason: Option<String>,
}

/// The gate's decision. Anything other than [`ApprovalOutcome::Approved`] means
/// no data is released — the server treats denial, timeout, and error
/// identically (fail-closed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalOutcome {
    /// The user approved these specific names/keys. For reads this may be a
    /// subset of what was requested (partial approval); for writes it contains
    /// the single key when approved, empty when not.
    Approved { granted: Vec<String> },
    /// The user explicitly declined.
    Denied,
    /// No response within the approval timeout.
    Timeout,
    /// The approval UI could not be shown or the desktop app could not be
    /// reached. Treated as a denial — never as an implicit yes.
    Error,
}

impl ApprovalOutcome {
    /// Convenience for the common "approve everything asked" case.
    pub fn approved_all(names: impl IntoIterator<Item = String>) -> Self {
        ApprovalOutcome::Approved {
            granted: names.into_iter().collect(),
        }
    }
}

/// Human-in-the-loop gate. The real implementation pops a native modal on the
/// desktop and blocks (with a timeout) until the user decides (spec §4.1).
pub trait ApprovalGate {
    fn request(&self, req: &ApprovalRequest) -> ApprovalOutcome;
}

/// Read/write access to the (decrypted) project store. Implemented by the Tauri
/// layer over the encrypted [`crate::core::storage::Store`].
pub trait EnvStore {
    fn list_projects(&self) -> Vec<ProjectSummary>;
    fn read_env_variables(&self, project_id: &str) -> Result<Vec<EnvVariable>, String>;
    fn write_env_variable(&self, project_id: &str, key: &str, value: &str) -> Result<(), String>;
}

/// The MCP server, generic over the store and approval gate.
pub struct McpServer<S: EnvStore, G: ApprovalGate> {
    store: S,
    gate: G,
    policy: McpPolicy,
    /// Client name learned from the `initialize` handshake. Interior mutability
    /// because the STDIO loop drives the server through `&self`; the loop is
    /// single-threaded so a `RefCell` is sufficient.
    client: RefCell<String>,
}

const UNKNOWN_CLIENT: &str = "Unknown MCP client";

impl<S: EnvStore, G: ApprovalGate> McpServer<S, G> {
    pub fn new(store: S, gate: G, policy: McpPolicy) -> Self {
        Self {
            store,
            gate,
            policy,
            client: RefCell::new(UNKNOWN_CLIENT.to_string()),
        }
    }

    fn client_name(&self) -> String {
        self.client.borrow().clone()
    }

    /// Handle a single JSON-RPC request line. Returns `Some(response_json)` for
    /// requests, or `None` for notifications (which take no response).
    pub fn handle_line(&self, line: &str) -> Option<String> {
        let req: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return Some(error_response(Value::Null, -32700, "Parse error")),
        };

        let method = req.get("method").and_then(Value::as_str).unwrap_or("");

        // Notifications have no `id` and expect no response.
        let id = req.get("id").cloned()?;

        let result = match method {
            "initialize" => Ok(self.initialize(req.get("params"))),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(self.tools_list()),
            "tools/call" => self.tools_call(req.get("params")),
            other => Err((-32601, format!("Method not found: {other}"))),
        };

        Some(match result {
            Ok(value) => success_response(id, value),
            Err((code, msg)) => error_response(id, code, &msg),
        })
    }

    fn initialize(&self, params: Option<&Value>) -> Value {
        // Remember who connected so approval dialogs can name the client.
        if let Some(name) = params
            .and_then(|p| p.get("clientInfo"))
            .and_then(|c| c.get("name"))
            .and_then(Value::as_str)
        {
            let name = name.trim();
            if !name.is_empty() {
                *self.client.borrow_mut() = name.to_string();
            }
        }
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "envyou", "version": env!("CARGO_PKG_VERSION") }
        })
    }

    fn tools_list(&self) -> Value {
        // Advertise only the tools the current policy actually permits, so an AI
        // isn't told it can write when writes are off.
        json!({ "tools": tool_schemas(&self.policy) })
    }

    fn tools_call(&self, params: Option<&Value>) -> Result<Value, (i64, String)> {
        let params = params.ok_or((-32602, "Missing params".to_string()))?;
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((-32602, "Missing tool name".to_string()))?;
        let args = params
            .get("arguments")
            .cloned()
            .unwrap_or_else(|| json!({}));

        // Master switch: if MCP is disabled, nothing is reachable.
        if !self.policy.enabled {
            return Ok(tool_text(
                "envyou MCP access is disabled. Enable it in envyou → Settings → AI Integrations.",
                true,
            ));
        }

        match name {
            "list_projects" => self.call_list_projects(),
            "list_variable_names" => self.call_list_variable_names(&args),
            "read_env_variables" => self.call_read(&args),
            "write_env_variable" => self.call_write(&args),
            other => Err((-32602, format!("Unknown tool: {other}"))),
        }
    }

    fn call_list_projects(&self) -> Result<Value, (i64, String)> {
        if !self.policy.list_projects {
            return Ok(tool_text("Listing projects is disabled in envyou.", true));
        }
        let summaries = self.store.list_projects();
        Ok(tool_text(
            &serde_json::to_string_pretty(&summaries).unwrap_or_else(|_| "[]".into()),
            false,
        ))
    }

    fn call_list_variable_names(&self, args: &Value) -> Result<Value, (i64, String)> {
        if !self.policy.list_variable_names {
            return Ok(tool_text(
                "Listing variable names is disabled in envyou.",
                true,
            ));
        }
        let project_id = require_str(args, "projectId")?;
        match self.store.read_env_variables(&project_id) {
            Ok(vars) => {
                // Names + presence only — never values.
                let names: Vec<Value> = vars
                    .iter()
                    .map(|v| {
                        json!({
                            "key": v.key,
                            "hasValue": !v.value.is_empty(),
                        })
                    })
                    .collect();
                Ok(tool_text(
                    &serde_json::to_string_pretty(&json!({
                        "projectId": project_id,
                        "variables": names,
                    }))
                    .unwrap_or_else(|_| "{}".into()),
                    false,
                ))
            }
            Err(e) => Ok(tool_text(&format!("Error: {e}"), true)),
        }
    }

    fn call_read(&self, args: &Value) -> Result<Value, (i64, String)> {
        if !self.policy.read_values {
            return Ok(tool_text(
                "Reading variable values is disabled in envyou.",
                true,
            ));
        }
        let project_id = require_str(args, "projectId")?;
        let names = require_str_array(args, "names")?;

        // Scoping rules (spec hardening): the AI must name what it wants. No
        // empty-means-all, no wildcards.
        if names.is_empty() {
            return Err((
                -32602,
                "`names` must list the specific variables to read (reading all variables at once is not allowed)".to_string(),
            ));
        }
        if names.iter().any(|n| n.trim().is_empty() || n.contains('*')) {
            return Err((
                -32602,
                "`names` must be explicit variable names; wildcards and empty names are not allowed"
                    .to_string(),
            ));
        }

        let project_name = self.project_display_name(&project_id);
        let reason = optional_str(args, "reason").or_else(|| optional_str(args, "purpose"));

        // Human-in-the-loop, fail-closed.
        let outcome = self.gate.request(&ApprovalRequest {
            client: self.client_name(),
            project_id: project_id.clone(),
            project_name,
            action: ApprovalAction::ReadValues {
                names: names.clone(),
            },
            reason,
        });
        let granted = match approved_names(&outcome) {
            Some(g) => g,
            None => return Ok(denial_result(&outcome)),
        };
        if granted.is_empty() {
            return Ok(tool_text(
                "User did not approve any of the requested variables.",
                true,
            ));
        }

        // Load the project's variables, then return only those that were both
        // requested-and-granted and actually exist. Values for anything else
        // never leave the store.
        let vars = match self.store.read_env_variables(&project_id) {
            Ok(v) => v,
            Err(e) => return Ok(tool_text(&format!("Error: {e}"), true)),
        };
        let returned: Vec<&EnvVariable> = vars
            .iter()
            .filter(|v| granted.iter().any(|g| g == &v.key) && names.iter().any(|n| n == &v.key))
            .collect();
        let returned_keys: Vec<&str> = returned.iter().map(|v| v.key.as_str()).collect();
        // Names the caller asked for that were not returned (either the user did
        // not approve them, or they do not exist). We do not distinguish the two
        // to avoid turning the tool into an existence oracle.
        let omitted: Vec<&String> = names
            .iter()
            .filter(|n| !returned_keys.contains(&n.as_str()))
            .collect();

        Ok(tool_text(
            &serde_json::to_string_pretty(&json!({
                "projectId": project_id,
                "variables": returned,
                "omittedNames": omitted,
            }))
            .unwrap_or_else(|_| "{}".into()),
            false,
        ))
    }

    fn call_write(&self, args: &Value) -> Result<Value, (i64, String)> {
        // Writes are opt-in. If the user has not enabled them, refuse before any
        // dialog — and never echo the value the AI tried to set.
        if !self.policy.write_values {
            return Ok(tool_text(
                "Writing variables via AI is disabled in envyou. Enable it in Settings → AI Integrations if you want to allow this.",
                true,
            ));
        }
        let project_id = require_str(args, "projectId")?;
        let key = require_str(args, "key")?;
        let value = require_str(args, "value")?;
        let reason = optional_str(args, "reason");

        // Is this a create or an update? Used only to warn the user; the check
        // reads existence, never exposes a value.
        let creating = match self.store.read_env_variables(&project_id) {
            Ok(vars) => !vars.iter().any(|v| v.key == key),
            Err(_) => true,
        };

        let outcome = self.gate.request(&ApprovalRequest {
            client: self.client_name(),
            project_id: project_id.clone(),
            project_name: self.project_display_name(&project_id),
            action: ApprovalAction::WriteValue {
                key: key.clone(),
                creating,
            },
            reason,
        });
        let granted = match approved_names(&outcome) {
            Some(g) => g,
            None => return Ok(denial_result(&outcome)),
        };
        if !granted.iter().any(|g| g == &key) {
            return Ok(tool_text("User did not approve the write.", true));
        }

        match self.store.write_env_variable(&project_id, &key, &value) {
            // Report only the changed key — never the value that was stored.
            Ok(()) => Ok(tool_text(
                &format!("Saved `{key}` to project `{project_id}`."),
                false,
            )),
            Err(e) => Ok(tool_text(&format!("Error: {e}"), true)),
        }
    }

    fn project_display_name(&self, project_id: &str) -> String {
        self.store
            .list_projects()
            .into_iter()
            .find(|p| p.id == project_id)
            .map(|p| p.name)
            .unwrap_or_else(|| project_id.to_string())
    }
}

/// Run the MCP server over the provided reader/writer (STDIO in production).
///
/// Only JSON-RPC responses are ever written to `writer`; the caller wires
/// `writer` to stdout and keeps all logging on stderr so the protocol stream
/// stays clean.
pub fn serve_stdio<S, G, R, W>(
    server: &McpServer<S, G>,
    reader: R,
    mut writer: W,
) -> std::io::Result<()>
where
    S: EnvStore,
    G: ApprovalGate,
    R: BufRead,
    W: Write,
{
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        if let Some(resp) = server.handle_line(&line) {
            writeln!(writer, "{resp}")?;
            writer.flush()?;
        }
    }
    Ok(())
}

// ---- helpers -------------------------------------------------------------

/// Names the user approved, or `None` for any non-approval outcome (denied,
/// timeout, error — all fail-closed).
fn approved_names(outcome: &ApprovalOutcome) -> Option<Vec<String>> {
    match outcome {
        ApprovalOutcome::Approved { granted } => Some(granted.clone()),
        ApprovalOutcome::Denied | ApprovalOutcome::Timeout | ApprovalOutcome::Error => None,
    }
}

/// Structured, value-free result for a non-approval outcome.
fn denial_result(outcome: &ApprovalOutcome) -> Value {
    let msg = match outcome {
        ApprovalOutcome::Denied => "User denied the request.",
        ApprovalOutcome::Timeout => "Approval request timed out; treated as denied.",
        ApprovalOutcome::Error => {
            "Approval could not be requested (envyou approval UI unavailable); treated as denied."
        }
        // Not reachable — approved is handled by the caller.
        ApprovalOutcome::Approved { .. } => "Approved.",
    };
    tool_text(msg, true)
}

fn require_str(args: &Value, field: &str) -> Result<String, (i64, String)> {
    args.get(field)
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .ok_or((-32602, format!("Missing required argument: {field}")))
}

fn optional_str(args: &Value, field: &str) -> Option<String> {
    args.get(field)
        .and_then(Value::as_str)
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Require an array-of-strings argument. A present-but-wrong-typed value is a
/// param error; a missing value is also an error (the caller decides whether an
/// empty array is acceptable).
fn require_str_array(args: &Value, field: &str) -> Result<Vec<String>, (i64, String)> {
    match args.get(field) {
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                match it.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => return Err((-32602, format!("`{field}` must be an array of strings"))),
                }
            }
            Ok(out)
        }
        Some(_) => Err((-32602, format!("`{field}` must be an array of strings"))),
        None => Err((-32602, format!("Missing required argument: {field}"))),
    }
}

fn tool_text(text: &str, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": is_error
    })
}

fn success_response(id: Value, result: Value) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string()
}

fn error_response(id: Value, code: i64, message: &str) -> String {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }).to_string()
}

/// Tool schema definitions, filtered to the tools the policy permits.
fn tool_schemas(policy: &McpPolicy) -> Value {
    let mut tools: Vec<Value> = Vec::new();

    if policy.list_projects {
        tools.push(json!({
            "name": "list_projects",
            "description": "List the projects stored in envyou. Returns names, ids and variable counts only — never any variable values.",
            "inputSchema": { "type": "object", "properties": {} }
        }));
    }
    if policy.list_variable_names {
        tools.push(json!({
            "name": "list_variable_names",
            "description": "List the variable names in a project (with whether each has a value). Never returns values.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "projectId": { "type": "string", "description": "The project's id (from list_projects)." }
                },
                "required": ["projectId"]
            }
        }));
    }
    if policy.read_values {
        tools.push(json!({
            "name": "read_env_variables",
            "description": "Read the plaintext values of specific, named environment variables. You MUST list the exact variable names in `names` (reading everything at once and wildcards are not allowed). The user is shown every requested name and must approve; values you receive are sent through this AI client and may be processed by its provider. Only request what you need.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "projectId": { "type": "string", "description": "The project's id." },
                    "names": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "Exact variable names to read. Required and non-empty; no wildcards."
                    },
                    "reason": { "type": "string", "description": "Optional: why you need these values (shown to the user)." }
                },
                "required": ["projectId", "names"]
            }
        }));
    }
    if policy.write_values {
        tools.push(json!({
            "name": "write_env_variable",
            "description": "Create or update a single environment variable's value. Requires explicit user approval on every call.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "projectId": { "type": "string", "description": "The project's id." },
                    "key": { "type": "string", "description": "Variable name (e.g. NODE_ENV)." },
                    "value": { "type": "string", "description": "The value to store." },
                    "reason": { "type": "string", "description": "Optional: why (shown to the user)." }
                },
                "required": ["projectId", "key", "value"]
            }
        }));
    }

    Value::Array(tools)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct FakeStore {
        vars: Vec<EnvVariable>,
        writes: RefCell<Vec<(String, String, String)>>,
    }
    impl FakeStore {
        fn new() -> Self {
            Self {
                vars: vec![
                    EnvVariable {
                        key: "DATABASE_URL".into(),
                        value: "postgres://localhost/db".into(),
                        comment: None,
                        is_masked: true,
                    },
                    EnvVariable {
                        key: "API_URL".into(),
                        value: "https://api.example.com".into(),
                        comment: None,
                        is_masked: false,
                    },
                ],
                writes: RefCell::new(vec![]),
            }
        }
    }
    impl EnvStore for FakeStore {
        fn list_projects(&self) -> Vec<ProjectSummary> {
            vec![ProjectSummary {
                id: "p1".into(),
                name: "api".into(),
                variable_count: self.vars.len(),
            }]
        }
        fn read_env_variables(&self, project_id: &str) -> Result<Vec<EnvVariable>, String> {
            if project_id == "p1" {
                Ok(self.vars.clone())
            } else {
                Err("project not found".into())
            }
        }
        fn write_env_variable(&self, p: &str, k: &str, v: &str) -> Result<(), String> {
            self.writes
                .borrow_mut()
                .push((p.into(), k.into(), v.into()));
            Ok(())
        }
    }

    /// A gate that always returns a fixed outcome, recording the requests it saw.
    struct Gate {
        outcome: ApprovalOutcome,
        seen: RefCell<Vec<ApprovalRequest>>,
    }
    impl Gate {
        fn new(outcome: ApprovalOutcome) -> Self {
            Self {
                outcome,
                seen: RefCell::new(vec![]),
            }
        }
    }
    impl ApprovalGate for Gate {
        fn request(&self, req: &ApprovalRequest) -> ApprovalOutcome {
            self.seen.borrow_mut().push(req.clone());
            self.outcome.clone()
        }
    }

    /// Full-access policy for tests that exercise a specific tool.
    fn open_policy() -> McpPolicy {
        McpPolicy {
            enabled: true,
            list_projects: true,
            list_variable_names: true,
            read_values: true,
            write_values: true,
            delete_values: true,
        }
    }

    fn server_with(outcome: ApprovalOutcome, policy: McpPolicy) -> McpServer<FakeStore, Gate> {
        McpServer::new(FakeStore::new(), Gate::new(outcome), policy)
    }

    fn server(outcome: ApprovalOutcome) -> McpServer<FakeStore, Gate> {
        server_with(outcome, open_policy())
    }

    fn call(s: &McpServer<FakeStore, Gate>, line: &str) -> Value {
        serde_json::from_str(&s.handle_line(line).unwrap()).unwrap()
    }

    fn result_text(v: &Value) -> String {
        v["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .to_string()
    }

    #[test]
    fn initialize_reports_tools_capability_and_captures_client() {
        let s = server(ApprovalOutcome::Denied);
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"Claude Desktop"}}}"#,
        );
        assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(r["result"]["serverInfo"]["name"], "envyou");
        assert_eq!(s.client_name(), "Claude Desktop");
    }

    #[test]
    fn notifications_get_no_response() {
        let s = server(ApprovalOutcome::approved_all([]));
        assert!(s
            .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .is_none());
    }

    #[test]
    fn tools_list_reflects_policy() {
        // Read-only policy: no write tool advertised.
        let ro = McpPolicy {
            write_values: false,
            ..open_policy()
        };
        let s = server_with(ApprovalOutcome::Denied, ro);
        let r = call(&s, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        let names: Vec<&str> = r["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert!(names.contains(&"list_projects"));
        assert!(names.contains(&"list_variable_names"));
        assert!(names.contains(&"read_env_variables"));
        assert!(!names.contains(&"write_env_variable"), "writes are off");
    }

    #[test]
    fn disabled_master_switch_refuses_every_tool() {
        let disabled = McpPolicy {
            enabled: false,
            ..open_policy()
        };
        let s = server_with(
            ApprovalOutcome::approved_all(["DATABASE_URL".into()]),
            disabled,
        );
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_projects"}}"#,
        );
        assert_eq!(r["result"]["isError"], true);
        assert!(result_text(&r).contains("disabled"));
    }

    #[test]
    fn list_projects_needs_no_approval_and_hides_values() {
        let s = server(ApprovalOutcome::Denied); // would deny if consulted
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_projects"}}"#,
        );
        let text = result_text(&r);
        assert!(text.contains("api"));
        assert!(!text.contains("postgres"));
        assert_eq!(r["result"]["isError"], false);
        assert!(s.gate.seen.borrow().is_empty(), "list must not prompt");
    }

    #[test]
    fn list_variable_names_returns_names_not_values() {
        let s = server(ApprovalOutcome::Denied);
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_variable_names","arguments":{"projectId":"p1"}}}"#,
        );
        let text = result_text(&r);
        assert!(text.contains("DATABASE_URL"));
        assert!(text.contains("hasValue"));
        assert!(
            !text.contains("postgres://localhost/db"),
            "must not leak values"
        );
        assert!(s.gate.seen.borrow().is_empty());
    }

    #[test]
    fn list_variable_names_can_be_disabled() {
        let p = McpPolicy {
            list_variable_names: false,
            ..open_policy()
        };
        let s = server_with(ApprovalOutcome::Denied, p);
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_variable_names","arguments":{"projectId":"p1"}}}"#,
        );
        assert_eq!(r["result"]["isError"], true);
    }

    #[test]
    fn read_requires_names_argument() {
        let s = server(ApprovalOutcome::approved_all(["DATABASE_URL".into()]));
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"read_env_variables","arguments":{"projectId":"p1"}}}"#,
        );
        assert_eq!(r["error"]["code"], -32602);
    }

    #[test]
    fn read_rejects_empty_names() {
        let s = server(ApprovalOutcome::approved_all([]));
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"read_env_variables","arguments":{"projectId":"p1","names":[]}}}"#,
        );
        assert_eq!(r["error"]["code"], -32602);
        // The gate must never even be consulted for an invalid request.
        assert!(s.gate.seen.borrow().is_empty());
    }

    #[test]
    fn read_rejects_wildcards() {
        let s = server(ApprovalOutcome::approved_all(["*".into()]));
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"read_env_variables","arguments":{"projectId":"p1","names":["*"]}}}"#,
        );
        assert_eq!(r["error"]["code"], -32602);
        assert!(s.gate.seen.borrow().is_empty());
    }

    #[test]
    fn read_returns_only_named_and_approved_values() {
        let s = server(ApprovalOutcome::approved_all(["DATABASE_URL".into()]));
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"read_env_variables","arguments":{"projectId":"p1","names":["DATABASE_URL"]}}}"#,
        );
        let text = result_text(&r);
        assert!(text.contains("postgres://localhost/db"));
        // API_URL was neither requested nor approved: its value must not appear.
        assert!(!text.contains("api.example.com"));
        // The approval request carried the exact names.
        let seen = s.gate.seen.borrow();
        assert_eq!(seen.len(), 1);
        match &seen[0].action {
            ApprovalAction::ReadValues { names } => {
                assert_eq!(names, &vec!["DATABASE_URL".to_string()])
            }
            _ => panic!("expected a read approval"),
        }
    }

    #[test]
    fn read_partial_approval_returns_only_the_granted_subset() {
        // The AI asks for two; the user grants only one.
        let s = server(ApprovalOutcome::approved_all(["API_URL".into()]));
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"read_env_variables","arguments":{"projectId":"p1","names":["DATABASE_URL","API_URL"]}}}"#,
        );
        let text = result_text(&r);
        assert!(text.contains("api.example.com"), "granted value returned");
        assert!(
            !text.contains("postgres://localhost/db"),
            "ungranted value withheld"
        );
        assert!(
            text.contains("DATABASE_URL"),
            "withheld name reported as omitted"
        );
    }

    #[test]
    fn read_denied_returns_no_values() {
        let s = server(ApprovalOutcome::Denied);
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"read_env_variables","arguments":{"projectId":"p1","names":["DATABASE_URL"]}}}"#,
        );
        assert_eq!(r["result"]["isError"], true);
        let text = result_text(&r);
        assert!(text.to_lowercase().contains("denied"));
        assert!(!text.contains("postgres"));
    }

    #[test]
    fn read_timeout_is_fail_closed() {
        let s = server(ApprovalOutcome::Timeout);
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"read_env_variables","arguments":{"projectId":"p1","names":["DATABASE_URL"]}}}"#,
        );
        assert_eq!(r["result"]["isError"], true);
        assert!(result_text(&r).to_lowercase().contains("timed out"));
        assert!(!result_text(&r).contains("postgres"));
    }

    #[test]
    fn read_gate_error_is_fail_closed() {
        let s = server(ApprovalOutcome::Error);
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"read_env_variables","arguments":{"projectId":"p1","names":["DATABASE_URL"]}}}"#,
        );
        assert_eq!(r["result"]["isError"], true);
        assert!(!result_text(&r).contains("postgres"));
    }

    #[test]
    fn read_disabled_by_policy() {
        let p = McpPolicy {
            read_values: false,
            ..open_policy()
        };
        let s = server_with(ApprovalOutcome::approved_all(["DATABASE_URL".into()]), p);
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"read_env_variables","arguments":{"projectId":"p1","names":["DATABASE_URL"]}}}"#,
        );
        assert_eq!(r["result"]["isError"], true);
        assert!(
            s.gate.seen.borrow().is_empty(),
            "disabled read must not prompt"
        );
    }

    #[test]
    fn write_disabled_by_default_policy_and_does_not_leak_value() {
        // read/list on, writes off — the realistic default.
        let p = McpPolicy {
            write_values: false,
            ..open_policy()
        };
        let s = server_with(ApprovalOutcome::approved_all(["K".into()]), p);
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"write_env_variable","arguments":{"projectId":"p1","key":"K","value":"leak-me"}}}"#,
        );
        assert_eq!(r["result"]["isError"], true);
        assert!(s.store.writes.borrow().is_empty(), "nothing persisted");
        assert!(
            s.gate.seen.borrow().is_empty(),
            "no dialog for a disabled tool"
        );
        assert!(
            !result_text(&r).contains("leak-me"),
            "value must not be echoed"
        );
    }

    #[test]
    fn write_denied_does_not_persist_or_leak() {
        let s = server(ApprovalOutcome::Denied);
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"write_env_variable","arguments":{"projectId":"p1","key":"AWS_SECRET_KEY","value":"leak-me"}}}"#,
        );
        assert_eq!(r["result"]["isError"], true);
        assert!(s.store.writes.borrow().is_empty());
        assert!(!result_text(&r).contains("leak-me"));
    }

    #[test]
    fn write_persists_when_enabled_and_approved_without_echoing_value() {
        let s = server(ApprovalOutcome::approved_all(["AWS_SECRET_KEY".into()]));
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"write_env_variable","arguments":{"projectId":"p1","key":"AWS_SECRET_KEY","value":"xyz-secret"}}}"#,
        );
        assert_eq!(r["result"]["isError"], false);
        assert_eq!(s.store.writes.borrow().len(), 1);
        assert_eq!(s.store.writes.borrow()[0].1, "AWS_SECRET_KEY");
        // The confirmation names the key but never the value.
        assert!(!result_text(&r).contains("xyz-secret"));
        // The approval request flagged this as a create (key not pre-existing).
        let seen = s.gate.seen.borrow();
        match &seen[0].action {
            ApprovalAction::WriteValue { key, creating } => {
                assert_eq!(key, "AWS_SECRET_KEY");
                assert!(*creating);
            }
            _ => panic!("expected a write approval"),
        }
    }

    #[test]
    fn write_timeout_is_fail_closed() {
        let s = server(ApprovalOutcome::Timeout);
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"write_env_variable","arguments":{"projectId":"p1","key":"K","value":"v"}}}"#,
        );
        assert_eq!(r["result"]["isError"], true);
        assert!(s.store.writes.borrow().is_empty());
    }

    #[test]
    fn approval_request_names_the_client() {
        let s = server(ApprovalOutcome::Denied);
        // Learn the client from initialize first.
        let _ = s.handle_line(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"clientInfo":{"name":"Claude Code"}}}"#,
        );
        let _ = call(
            &s,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_env_variables","arguments":{"projectId":"p1","names":["DATABASE_URL"]}}}"#,
        );
        assert_eq!(s.gate.seen.borrow()[0].client, "Claude Code");
        assert_eq!(s.gate.seen.borrow()[0].project_name, "api");
    }

    #[test]
    fn missing_required_arg_errors() {
        let s = server(ApprovalOutcome::approved_all([]));
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"list_variable_names","arguments":{}}}"#,
        );
        assert_eq!(r["error"]["code"], -32602);
    }

    #[test]
    fn unknown_tool_errors() {
        let s = server(ApprovalOutcome::approved_all([]));
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"bogus"}}"#,
        );
        assert_eq!(r["error"]["code"], -32602);
    }

    #[test]
    fn unknown_method_errors() {
        let s = server(ApprovalOutcome::approved_all([]));
        let r = call(&s, r#"{"jsonrpc":"2.0","id":9,"method":"bogus"}"#);
        assert_eq!(r["error"]["code"], -32601);
    }

    #[test]
    fn malformed_json_is_parse_error() {
        let s = server(ApprovalOutcome::approved_all([]));
        let r = call(&s, "{not json");
        assert_eq!(r["error"]["code"], -32700);
    }

    #[test]
    fn responses_are_single_line_for_stdio_framing() {
        // Each response must be one newline-delimited JSON object; an embedded
        // newline would corrupt the STDIO framing Claude relies on.
        let s = server(ApprovalOutcome::approved_all(["DATABASE_URL".into()]));
        let out = s
            .handle_line(r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"read_env_variables","arguments":{"projectId":"p1","names":["DATABASE_URL"]}}}"#)
            .unwrap();
        assert!(!out.contains('\n'), "response must not contain a newline");
    }

    #[test]
    fn serve_stdio_processes_multiple_lines() {
        let s = server(ApprovalOutcome::approved_all([]));
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
            "\n"
        );
        let mut out = Vec::new();
        serve_stdio(&s, std::io::Cursor::new(input), &mut out).unwrap();
        let out = String::from_utf8(out).unwrap();
        // Two responses (the notification produced none).
        assert_eq!(out.lines().count(), 2);
    }
}
