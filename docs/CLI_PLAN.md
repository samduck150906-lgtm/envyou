# envyou CLI Companion — Design (P1)

A terminal companion so developers can pull secrets into shells, scripts, and CI
without opening the app. **Design only** — phase 1 implementation is pending.

## Principles (same as the app)

- Local-first, no network, no cloud.
- Reuses `envyou-core` (model, crypto, storage) — one source of truth.
- **Raw secrets are never printed by default.** Masked output unless the user
  explicitly opts in *and* proves they may.

## Distribution

Ship a separate `envyou` binary from a new `crates/envyou-cli` workspace member
(clap-based), reusing `envyou-core`. Installed alongside the desktop app, or via
`cargo install` / a downloadable binary. The CLI reads the **same** local
encrypted vault the app writes.

## Phase 1 commands

```bash
envyou projects                              # list projects (names + counts)
envyou list PROJECT                          # list keys in a project (masked)
envyou get PROJECT KEY                        # print one value (masked by default)
envyou export PROJECT --format dotenv         # export (dotenv|shell|json|docker|gha|example)
envyou import PROJECT ./.env                  # bulk import (same parser as the app)
envyou diff PROJECT_A PROJECT_B               # key/value diff (masked)
```

Read-only commands to implement first: `projects`, `list`, `diff`, `export`
(masked). These never expose raw secrets.

## Reveal policy (raw secrets)

`get`/`export` print masked values unless `--reveal` is passed. `--reveal`
requires **one** of:

1. the vault master password entered at the prompt (verified via `envyou-core`
   `unlock`), or
2. an approval handshake with the running app over local IPC (the app shows a
   confirm, mirroring the MCP approval gate).

If the vault is password-protected and no password/approval is supplied,
`--reveal` fails closed.

## Shared logic

The parser/formatter/diff already exist as pure functions in
`src/js/devtools.js`. Phase 1 should **port these to `envyou-core`** (Rust) so
the app (via a thin command), the CLI, and the webhook all share one tested
implementation. Until then the CLI can carry an equivalent Rust module with its
own unit tests mirroring `test/devtools.test.js`.

## Out of scope for phase 1

Writes beyond `import` (e.g. `set`, `rm`), watch mode, shell completions,
and `envyou run -- <cmd>` (inject env into a subprocess) — all phase 2.
