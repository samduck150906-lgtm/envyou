# envyou

> 개발자를 위한 초경량 레트로 환경변수 관리 데스크톱 앱
> A lightweight, retro (80s Windows style), **local-only** environment-variable
> manager with native Claude Desktop / MCP integration.

This repository is an MVP implementation of the [product specification](#)
(`envyou` PRD v1.0.0). It is built on **Tauri v2 (Rust + Vanilla JS)** and
ships a GUI-free, fully unit-tested Rust core.

---

## Highlights

| Area | What's implemented |
| --- | --- |
| **Zero-cloud storage** | All state lives in a single **AES-256-GCM** encrypted file (`enc_state.json`). Nothing is ever sent off-machine. |
| **Data model** | Full `EnvYouLocalState` model (projects, variables, settings, license) — spec §7. |
| **MCP server** | `envyou --mcp` runs a JSON-RPC 2.0 MCP server over STDIO exposing `list_projects`, `read_env_variables`, `write_env_variable` — spec §4. |
| **Human-in-the-loop** | `read`/`write` tool calls block on a native OS approval dialog before any secret is released — spec §4.1. |
| **Claude Desktop linking** | One click merges an `envyou` entry into `claude_desktop_config.json` non-destructively — spec §5. |
| **Retro UI** | Vanilla HTML/CSS/JS floating window: hard pixel bevels, navy title bar, classic gray, always-on-top pin — spec §3. |
| **Freemium** | Free tier caps (3 projects / 10 vars) enforced; Pro unlock via offline, hardware-bound license key — spec §6. |

---

## Architecture

```
envyou/
├── Cargo.toml                  # Cargo workspace
├── crates/
│   └── envyou-core/            # ⭐ Pure Rust, NO GUI deps — fully unit-tested
│       └── src/
│           ├── core/
│           │   ├── model.rs        # EnvYouLocalState data model (§7)
│           │   ├── crypto.rs       # AES-256-GCM envelope (§2.1)
│           │   ├── storage.rs      # encrypted enc_state.json (§1.2, §7)
│           │   ├── license.rs      # offline license activation (§6.3)
│           │   └── claude_config.rs# Claude Desktop merge utility (§5)
│           └── mcp/server.rs       # JSON-RPC 2.0 MCP server (§4)
├── src-tauri/                  # Tauri desktop shell (GUI + --mcp runtime)
│   ├── src/
│   │   ├── main.rs             # CLI mode switch: GUI vs --mcp (§2.2)
│   │   ├── lib.rs             # Tauri builder, tray, commands
│   │   ├── commands.rs        # GUI ↔ store commands
│   │   └── mcp_runtime.rs     # wires core MCP server to store + native dialog
│   ├── tauri.conf.json
│   └── capabilities/default.json
└── src/                        # Vanilla JS retro frontend
    ├── index.html
    ├── styles/retro.css
    └── js/{api.js, app.js}
```

The **`envyou-core`** crate intentionally has zero Tauri/UI dependencies so the
security-critical logic (crypto, storage, MCP, licensing) can be tested in
isolation and reused by both runtime modes.

---

## Develop & test

The core crate builds and tests anywhere with a Rust toolchain — no system GUI
libraries required:

```bash
cargo test -p envyou-core      # 33 tests: crypto, storage, MCP, license, config
```

### Building the desktop app

The Tauri shell needs the usual platform WebView/GTK build dependencies
(see <https://v2.tauri.app/start/prerequisites/>). On Linux you'll need
`webkit2gtk`, `libglib`, etc. Once installed:

```bash
cargo tauri dev      # run the GUI (requires `cargo install tauri-cli`)
cargo tauri build    # produce a release bundle
```

### UI preview without compiling

`src/index.html` runs in a plain browser too: `js/api.js` falls back to a
`localStorage`-backed mock that mirrors the backend, so you can click through the
retro UI without building the Rust shell.

---

## MCP / Claude Desktop integration

1. In **Settings → Link with Claude Desktop**, envyou writes its server entry
   into `claude_desktop_config.json`:

   ```json
   {
     "mcpServers": {
       "envyou": {
         "command": "/Applications/envyou.app/Contents/MacOS/envyou",
         "args": ["--mcp"],
         "env": {}
       }
     }
   }
   ```

2. Claude Desktop then launches `envyou --mcp`, which speaks MCP over STDIO.
3. When Claude calls `read_env_variables` / `write_env_variable`, envyou pops a
   **physical approval dialog** — secrets are only released after you click
   **Yes**.

---

## Security notes (MVP scope)

- **Encryption key** is derived (SHA-256 over a fixed salt) from the machine id,
  binding the encrypted state to the local machine. A production build can swap
  in Argon2 + a user master password without changing the storage format.
- **License verification** is offline and hardware-bound (spec §6.3). The MVP
  proves possession of a well-formed key on the activating machine; a production
  build would additionally verify a Paddle-issued Ed25519 signature (the
  verification surface is isolated in `license.rs` for a drop-in upgrade).
- **Paddle checkout** itself (purchase flow) is out of MVP scope; activation of
  an issued key is implemented.

---

## License

MIT
