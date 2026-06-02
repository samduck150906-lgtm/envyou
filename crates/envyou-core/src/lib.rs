//! # envyou-core
//!
//! GUI-free core for **envyou** — a lightweight, retro, local-only environment
//! variable manager (see the product specification). This crate is deliberately
//! free of any Tauri/UI dependency so it can be unit-tested in isolation and
//! reused by both the desktop binary and the `--mcp` server runtime.
//!
//! Modules:
//! - [`core::model`] — the `EnvYouLocalState` data model (spec §7)
//! - [`core::crypto`] — AES-256-GCM envelope encryption (spec §2.1)
//! - [`core::storage`] — encrypted `enc_state.json` persistence (spec §1.2, §7)
//! - [`core::license`] — offline license activation/verification (spec §6.3)
//! - [`core::claude_config`] — Claude Desktop config merge utility (spec §5)
//! - [`mcp`] — the Model Context Protocol server (spec §4)

pub mod core;
pub mod error;
pub mod mcp;

pub use error::{Error, Result};
