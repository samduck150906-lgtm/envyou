# envyou

> **Give Claude your secrets — safely.** A lightweight, retro (80s Windows style),
> **local-only** environment-variable manager for developers, with native
> Claude Desktop / MCP integration and a human approval gate on every secret
> access.

envyou keeps your `.env` secrets and API keys in a single **AES-256-GCM
encrypted file on your machine** — never in the cloud — and exposes them to
Claude Desktop over MCP **only when you click "Allow"**. Built on **Tauri v2
(Rust + Vanilla JS)** with a GUI-free, fully unit-tested Rust core.

<!-- TODO: add screenshots — main window, approval dialog, Claude Desktop link. -->
<!-- Suggested assets: docs/screenshot-main.png, docs/screenshot-approval.png, docs/demo.gif -->

---

## Why envyou

- **Local-first, zero-cloud.** Everything lives in one encrypted file on your
  machine. envyou makes no network calls.
- **AI-native.** Claude Desktop can list your projects and read/write env
  variables through envyou's MCP server — but every read and write is gated by
  a physical OS approval dialog you have to click.
- **Human-in-the-loop by default.** A secret is only released after you say yes.
- **Retro, credible UI.** A floating, always-on-top 80s-style window — hard
  pixel bevels, navy title bar, classic gray.

---

## Highlights

| Area | What's implemented |
| --- | --- |
| **Zero-cloud storage** | All state lives in a single **AES-256-GCM** encrypted file (`enc_state.json`). Nothing is ever sent off-machine. |
| **Key derivation** | Device-bound key by default; **optional Argon2id master-password** vault for password-protected encryption. |
| **Data model** | Full `EnvYouLocalState` model (projects, variables, settings, license). |
| **MCP server** | `envyou --mcp` runs a JSON-RPC 2.0 MCP server over STDIO exposing `list_projects`, `read_env_variables`, `write_env_variable`. |
| **Human-in-the-loop** | `read`/`write` tool calls block on a native OS approval dialog before any secret is released. |
| **Claude Desktop linking** | One click merges an `envyou` entry into `claude_desktop_config.json` **non-destructively** (existing servers preserved). |
| **Retro UI** | Vanilla HTML/CSS/JS floating window with an always-on-top pin. |
| **Freemium** | Free tier caps (3 projects / 10 vars per project) enforced; Pro unlock via an offline, **Ed25519-signed** license. |

---

## Security model

envyou is a local secrets tool, so here is exactly what it does and does not
protect — stated plainly.

### Encryption at rest
- State is sealed with **AES-256-GCM**. The on-disk file is a small JSON
  envelope (`{ v, alg, nonce, ciphertext }`); the plaintext never touches disk.
- **Two key modes:**
  - **Device-bound (default).** The key is derived (SHA-256) from a stable
    machine identifier (`/etc/machine-id`, hostname, or a **randomly generated,
    persisted per-install device secret**). This protects the file against being
    copied to another machine, but it does **not** protect against an attacker
    with code-execution or read access on *this* machine — the key material is
    derivable locally.
  - **Master password (Argon2id, opt-in).** A password-protected vault derives
    the key with **Argon2id** (memory-hard) over a random per-vault salt stored
    in the file. A wrong password fails to decrypt. The password is held only in
    memory. Use this if you want protection beyond machine-binding.

### AI access gate
- The MCP `read_env_variables` and `write_env_variable` tools **block on a native
  OS confirmation dialog** and only proceed if you approve. `list_projects`
  returns names + counts only — never values.

### License verification
- Pro is unlocked by an **Ed25519-signed license token** verified fully offline
  against a public key embedded in the app. The app can *verify* a license but
  never *mint* one, so a valid Pro token cannot be forged on the client.

---

## Current MVP limitations (read before shipping)

These are deliberate, documented gaps — not hidden ones.

- **License signing key is unset.** `license::LICENSE_PUBLIC_KEY_B64` ships as a
  **placeholder**, so the build **fails closed**: every activation is rejected
  until the product owner generates a keypair and pastes their real public key.
  This is intentional (safer to ship with Pro un-activatable than forgeable).
  See *License model* below. **The private signing key must never live in this
  repo.**
- **Master password has no UI yet.** The Argon2id password vault is implemented
  and tested in the core (`Store::with_password`, `migrate_to_password`), but the
  desktop app currently opens the **device-bound** vault by default. A first-run
  "set password" / "unlock" screen is the next step to expose it.
- **Device-bound mode is not a defense against local attackers.** See above.
  It binds the file to the machine; it does not resist local code execution.
- **Paddle/Lemon Squeezy checkout** (the purchase flow) is out of scope; only
  *activation* of an issued signed license is implemented.

---

## Architecture

```
envyou/
├── Cargo.toml                  # Cargo workspace
├── crates/
│   └── envyou-core/            # ⭐ Pure Rust, NO GUI deps — fully unit-tested
│       └── src/
│           ├── core/
│           │   ├── model.rs        # EnvYouLocalState data model + tier policy
│           │   ├── crypto.rs       # AES-256-GCM + Argon2id password vaults
│           │   ├── storage.rs      # encrypted enc_state.json, device secret, migration
│           │   ├── license.rs      # Ed25519-signed offline license verification
│           │   └── claude_config.rs# Claude Desktop non-destructive config merge
│           └── mcp/server.rs       # JSON-RPC 2.0 MCP server
├── src-tauri/                  # Tauri desktop shell (GUI + --mcp runtime)
│   ├── src/
│   │   ├── main.rs             # CLI mode switch: GUI vs --mcp
│   │   ├── lib.rs              # Tauri builder, tray, commands
│   │   ├── commands.rs         # GUI ↔ store commands (shares tier policy with MCP)
│   │   └── mcp_runtime.rs      # wires core MCP server to store + native dialog
│   └── tauri.conf.json
├── src/                        # Vanilla JS retro frontend
│   └── {index.html, styles/retro.css, js/{api.js, app.js}}
└── landing/                    # Static marketing site (deployed to Vercel)
```

The **`envyou-core`** crate has zero Tauri/UI dependencies so the
security-critical logic (crypto, storage, MCP, licensing) is tested in isolation
and reused by both runtime modes.

---

## Install

Pre-built desktop bundles are not published yet — build from source (below).

## Develop & test

The core crate builds and tests anywhere with a Rust toolchain — no system GUI
libraries required:

```bash
cargo test -p envyou-core      # crypto, storage, MCP, license, model, config
```

### Building the desktop app

The Tauri shell needs the usual platform WebView/GTK build dependencies
(see <https://v2.tauri.app/start/prerequisites/>). On Linux you'll need
`webkit2gtk`, `libglib`, GTK, etc. Once installed:

```bash
cargo tauri dev      # run the GUI (requires `cargo install tauri-cli`)
cargo tauri build    # produce a release bundle
```

### UI preview without compiling

`src/index.html` runs in a plain browser too: `js/api.js` falls back to a
`localStorage`-backed mock that mirrors the backend, so you can click through the
retro UI without building the Rust shell. **Note:** the browser preview stores
data unencrypted in `localStorage` — it is a UI demo, not the real vault.

---

## Claude Desktop / MCP integration

1. In **Settings → Link with Claude Desktop**, envyou writes its server entry
   into `claude_desktop_config.json`, merging non-destructively:

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

Config paths are resolved per-OS (macOS: `~/Library/Application Support/Claude/…`,
Windows: `%APPDATA%\Claude\…`). Linux has no official Claude Desktop config
location, so linking is macOS/Windows only.

---

## Freemium / Pro

- **Free:** up to **3 projects**, **10 variables per project**. Existing data
  always stays fully readable; caps only limit *adding more*. The MCP integration
  and local encryption are free.
- **Pro:** removes the caps, unlocked by an offline Ed25519-signed license.
- The free-tier policy is enforced by a single shared predicate
  (`EnvYouLocalState::can_write_variable`) used by **both** the GUI and MCP write
  paths, so they can never diverge.

### License model

Licenses are compact **Ed25519-signed tokens**: `<payload>.<signature>`
(URL-safe base64). The payload is JSON with `product`, `plan`, optional
`hardwareId`, `issuedAt`, optional `expiresAt`, and `features`. Verification is
fully offline against the embedded public key.

**Setting up signing (product owner, one time, offline).** The repo ships an
offline `license_tool` (gated behind the `issuer` feature so the app itself can
never mint licenses):

```bash
# 1. Generate a keypair. Writes the PRIVATE key to a 0600 file (keep it secret)
#    and prints the PUBLIC key to paste into LICENSE_PUBLIC_KEY_B64.
cargo run -p envyou-core --features issuer --example license_tool -- \
    keygen envyou-signing.key

# 2. Paste the printed public key into
#    crates/envyou-core/src/core/license.rs -> LICENSE_PUBLIC_KEY_B64

# 3. Mint a license for a buyer (do this from your purchase webhook):
cargo run -p envyou-core --features issuer --example license_tool -- \
    issue envyou-signing.key --plan pro \
    --hardware-id <machine-id> --expires 2027-07-06T00:00:00Z \
    --features unlimited_projects
```

Keep the **private key** in your payment provider's secret store / a hardware
token. **Never commit it** — `*-signing.key` is gitignored. On each purchase a
webhook (Paddle / Lemon Squeezy) runs step 3 and emails the token to the buyer.
Until the public key is configured, the build rejects all activations by design.

---

## Roadmap

- [ ] First-run **set-password / unlock** UI for the Argon2id vault
- [ ] Configure production Ed25519 public key + issuance webhook
- [ ] Export/import (`.env`, JSON, shell), encrypted backups
- [ ] Signed, notarized macOS/Windows release bundles
- [ ] Per-project MCP allow-list

---

## License

MIT
