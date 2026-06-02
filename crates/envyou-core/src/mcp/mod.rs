//! Model Context Protocol (MCP) server (spec §4).
//!
//! Implements a minimal JSON-RPC 2.0 server exposing three tools
//! (`list_projects`, `read_env_variables`, `write_env_variable`). The
//! transport-agnostic [`McpServer::handle_line`] makes the protocol logic
//! fully unit-testable; [`serve_stdio`] wires it to STDIO for the
//! `envyou --mcp` runtime mode (spec §2.2).

mod server;

pub use server::{serve_stdio, ApprovalDecision, ApprovalGate, ApprovalRequest, EnvStore, McpServer};
