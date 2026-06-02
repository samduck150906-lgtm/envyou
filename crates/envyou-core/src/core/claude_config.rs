//! Claude Desktop auto-configuration (spec §5).
//!
//! One click writes an `envyou` entry into the user's
//! `claude_desktop_config.json` under `mcpServers`, *merging* with any existing
//! servers rather than overwriting them.

use std::path::PathBuf;

use serde_json::{json, Map, Value};

use crate::error::{Error, Result};

/// The server key inserted under `mcpServers`.
pub const SERVER_KEY: &str = "envyou";

/// Resolve the Claude Desktop config path for the current OS (spec §5.1).
///
/// Returns `None` on unsupported platforms (e.g. Linux, where Claude Desktop
/// has no official config location).
pub fn config_path() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        directories::BaseDirs::new().map(|b| {
            b.home_dir()
                .join("Library/Application Support/Claude/claude_desktop_config.json")
        })
    }
    #[cfg(target_os = "windows")]
    {
        std::env::var_os("APPDATA")
            .map(|appdata| PathBuf::from(appdata).join("Claude").join("claude_desktop_config.json"))
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        None
    }
}

/// Build the `envyou` MCP server entry pointing at the given executable path
/// (spec §5.2 step 4 — the absolute path of the current binary).
pub fn server_entry(executable_path: &str) -> Value {
    json!({
        "command": executable_path,
        "args": ["--mcp"],
        "env": {}
    })
}

/// Merge the envyou server entry into an existing config document.
///
/// `existing` is the parsed contents of `claude_desktop_config.json`, or `None`
/// when the file does not exist yet (spec §5.2 step 1). Returns the new,
/// fully-merged document. Pre-existing servers are preserved untouched
/// (spec §5.2 — non-destructive merge).
pub fn merge_config(existing: Option<Value>, executable_path: &str) -> Result<Value> {
    let mut root: Map<String, Value> = match existing {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m,
        Some(_) => {
            return Err(Error::Config(
                "claude_desktop_config.json root is not a JSON object".into(),
            ))
        }
    };

    let servers = root
        .entry("mcpServers".to_string())
        .or_insert_with(|| Value::Object(Map::new()));

    let servers_obj = servers
        .as_object_mut()
        .ok_or_else(|| Error::Config("`mcpServers` is not a JSON object".into()))?;

    servers_obj.insert(SERVER_KEY.to_string(), server_entry(executable_path));

    Ok(Value::Object(root))
}

/// Convenience: parse a raw config string (or `None`), merge, and return the
/// pretty-printed JSON to write back.
pub fn merge_config_str(existing_raw: Option<&str>, executable_path: &str) -> Result<String> {
    let existing = match existing_raw {
        Some(s) if !s.trim().is_empty() => Some(
            serde_json::from_str(s)
                .map_err(|e| Error::Config(format!("existing config is not valid JSON: {e}")))?,
        ),
        _ => None,
    };
    let merged = merge_config(existing, executable_path)?;
    serde_json::to_string_pretty(&merged).map_err(Error::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_fresh_config_when_absent() {
        let out = merge_config(None, "/Applications/envyou.app/Contents/MacOS/envyou").unwrap();
        assert_eq!(
            out["mcpServers"]["envyou"]["command"],
            "/Applications/envyou.app/Contents/MacOS/envyou"
        );
        assert_eq!(out["mcpServers"]["envyou"]["args"][0], "--mcp");
    }

    #[test]
    fn preserves_existing_servers() {
        let existing = json!({
            "mcpServers": {
                "filesystem": { "command": "npx", "args": ["fs-server"] }
            },
            "someOtherSetting": true
        });
        let out = merge_config(Some(existing), "/usr/local/bin/envyou").unwrap();
        // existing server untouched
        assert_eq!(out["mcpServers"]["filesystem"]["command"], "npx");
        // unrelated settings untouched
        assert_eq!(out["someOtherSetting"], true);
        // envyou added
        assert_eq!(out["mcpServers"]["envyou"]["command"], "/usr/local/bin/envyou");
    }

    #[test]
    fn updates_existing_envyou_entry() {
        let existing = json!({
            "mcpServers": { "envyou": { "command": "/old/path", "args": [] } }
        });
        let out = merge_config(Some(existing), "/new/path/envyou").unwrap();
        assert_eq!(out["mcpServers"]["envyou"]["command"], "/new/path/envyou");
    }

    #[test]
    fn rejects_non_object_root() {
        assert!(merge_config(Some(json!([1, 2, 3])), "/bin/envyou").is_err());
    }

    #[test]
    fn str_helper_handles_empty_input() {
        let out = merge_config_str(Some("   "), "/bin/envyou").unwrap();
        assert!(out.contains("envyou"));
        assert!(out.contains("--mcp"));
    }
}
