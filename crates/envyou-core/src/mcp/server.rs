//! JSON-RPC 2.0 / MCP request handling.

use std::io::{BufRead, Write};

use serde_json::{json, Value};

use crate::core::model::{EnvVariable, ProjectSummary};

/// Protocol version advertised during `initialize`.
const PROTOCOL_VERSION: &str = "2024-11-05";

/// What an env-variable action targets — passed to the approval gate so the
/// user sees exactly what the AI is asking for (spec §4.1).
#[derive(Debug, Clone, PartialEq)]
pub enum ApprovalRequest {
    Read { project_id: String },
    Write { project_id: String, key: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approved,
    Denied,
}

/// Human-in-the-loop gate. The real implementation pops an 80s-style modal on
/// the desktop and blocks until the user clicks Yes/No (spec §4.1).
pub trait ApprovalGate {
    fn request(&self, req: &ApprovalRequest) -> ApprovalDecision;
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
}

impl<S: EnvStore, G: ApprovalGate> McpServer<S, G> {
    pub fn new(store: S, gate: G) -> Self {
        Self { store, gate }
    }

    /// Handle a single JSON-RPC request line. Returns `Some(response_json)` for
    /// requests, or `None` for notifications (which take no response).
    pub fn handle_line(&self, line: &str) -> Option<String> {
        let req: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => return Some(error_response(Value::Null, -32700, "Parse error")),
        };

        let id = req.get("id").cloned();
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");

        // Notifications have no `id` and expect no response.
        if id.is_none() {
            return None;
        }
        let id = id.unwrap();

        let result = match method {
            "initialize" => Ok(self.initialize()),
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

    fn initialize(&self) -> Value {
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "envyou", "version": env!("CARGO_PKG_VERSION") }
        })
    }

    fn tools_list(&self) -> Value {
        json!({ "tools": tool_schemas() })
    }

    fn tools_call(&self, params: Option<&Value>) -> Result<Value, (i64, String)> {
        let params = params.ok_or((-32602, "Missing params".to_string()))?;
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or((-32602, "Missing tool name".to_string()))?;
        let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));

        match name {
            "list_projects" => Ok(self.call_list_projects()),
            "read_env_variables" => self.call_read(&args),
            "write_env_variable" => self.call_write(&args),
            other => Err((-32602, format!("Unknown tool: {other}"))),
        }
    }

    fn call_list_projects(&self) -> Value {
        let summaries = self.store.list_projects();
        tool_text(
            &serde_json::to_string_pretty(&summaries).unwrap_or_else(|_| "[]".into()),
            false,
        )
    }

    fn call_read(&self, args: &Value) -> Result<Value, (i64, String)> {
        let project_id = require_str(args, "projectId")?;

        // Human-in-the-loop gate (spec §4.1): block until the user approves.
        let decision = self.gate.request(&ApprovalRequest::Read {
            project_id: project_id.clone(),
        });
        if decision == ApprovalDecision::Denied {
            return Ok(tool_text(
                "User denied access to environment variables.",
                true,
            ));
        }

        match self.store.read_env_variables(&project_id) {
            Ok(vars) => Ok(tool_text(
                &serde_json::to_string_pretty(&vars).unwrap_or_else(|_| "[]".into()),
                false,
            )),
            Err(e) => Ok(tool_text(&format!("Error: {e}"), true)),
        }
    }

    fn call_write(&self, args: &Value) -> Result<Value, (i64, String)> {
        let project_id = require_str(args, "projectId")?;
        let key = require_str(args, "key")?;
        let value = require_str(args, "value")?;

        let decision = self.gate.request(&ApprovalRequest::Write {
            project_id: project_id.clone(),
            key: key.clone(),
        });
        if decision == ApprovalDecision::Denied {
            return Ok(tool_text("User denied the write request.", true));
        }

        match self.store.write_env_variable(&project_id, &key, &value) {
            Ok(()) => Ok(tool_text(
                &format!("Saved `{key}` to project `{project_id}`."),
                false,
            )),
            Err(e) => Ok(tool_text(&format!("Error: {e}"), true)),
        }
    }
}

/// Run the MCP server over the provided reader/writer (STDIO in production).
pub fn serve_stdio<S, G, R, W>(server: &McpServer<S, G>, reader: R, mut writer: W) -> std::io::Result<()>
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

fn require_str(args: &Value, field: &str) -> Result<String, (i64, String)> {
    args.get(field)
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .ok_or((-32602, format!("Missing required argument: {field}")))
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

/// Tool schema definitions (spec §4.2).
fn tool_schemas() -> Value {
    json!([
        {
            "name": "list_projects",
            "description": "envyou 앱에 등록된 모든 프로젝트 및 사이트 카테고리 목록을 반환합니다.",
            "inputSchema": { "type": "object", "properties": {} }
        },
        {
            "name": "read_env_variables",
            "description": "지정한 프로젝트 ID에 저장된 전체 .env 변수 리스트를 가져옵니다. 호출 시 사용자 승인 팝업이 발생합니다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "projectId": { "type": "string", "description": "조회할 프로젝트의 고유 식별자" }
                },
                "required": ["projectId"]
            }
        },
        {
            "name": "write_env_variable",
            "description": "지정한 프로젝트에 새로운 환경변수 키-값을 삽입하거나 기존 값을 업데이트합니다. 사용자 승인이 필수적입니다.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "projectId": { "type": "string", "description": "대상 프로젝트 ID" },
                    "key": { "type": "string", "description": "환경변수 이름 (예: DATABASE_URL)" },
                    "value": { "type": "string", "description": "설정할 환경변수 값 명세" }
                },
                "required": ["projectId", "key", "value"]
            }
        }
    ])
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
                vars: vec![EnvVariable {
                    key: "DATABASE_URL".into(),
                    value: "postgres://localhost/db".into(),
                    comment: None,
                    is_masked: true,
                }],
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
            self.writes.borrow_mut().push((p.into(), k.into(), v.into()));
            Ok(())
        }
    }

    struct Gate(ApprovalDecision);
    impl ApprovalGate for Gate {
        fn request(&self, _: &ApprovalRequest) -> ApprovalDecision {
            self.0
        }
    }

    fn server(decision: ApprovalDecision) -> McpServer<FakeStore, Gate> {
        McpServer::new(FakeStore::new(), Gate(decision))
    }

    fn call(s: &McpServer<FakeStore, Gate>, line: &str) -> Value {
        serde_json::from_str(&s.handle_line(line).unwrap()).unwrap()
    }

    #[test]
    fn initialize_reports_tools_capability() {
        let s = server(ApprovalDecision::Approved);
        let r = call(&s, r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#);
        assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSION);
        assert_eq!(r["result"]["serverInfo"]["name"], "envyou");
    }

    #[test]
    fn notifications_get_no_response() {
        let s = server(ApprovalDecision::Approved);
        assert!(s
            .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .is_none());
    }

    #[test]
    fn tools_list_has_three_tools() {
        let s = server(ApprovalDecision::Approved);
        let r = call(&s, r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#);
        let tools = r["result"]["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 3);
        let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert!(names.contains(&"list_projects"));
        assert!(names.contains(&"read_env_variables"));
        assert!(names.contains(&"write_env_variable"));
    }

    #[test]
    fn list_projects_does_not_require_approval() {
        let s = server(ApprovalDecision::Denied); // gate would deny if consulted
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_projects"}}"#,
        );
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("api"));
        assert_eq!(r["result"]["isError"], false);
    }

    #[test]
    fn read_requires_approval_and_returns_values_when_approved() {
        let s = server(ApprovalDecision::Approved);
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"read_env_variables","arguments":{"projectId":"p1"}}}"#,
        );
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("postgres://localhost/db"));
    }

    #[test]
    fn read_blocked_when_denied() {
        let s = server(ApprovalDecision::Denied);
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"read_env_variables","arguments":{"projectId":"p1"}}}"#,
        );
        assert_eq!(r["result"]["isError"], true);
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(text.to_lowercase().contains("denied"));
        assert!(!text.contains("postgres"));
    }

    #[test]
    fn write_requires_approval() {
        let s = server(ApprovalDecision::Denied);
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"write_env_variable","arguments":{"projectId":"p1","key":"K","value":"V"}}}"#,
        );
        assert_eq!(r["result"]["isError"], true);
    }

    #[test]
    fn write_denied_does_not_persist_anything() {
        let s = server(ApprovalDecision::Denied);
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"write_env_variable","arguments":{"projectId":"p1","key":"AWS_SECRET_KEY","value":"leak-me"}}}"#,
        );
        assert_eq!(r["result"]["isError"], true);
        // The store must not have been mutated when the user denied the write.
        assert!(
            s.store.writes.borrow().is_empty(),
            "a denied write must not reach the store"
        );
        // And the denied secret value must never appear in the response.
        let text = r["result"]["content"][0]["text"].as_str().unwrap();
        assert!(!text.contains("leak-me"));
    }

    #[test]
    fn write_persists_when_approved() {
        let s = server(ApprovalDecision::Approved);
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"write_env_variable","arguments":{"projectId":"p1","key":"AWS_SECRET_KEY","value":"xyz"}}}"#,
        );
        assert_eq!(r["result"]["isError"], false);
        assert_eq!(s.store.writes.borrow().len(), 1);
        assert_eq!(s.store.writes.borrow()[0].1, "AWS_SECRET_KEY");
    }

    #[test]
    fn missing_required_arg_errors() {
        let s = server(ApprovalDecision::Approved);
        let r = call(
            &s,
            r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"read_env_variables","arguments":{}}}"#,
        );
        assert_eq!(r["error"]["code"], -32602);
    }

    #[test]
    fn unknown_method_errors() {
        let s = server(ApprovalDecision::Approved);
        let r = call(&s, r#"{"jsonrpc":"2.0","id":9,"method":"bogus"}"#);
        assert_eq!(r["error"]["code"], -32601);
    }

    #[test]
    fn malformed_json_is_parse_error() {
        let s = server(ApprovalDecision::Approved);
        let r = call(&s, "{not json");
        assert_eq!(r["error"]["code"], -32700);
    }

    #[test]
    fn serve_stdio_processes_multiple_lines() {
        let s = server(ApprovalDecision::Approved);
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
        // Two responses (notification produced none).
        assert_eq!(out.lines().count(), 2);
    }
}
