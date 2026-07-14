//! Model Context Protocol (MCP) server (spec §4).
//!
//! Implements a JSON-RPC 2.0 server exposing four tools (`list_projects`,
//! `list_variable_names`, `read_env_variables`, `write_env_variable`), each
//! gated by an [`McpPolicy`] the user controls and, for anything that reveals or
//! changes a value, a fail-closed [`ApprovalGate`]. The transport-agnostic
//! [`McpServer::handle_line`] makes the protocol logic fully unit-testable;
//! [`serve_stdio`] wires it to STDIO for the `envyou --mcp` runtime mode
//! (spec §2.2).

pub mod audit;
pub mod sensitivity;
mod server;

pub use audit::{
    clear_audit_log, read_audit_jsonl, AuditEvent, AuditOutcome, AuditRecord, AuditSink, AuditTool,
    FileAuditSink,
};
pub use sensitivity::is_sensitive_name;
pub use server::{
    serve_stdio, ApprovalAction, ApprovalGate, ApprovalOutcome, ApprovalRequest, EnvStore,
    McpPolicy, McpServer,
};
