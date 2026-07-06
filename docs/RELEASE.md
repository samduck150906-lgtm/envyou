# Release playbook

Drafts and a pre-launch checklist for shipping envyou. Copy/adapt as needed.

> **Version note:** the workspace is currently versioned `1.0.0`, but this is the
> first public MVP. Consider tagging the first public release **`v0.1.0`** to set
> honest expectations, and reserve `1.0.0` for after the license signing key is
> configured and the desktop bundles are signed/notarized.

---

## Pre-launch checklist

Blockers before **selling Pro** (not before open-sourcing):

- [ ] Generate an Ed25519 signing keypair offline; paste the **public** key into
      `crates/envyou-core/src/core/license.rs` → `LICENSE_PUBLIC_KEY_B64`.
      Confirm the private key is **not** anywhere in the repo.
- [ ] Wire the purchase → license-issuance webhook (Paddle / Lemon Squeezy) that
      signs a payload and emails the token.
- [ ] `cargo tauri build` produces a working bundle on **macOS, Windows, Linux**.
- [ ] Sign + notarize the macOS build; sign the Windows build.

Should-fix before a public launch:

- [ ] Verify the master-password unlock/setup flow end-to-end in the built app.
- [ ] Add a visible "browser preview is unencrypted" note if the web preview is
      published anywhere.
- [ ] Confirm no secret values appear in logs or error dialogs (covered by tests
      at the core/MCP layer).

Nice to have:

- [ ] Export/import (`.env`, JSON), encrypted backups.
- [ ] Per-project MCP allow-list.
- [ ] Screenshots / demo GIF in the README.

---

## GitHub Release — `v0.1.0`

```markdown
## envyou v0.1.0 — Public MVP

Give Claude your secrets, safely. A local-only, retro environment-variable
manager with a human approval gate on every AI access.

### Highlights
- 🔐 Local AES-256-GCM encrypted vault — nothing leaves your machine
- 🤝 Claude Desktop MCP integration (list / read / write env vars)
- ✋ Native OS approval dialog before any secret is released
- 🔑 Optional Argon2id master-password vault
- 🖥️ Retro 80s floating desktop UI (Tauri v2, Rust core)
- 🆓 Free: 3 projects / 10 vars — Pro removes the caps

### Security model (honest MVP notes)
- The default key is device-bound; an optional Argon2id master-password vault is
  available. See the README security section for what each mode does and does
  not protect against.
- Pro is unlocked by an Ed25519-signed, offline-verified license. This build
  ships without a configured signing key, so paid activation is disabled until
  the maintainer sets one.

### Build from source
`cargo test -p envyou-core` (core) · `cargo tauri build` (desktop; needs
GTK/WebKit on Linux — see v2.tauri.app/start/prerequisites).
```

---

## Product Hunt intro

```
envyou — Give Claude your .env secrets, safely 🔐

Local-only. No cloud. No account. Your API keys live in one AES-256-encrypted
file on your machine — and Claude Desktop can read or write them only when you
click "Allow" on a native approval dialog.

Built for AI-coding devs who love the Claude/Cursor workflow but don't want to
paste secrets into a chat or trust yet another cloud vault.

• Local-first, offline, zero-knowledge by design
• Claude Desktop MCP native (list / read / write, all gated by your approval)
• Optional master-password (Argon2id) encryption
• Retro 80s desktop UI that doesn't take itself too seriously
• Free forever tier; Pro unlocks unlimited projects

Tauri + Rust core, fully open source. No telemetry.

🚀 Launch week: Pro 40% off ($17 first year) + Founder's Lifetime $49 (first 250).
```

---

## Internal changelog (this release)

### Security
- License verification moved from a forgeable `SHA256(key + hardware_id)` scheme
  to **Ed25519 signed-token** verification (offline; fails closed until the
  public key is configured).
- Added **Argon2id master-password vaults** (v2 salted envelope) alongside the
  device-bound path; v1 files remain readable (backward compatible).
- Removed the predictable hard-coded machine-id fallback in favour of a
  **persisted random per-install device secret**.

### Fixes
- MCP: existing variables can be updated once a free-tier project hits the cap
  (previously blocked). The GUI and MCP write paths now share one policy
  predicate (`can_write_variable`).
- UI: unified the masking glyph; select a new project by id rather than array
  position.

### UX & accessibility
- Master-password unlock gate on launch + "Set master password" in Settings.
- Keyboard-operable project rows and copy targets, aria-labels, dialog focus
  trap + restore, aria-live status, `:focus-visible` outlines, free-tier
  usage counters.

### Docs, site, CI
- Honest README security/limitations section.
- Landing SEO/GEO/AEO pass (JSON-LD, FAQ, comparison table).
- CI: fmt + clippy + core tests, plus a 3-OS workspace build matrix.

### Tests
- Core suite 33 → 53; workspace 58 total, all passing.
