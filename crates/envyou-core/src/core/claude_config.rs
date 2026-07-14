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
        std::env::var_os("APPDATA").map(|appdata| {
            PathBuf::from(appdata)
                .join("Claude")
                .join("claude_desktop_config.json")
        })
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

/// Remove **only** the envyou entry from an existing config, leaving every other
/// server and top-level setting untouched (the disconnect counterpart to
/// [`merge_config_str`]).
///
/// Returns:
/// - `Ok(Some(json))` — envyou was present; this is the new document to write.
/// - `Ok(None)` — nothing to do (no file, empty file, or envyou wasn't there),
///   so the caller should not rewrite the file at all.
/// - `Err(..)` — the existing config isn't valid JSON / isn't a JSON object, so
///   we refuse to touch it rather than risk clobbering a file we don't
///   understand.
pub fn remove_server_str(existing_raw: Option<&str>) -> Result<Option<String>> {
    let raw = match existing_raw {
        Some(s) if !s.trim().is_empty() => s,
        _ => return Ok(None),
    };
    let value: Value = serde_json::from_str(raw)
        .map_err(|e| Error::Config(format!("existing config is not valid JSON: {e}")))?;
    let mut root = match value {
        Value::Object(m) => m,
        _ => {
            return Err(Error::Config(
                "claude_desktop_config.json root is not a JSON object".into(),
            ))
        }
    };

    let removed = match root.get_mut("mcpServers") {
        Some(Value::Object(servers)) => servers.remove(SERVER_KEY).is_some(),
        _ => false,
    };
    if !removed {
        return Ok(None);
    }
    let out = serde_json::to_string_pretty(&Value::Object(root)).map_err(Error::from)?;
    Ok(Some(out))
}

/// The argument vector for registering envyou with **Claude Code**, to be passed
/// to the `claude` CLI as an **argv array** — never concatenated into a shell
/// string. Passing argv directly is what makes an executable path containing
/// spaces or shell metacharacters safe: there is no shell to reinterpret it, so
/// there is nothing to inject.
///
/// Equivalent to: `claude mcp add --transport stdio envyou -- <exe> --mcp`
pub fn claude_code_add_args(executable_path: &str) -> Vec<String> {
    vec![
        "mcp".into(),
        "add".into(),
        "--transport".into(),
        "stdio".into(),
        SERVER_KEY.into(),
        "--".into(),
        executable_path.into(),
        "--mcp".into(),
    ]
}

/// A copy-pasteable `claude mcp add …` command for the UI. The executable path
/// is POSIX-single-quoted when it contains anything outside a safe set, so a
/// user can paste it verbatim into a shell. This is **display only** — the app
/// itself runs the argv form ([`claude_code_add_args`]) with no shell involved.
pub fn claude_code_add_command(executable_path: &str) -> String {
    format!(
        "claude mcp add --transport stdio {SERVER_KEY} -- {} --mcp",
        posix_shell_quote(executable_path)
    )
}

/// Single-quote a string for POSIX shells if it isn't already a "safe word".
fn posix_shell_quote(s: &str) -> String {
    let safe = !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '/'));
    if safe {
        s.to_string()
    } else {
        // Close the quote, emit an escaped literal quote, reopen — the standard
        // POSIX trick for embedding a single quote inside a single-quoted string.
        format!("'{}'", s.replace('\'', r"'\''"))
    }
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
        assert_eq!(
            out["mcpServers"]["envyou"]["command"],
            "/usr/local/bin/envyou"
        );
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

    #[test]
    fn remove_takes_out_only_envyou() {
        let existing = json!({
            "mcpServers": {
                "filesystem": { "command": "npx", "args": ["fs-server"] },
                "envyou": { "command": "/old/envyou", "args": ["--mcp"] }
            },
            "someOtherSetting": true
        })
        .to_string();
        let out = remove_server_str(Some(&existing))
            .unwrap()
            .expect("should change");
        let v: Value = serde_json::from_str(&out).unwrap();
        // envyou gone, everything else intact.
        assert!(v["mcpServers"].get("envyou").is_none());
        assert_eq!(v["mcpServers"]["filesystem"]["command"], "npx");
        assert_eq!(v["someOtherSetting"], true);
    }

    #[test]
    fn remove_is_noop_when_envyou_absent_or_no_file() {
        // Not present -> None (don't rewrite the file).
        let existing = json!({ "mcpServers": { "filesystem": { "command": "npx" } } }).to_string();
        assert!(remove_server_str(Some(&existing)).unwrap().is_none());
        // No/empty file -> None.
        assert!(remove_server_str(None).unwrap().is_none());
        assert!(remove_server_str(Some("   ")).unwrap().is_none());
    }

    #[test]
    fn remove_rejects_invalid_or_non_object_config() {
        assert!(remove_server_str(Some("{not json")).is_err());
        assert!(remove_server_str(Some("[1,2,3]")).is_err());
    }

    #[test]
    fn claude_code_args_are_an_argv_array_not_a_shell_string() {
        let args = claude_code_add_args("/Applications/envyou.app/Contents/MacOS/envyou");
        assert_eq!(
            args,
            vec![
                "mcp",
                "add",
                "--transport",
                "stdio",
                "envyou",
                "--",
                "/Applications/envyou.app/Contents/MacOS/envyou",
                "--mcp",
            ]
        );
        // The path is a single element — nothing to be reinterpreted by a shell.
        let inject = claude_code_add_args("/tmp/eviltool; rm -rf ~");
        assert!(inject.contains(&"/tmp/eviltool; rm -rf ~".to_string()));
        assert_eq!(inject.iter().filter(|a| a.contains("rm -rf")).count(), 1);
    }

    #[test]
    fn claude_code_command_quotes_unsafe_paths_only() {
        // A clean path is left unquoted.
        assert_eq!(
            claude_code_add_command("/usr/local/bin/envyou"),
            "claude mcp add --transport stdio envyou -- /usr/local/bin/envyou --mcp"
        );
        // A path with a space is single-quoted so it pastes safely.
        let cmd = claude_code_add_command("/Users/dev/My Apps/envyou");
        assert!(cmd.contains("'/Users/dev/My Apps/envyou'"));
        // A path with a single quote is escaped, not left to break the quoting.
        let cmd = claude_code_add_command("/a/b'c/envyou");
        assert!(cmd.contains(r"'\''"));
    }
}
