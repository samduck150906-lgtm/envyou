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

- **Local-first storage.** Your projects and secrets live in one encrypted file
  on your machine. envyou never syncs them to a cloud — there is no account and
  no envyou server that receives your secrets. (The app does make exactly one
  kind of outbound call: contacting the license server to activate Pro. That
  call carries your license code and email, never your stored secrets — see
  [Where your data goes](#where-your-data-goes).)
- **AI-native, with your approval.** Claude Desktop and Claude Code can list your
  projects, list variable names, and read/write the variables you name through
  envyou's MCP server — but reading a value or writing one is gated by a native
  approval dialog you have to click, and writing is off until you opt in.
- **Human-in-the-loop by default.** A value is only released after you say yes,
  and a value you approve is then sent to your AI provider (e.g. Anthropic) to
  answer your request. Approving is what sends it; nothing goes before that.
- **Retro, credible UI.** A floating, always-on-top 80s-style window — hard
  pixel bevels, navy title bar, classic gray.

---

## Highlights

| Area | What's implemented |
| --- | --- |
| **Zero-cloud storage** | All state lives in a single **AES-256-GCM** encrypted file (`enc_state.json`). envyou never syncs it to a cloud; the only things that leave your machine are a value you approve for an AI and your license code/email on Pro activation (see [Where your data goes](#where-your-data-goes)). |
| **Key derivation** | Device-bound key by default; **optional Argon2id master-password** vault for password-protected encryption. |
| **Data model** | Full `EnvYouLocalState` model (projects, variables, settings, license). |
| **MCP server** | `envyou --mcp` runs a JSON-RPC 2.0 MCP server over STDIO exposing `list_projects`, `list_variable_names`, `read_env_variables` (you name the exact variables), and `write_env_variable`. Works with **Claude Desktop** and **Claude Code**. |
| **Policy gating** | A user-controlled [`McpAccess`](crates/envyou-core/src/core/model.rs) policy decides which tools an AI may attempt. Reads/lists default on; **writes and deletes are opt-in**. |
| **Human-in-the-loop, fail-closed** | Reading a value or writing one blocks on a native approval dialog. Denial, a timeout, or an approval UI that can't be shown all mean *no data released* — there is no fail-open path. |
| **Claude Desktop linking** | One click merges an `envyou` entry into `claude_desktop_config.json` **non-destructively** (existing servers preserved). |
| **Retro UI** | Vanilla HTML/CSS/JS floating window with an always-on-top pin. |
| **Developer tools** | Smart Import (paste `.env`/`export`/JSON/`process.env`), multi-format Export, project Diff, Command Palette, and a Secret Generator — see below. |
| **Freemium** | Free tier caps (3 projects / 10 vars per project) enforced; Pro unlock via an offline, **Ed25519-signed** license. |

---

## Developer tools

Everyday `.env` workflow helpers, all local and all free. Pure logic lives in
`src/js/devtools.js` (unit-tested via `node test/devtools.test.js`); nothing
touches the vault encryption or the network.

- **Smart Import (`⇪`)** — paste a `.env` file, `export KEY=…` lines, a JSON
  object, or code using `process.env.KEY`. A preview flags new vs existing,
  empty/public/secret/duplicate keys, and lets you resolve conflicts
  (overwrite / keep / rename / skip) before saving.
- **Smart Export (`⧉`)** — `.env`, `.env.local`, shell `export`, JSON, Docker
  Compose, GitHub Actions (`${{ secrets.KEY }}`), or `.env.example`. Values /
  keys-only / masked, per-variable selection, copy or download. Raw-secret
  exports warn and confirm first.
- **Diff (`⇄`)** — compare two projects (only-in-A / only-in-B / changed /
  same); values masked by default; copy a missing key across.
- **Command Palette (`⌕`, `Ctrl/Cmd+K`, or `/`)** — keyboard-first search over
  projects, variables, and actions.
- **Secret Generator (`🔑`)** — hex / base64 / URL-safe / UUID v4 / strong
  password / JWT secret from the OS CSPRNG, inserted straight into a variable.

See [`docs/DEVELOPER_UX_ROADMAP.md`](docs/DEVELOPER_UX_ROADMAP.md) for what's
next (in-project environments, templates, Git-safety, CLI, MCP policies).

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
- **Policy first.** The [`McpAccess`](crates/envyou-core/src/core/model.rs)
  settings decide which tools an AI may even attempt. Listing projects/variable
  names and reading values default on; **`write_env_variable` is off until you
  opt in**, and a disabled tool is refused before any dialog appears.
- **Scoped reads.** `read_env_variables` requires you to name the exact
  variables — there is no "read everything" default and wildcards are rejected.
  The approval dialog shows every requested name and the count; you can approve a
  subset, and only approved-and-named values are returned.
- **Fail-closed approval.** `read_env_variables` and `write_env_variable` block
  on a native confirmation dialog and only proceed on an explicit *yes*. A
  denial, a timeout (default 60s, configurable), or an approval UI that can't be
  shown are all treated identically: nothing is released.
- **Names only where possible.** `list_projects` returns names + counts, and
  `list_variable_names` returns variable names + whether each has a value —
  never the values themselves. Both can be disabled in settings.
- **What the AI never sees in errors.** Secret values are never placed in a tool
  result's error text or in logs, and MCP diagnostics go to stderr so the STDIO
  JSON-RPC stream stays clean.

### Where your data goes
envyou is local-first, but "local-first" is not the same as "nothing ever
leaves your machine." Precisely:

| Data | Leaves your machine? |
| --- | --- |
| Your projects, variable names, and **values at rest** | **No** — stored only in the encrypted `enc_state.json`; no cloud sync, no account. |
| A variable **value you approve for Claude** | **Yes, when you approve it** — sent to your AI client (Claude Desktop / Claude Code) and processed by its provider (e.g. Anthropic). envyou has no control over that data once approved. |
| **License activation** (Pro) | **Yes** — activating Pro sends your license *code* and *email* to the activation server. Your stored secrets are never part of this. |

So the honest one-liner is: **stored locally and encrypted; the only secret
values that leave are the ones you explicitly approve for an AI, and those go to
that AI's provider.** Share API keys, passwords, and tokens with an AI only when
you actually need to.

### License verification
- Pro is unlocked by an **Ed25519-signed license token** verified fully offline
  against a public key embedded in the app. The app can *verify* a license but
  never *mint* one, so a valid Pro token cannot be forged on the client.

---

## Current MVP limitations (read before shipping)

These are deliberate, documented gaps — not hidden ones.

- **Master password has no UI yet.** The Argon2id password vault is implemented
  and tested in the core (`Store::with_password`, `migrate_to_password`), but the
  desktop app currently opens the **device-bound** vault by default. A first-run
  "set password" / "unlock" screen is the next step to expose it.
- **Device-bound mode is not a defense against local attackers.** See above.
  It binds the file to the machine; it does not resist local code execution.

The purchase → license → activation flow **is** implemented end-to-end (Paddle
checkout, signed-certificate issuance, offline verification) — see *License
model* below. The signing **private key must never live in this repo**; the app
ships only the public verification key, and the build fails closed if that key
is ever reset to the placeholder.

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

## Claude Desktop / Claude Code / MCP integration

`envyou --mcp` is a standard JSON-RPC 2.0 MCP server over STDIO, so it works with
any MCP client. The two first-class ones:

### Claude Desktop

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

Config paths are resolved per-OS (macOS: `~/Library/Application Support/Claude/…`,
Windows: `%APPDATA%\Claude\…`). Linux has no official Claude Desktop config
location, so Desktop linking is macOS/Windows only.

### Claude Code

Claude Code adds any stdio MCP server from the CLI — point it at the same binary
with the `--mcp` flag (use the absolute path; quote it if it contains spaces):

```bash
claude mcp add --transport stdio envyou -- "/absolute/path/to/envyou" --mcp
claude mcp list          # verify it registered
# then inside Claude Code:  /mcp   → confirm the envyou tools are listed
```

### The tools, and what happens on a call

| Tool | Approval | Returns |
| --- | --- | --- |
| `list_projects` | none | project ids, names, variable counts — **no values** |
| `list_variable_names` | none | variable names + whether each has a value — **no values** |
| `read_env_variables` | **yes, per call** | only the **named** variables you approve, with values |
| `write_env_variable` | **yes, per call** (and off until you opt in) | confirmation of the changed key — **never the value** |

When Claude calls `read_env_variables` or `write_env_variable`, envyou pops a
**native approval dialog** naming the client, the project, and the exact
variables. A value is released only after you approve; a denial, a timeout, or a
dialog that can't be shown all deny the request. See
[Where your data goes](#where-your-data-goes) for what a value you approve does
next.

> **Note (this build):** the MCP *master on/off* switch and the write/delete
> opt-in toggles live in the [`McpAccess`](crates/envyou-core/src/core/model.rs)
> settings and are enforced by the server, but the Settings **UI** to flip them
> is still being built. Until it ships, reads/lists are enabled and AI writes are
> off by default.

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

Two layers, so the buyer-facing key is short and pretty while verification
stays cryptographic:

- **License code** — what the buyer sees: `ENVY-K7M4-9Q2P-D8X6-R3TA`. A
  short, unambiguous lookup key into the license DB. Not verifiable by itself.
- **Signed certificate** — what the app verifies: a compact
  **Ed25519-signed token** `<payload>.<signature>` (URL-safe base64). The
  payload is JSON with `product`, `plan`, optional `hardwareId`, `issuedAt`,
  optional `expiresAt`, `features`, and (v2) `licenseId`/`emailHash`/`codeHash`.
  Verification is fully **offline** against the public key embedded in the app.

On purchase, the Paddle webhook mints a certificate, stores it in Supabase keyed
by a freshly generated code, and emails the buyer the **code**. To activate, the
app trades the code + email at the Supabase `activate_license` RPC for the
certificate, verifies it offline, and stores it — after which Pro works
air-gapped and is re-verified on every load. The app holds only the public key,
so it can *verify* but never *mint* a license.

> **Full docs:** [`docs/LICENSE_SYSTEM.md`](docs/LICENSE_SYSTEM.md) (design +
> key management), [`docs/ACTIVATION_FLOW.md`](docs/ACTIVATION_FLOW.md) (buyer
> journey), [`docs/PADDLE_WEBHOOK.md`](docs/PADDLE_WEBHOOK.md) (issuance & ops).

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

# 3. Prove the app will accept the webhook's licenses (run before every release;
#    exits non-zero on mismatch or if the app still ships the placeholder):
ENVYOU_SIGNING_KEY_B64=<your private key b64> \
cargo run -p envyou-core --features issuer --example license_tool -- checkkey
```

Keep the **private key** only in the webhook's secret store (Railway
`ENVYOU_SIGNING_KEY_B64`). **Never commit it** — `*-signing.key` is gitignored.
Reset `LICENSE_PUBLIC_KEY_B64` to the placeholder (or empty) to force the build
closed and reject all activations by design.

---

## Roadmap

- [x] Production Ed25519 public key + Paddle issuance webhook + short-code
  activation (see `docs/LICENSE_SYSTEM.md`)
- [x] MCP hardening: scoped named reads, `list_variable_names`, capability policy
  (writes/deletes opt-in), fail-closed approval with timeout — all in
  `envyou-core` and unit-tested
- [ ] **Settings → AI Integrations** UI: MCP master switch, write/delete toggles,
  approval timeout, and one-click Claude Code (`claude mcp add`) setup
- [ ] Approval **broker**: hand MCP approval requests to the running GUI over
  local IPC (robust cross-platform prompt when launched headless)
- [ ] First-run **set-password / unlock** UI for the Argon2id vault
- [ ] Export/import (`.env`, JSON, shell), encrypted backups
- [ ] Signed, notarized macOS/Windows release bundles + `.mcpb` Desktop Extension
- [ ] Per-project MCP allow-list & "never share" variables; local audit log

---

## License

MIT
