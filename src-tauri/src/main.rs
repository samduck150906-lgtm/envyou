// Prevent an extra console window on Windows in release (GUI mode only).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Spec §2.2: the same binary runs in two modes depending on CLI flags.
    //   * `envyou --mcp` → headless STDIO MCP server for Claude Desktop.
    //   * `envyou`       → the retro floating GUI (default).
    if std::env::args().any(|a| a == "--mcp") {
        if let Err(e) = envyou_lib::mcp_runtime::run_stdio() {
            eprintln!("envyou MCP server error: {e}");
            std::process::exit(1);
        }
    } else {
        envyou_lib::run();
    }
}
