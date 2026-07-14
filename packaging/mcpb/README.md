# envyou — Claude Desktop extension (`.mcpb`)

This directory packages the envyou MCP server as a **Claude Desktop extension**
so users can install it with a double-click instead of hand-editing
`claude_desktop_config.json`.

- [`manifest.json`](manifest.json) — the MCP-bundle manifest (`manifest_version`
  `0.3`, `server.type: "binary"`). It points at the bundled `envyou` binary and
  runs it with `--mcp`. `platform_overrides.win32` selects `envyou.exe`.
- [`../../scripts/build-mcpb.sh`](../../scripts/build-mcpb.sh) — assembles the
  bundle from the manifest + a pre-built binary, syncs the version to the
  workspace `Cargo.toml`, and packs a `.mcpb` (with a SHA-256 checksum).
- [`../../.github/workflows/mcpb.yml`](../../.github/workflows/mcpb.yml) — builds,
  (optionally) signs, packs, and attaches the `.mcpb` to a tagged release.

## Build locally

```bash
# 1. Build the release binary (the same binary the GUI uses; --mcp is a flag).
cargo build --release --bin envyou --manifest-path src-tauri/Cargo.toml

# 2. Pack a per-platform .mcpb (install the darwin build on macOS, win32 on Windows).
scripts/build-mcpb.sh --binary target/release/envyou --os darwin       # macOS
scripts/build-mcpb.sh --binary target/release/envyou.exe --os win32    # Windows
# → dist/envyou-<os>.mcpb  (+ .sha256)
```

The script prefers the official `mcpb` CLI (`npm i -g @anthropic-ai/mcpb`),
which **validates** the manifest before packing; if it isn't installed it falls
back to a plain `zip` (a `.mcpb` is a zip archive). To validate by hand:

```bash
npx @anthropic-ai/mcpb validate packaging/mcpb/manifest.json
```

## Install

Open the `.mcpb` with Claude Desktop (or drag it into **Settings → Extensions**).
Claude then launches the bundled `envyou --mcp`. Reading a value or writing one
still pops envyou's native approval dialog — the bundle changes how the server is
*installed*, not how approval works.

## Code signing & notarization

> **Status:** the pipeline is wired but currently produces **UNSIGNED** bundles —
> no signing certificates are configured in this repo. Add the secrets below to
> turn signing on; each signing step is gated on its secret and skips when unset.

Sign the **binary before packing** (the `.mcpb` wraps the already-signed binary).

### macOS (Developer ID + notarization)
Set these repository secrets; the `mcpb.yml` "macOS — …" steps use them:

| Secret | Purpose |
| --- | --- |
| `APPLE_CERTIFICATE_P12_BASE64` | base64 of your Developer ID Application `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | password for that `.p12` |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Your Name (TEAMID)` |
| `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD` | notarization (`notarytool`) |

Commands (already in the workflow): `codesign --options runtime --timestamp`
then `xcrun notarytool submit … --wait`.

### Windows (Authenticode)
| Secret | Purpose |
| --- | --- |
| `WINDOWS_CERT_PFX_BASE64` | base64 of your code-signing `.pfx` |
| `WINDOWS_CERT_PASSWORD` | password for that `.pfx` |

Command (already in the workflow): `signtool sign /fd SHA256 /tr <timestamp> /td SHA256`.
For an EV/organization certificate, swap in your provider's signer (e.g.
SignPath / SSL.com) — the rest of the pipeline is unchanged.

## Scope note
Linux is intentionally excluded (`compatibility.platforms: ["darwin", "win32"]`)
because Claude Desktop has no official config location there. `envyou --mcp`
still runs on Linux for other MCP clients (e.g. Claude Code via `claude mcp add`).
