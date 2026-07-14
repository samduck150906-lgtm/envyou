# envyou Test Checklist

## Automated

| Suite | Command | Covers |
|---|---|---|
| Core (Rust) | `cargo test -p envyou-core` | model, crypto, license verify, free-tier caps, code normalize |
| MCP server (Rust) | `cargo test -p envyou-core mcp::` | protocol (initialize/ping/tools/errors, id echo), scoped named reads, wildcard/empty rejection, capability policy, fail-closed approval (deny/timeout/error), write/delete opt-in, never-share enforcement, sensitivity heuristic, value-free audit log + JSONL sink |
| MCP config (Rust) | `cargo test -p envyou-core claude_config` | non-destructive merge + unlink, Claude Code argv (injection-safe), shell-quoting |
| Core + issuer | `cargo test -p envyou-core --features issuer` | license issue↔verify round-trip, code generation, v2 cert fields |
| Webhook (Rust) | `cargo test -p paddle-webhook` | Paddle signature verify (valid/tampered/stale/malformed), percent-encoding |
| Dev tools (JS) | `node test/devtools.test.js` | .env/export/JSON parser, export formatters, secret generator, diff |
| App smoke (headless) | Playwright against `src/index.html` + the `api.js` mock | free-tier Pro locks, Smart Import, Export, Secret Generator, Command Palette, Diff, **AI Integrations settings persistence (mcp toggles/timeout/never-share) + audit-log viewer + scrollable modal** |

`node test/devtools.test.js` is dependency-free (no framework) and exits
non-zero on the first failure — safe to add to CI.

## License / activation (see docs/LICENSE_SYSTEM.md, docs/ACTIVATION_FLOW.md)

### Pre-release gate (must pass before every release)
- [ ] `license_tool checkkey` prints **MATCH ✓** — the shipped
  `LICENSE_PUBLIC_KEY_B64` corresponds to the webhook's `ENVYOU_SIGNING_KEY_B64`
  (`ENVYOU_SIGNING_KEY_B64=… cargo run -p envyou-core --features issuer --example license_tool -- checkkey`)
- [ ] `cargo test -p envyou-core` includes `shipped_build_has_a_real_license_key`
  (guards against ever shipping the placeholder or an empty key)

### Activation RPC (verify in Supabase SQL editor / MCP)
- [ ] Unknown code → `{ok:false, code:"LICENSE_NOT_FOUND"}`
- [ ] Right code, wrong email → `{ok:false, code:"EMAIL_MISMATCH"}`
- [ ] Valid code + email → `{ok:true, signed_certificate:"…"}` and
  `activation_count` increments by 1, `last_activated_at` set
- [ ] `activation_count == max_activations` → next call `TOO_MANY_ACTIVATIONS`
- [ ] `status != 'active'` → `LICENSE_INACTIVE`
- [ ] `license_status_by_txn(<txn>)` returns `{found:true, license_code, email, tier}`

### App (run `cargo tauri dev`)
- [ ] **Activate Pro** with a real email + code → Pro unlocks; caps lift
- [ ] Code field auto-formats to `ENVY-XXXX-XXXX-…` while typing; lowercase /
  no-hyphen paste still activates
- [ ] Wrong email shows "registered to a different email address" (not a raw code)
- [ ] Offline after activation: relaunch with no network → still Pro (stored
  certificate re-verifies)
- [ ] Hand-edit local state to `isPro:true` with no/invalid certificate →
  relaunch drops back to Free (re-verification wins)
- [ ] **Advanced → paste certificate** with a valid cert → Pro unlocks offline;
  garbage cert is rejected
- [ ] Deep link `envyou://activate?email=…&code=…` pre-fills and activates

### Webhook (Sandbox — see docs/PADDLE_WEBHOOK.md)
- [ ] Test purchase → email arrives with a short `ENVY-…` code + deep link
- [ ] Re-deliver the same `transaction.completed` → no second license; code re-sent
- [ ] Transaction with no Pro price → 200 + `ALERT` log, no license
- [ ] `lookup-license` / `resend-license` / `reset-activations` behave

### Landing (browser)
- [ ] A Pro **Buy** button opens the email modal; email is required + confirmed
- [ ] Paddle opens with `customer.email`, `customData.license_email`, and
  `successUrl` = `https://envyou.dev/success`
- [ ] Success page shows the code (Copy + Activate deep link) within ~a minute;
  falls back to the "check your email" message on timeout / missing `_ptxn`

## Manual verification (run `cargo tauri dev`)

### Smart Import
- [ ] Paste a `.env` blob → Preview shows NEW/UPDATE rows with public/secret/empty/dup tags
- [ ] Paste `export KEY="v"` lines and a JSON object → both parse
- [ ] Paste code with `process.env.FOO` → key `FOO` appears with empty value
- [ ] Conflict row: choose overwrite / keep / save-as (rename) / skip → applied correctly
- [ ] On Free at 10 vars, overflow is skipped and the Pro upsell appears

### Smart Export
- [ ] Switch formats (.env, .env.local, shell, JSON, Docker, GitHub Actions, .env.example) → output updates live
- [ ] Keys-only and Mask toggles hide values
- [ ] Select a subset of variables → only those export
- [ ] Raw-secret warning shows; Copy/Download of raw secrets asks to confirm
- [ ] GitHub Actions output shows `${{ secrets.KEY }}` (no raw values)

### Diff
- [ ] Compare two projects → Only-in-A / Only-in-B / Different / Identical sections
- [ ] Values masked by default; "Show values" reveals (prompts confirm if a project looks like prod)
- [ ] Copy a missing key A→B / B→A (blocked with upsell if B is at the Free cap)

### Command Palette
- [ ] `Ctrl/Cmd+K` and `/` (when not typing) open it; `⌕` button opens it
- [ ] Type to filter projects / variables / actions; `↑`/`↓`/`Enter`/`Esc` work
- [ ] Enter on a variable copies its value; sensitive values are not shown in the list

### Secret Generator
- [ ] From the variable editor, `🔑 Generate` opens it; type + length + symbols work
- [ ] Regenerate produces a new value; Copy works
- [ ] "Use value" returns to the variable editor with the value filled and the key preserved

### Free/Pro (regression)
- [ ] 3-project / 10-var caps still enforced; counters show `🔒` at the cap
- [ ] Custom color picker and Claude MCP link remain Pro-locked with the upsell

## MCP / AI Integrations (real device — macOS & Windows)

> This is the flow the Linux CI **cannot** exercise (no webkit2gtk to build the
> Tauri shell, no Claude clients). Run it on a real macOS and a real Windows box
> before shipping the MCP feature. Never type or screenshot a real secret value.

### Settings → AI Integrations
- [ ] New install: MCP master switch is **OFF**; reads/lists on, **writes & deletes OFF**
- [ ] Toggle enable + reads; set an approval timeout; **Save** → reopen Settings and
  confirm the values persisted (not reset to defaults)
- [ ] The long Settings dialog **scrolls** inside the small window — Save/Cancel are
  always reachable
- [ ] The data-flow note states approved values go to the AI provider (no "never leaves")

### Claude Desktop (macOS / Windows only)
- [ ] **Link** writes an `envyou` entry into `claude_desktop_config.json`; a
  pre-existing unrelated server in that file is preserved, and a timestamped
  `.bak` is created
- [ ] Restart Claude Desktop → envyou tools appear
- [ ] **Unlink** removes only the `envyou` entry (others untouched); a second Unlink
  reports "nothing to remove"
- [ ] Prefer the `.mcpb`? Installing `dist/envyou-<os>.mcpb` registers the server
  without editing any config (see Packaging below)

### Claude Code
- [ ] **Copy command** yields `claude mcp add --transport stdio envyou -- "<abs path>" --mcp`
  with the path quoted if it has spaces; pasting it registers envyou
- [ ] **Link automatically** runs it in-app; if `claude` isn't on PATH the error says so
- [ ] `claude mcp list` shows envyou; inside Claude Code `/mcp` lists its tools
- [ ] Path with a space and a non-ASCII (e.g. Korean) username both work

### Read approval (scoped)
- [ ] Ask Claude: "read my-app's `API_URL`" → dialog names the client, project, and the
  **exact** variable; **Allow** returns only that value
- [ ] Ask for several vars → dialog lists all names + count; names that look like
  credentials show a `⚠ looks sensitive` marker
- [ ] Ask Claude to "read everything" → the tool refuses (must name variables; no
  wildcard); no dialog for the refused shape
- [ ] **Deny** → no value returned; nothing appears in the transcript

### Write / delete (opt-in)
- [ ] With writes **off**, Claude's write attempt is refused with no dialog and the
  value is never echoed
- [ ] Enable writes → a write pops a stronger dialog naming create-vs-update; the
  value is not shown in the dialog; on Allow it saves
- [ ] Enable deletes → a delete pops the strongest dialog; on Allow the variable is
  removed **and** an encrypted `enc_state.json.bak.*` backup exists

### Never-share
- [ ] Add a variable name to **Never share**; ask Claude to read it → it is reported as
  blocked and **no approval dialog appears** (can't be approved)
- [ ] Requesting a never-share name alongside a normal one returns only the normal one

### Audit log
- [ ] After a few requests, **View audit log** shows client · tool · outcome · names
  with timestamps and **no values**; "Clear" empties it
- [ ] Inspect `<data dir>/mcp_audit.jsonl` — confirm it contains no variable values

### Fail-closed / robustness
- [ ] Approval that isn't answered within the timeout auto-denies (no hang, no leak)
- [ ] Kill the approval dialog / envyou mid-request → the request is denied, not granted
- [ ] `envyou --mcp` writes **only** JSON-RPC to stdout (logs go to stderr):
  `printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"initialize"}' '{"jsonrpc":"2.0","id":2,"method":"tools/list"}' | envyou --mcp`
  → every stdout line parses as JSON; response ids echo the request ids

## Packaging (.mcpb) — see packaging/mcpb/README.md
- [ ] `cargo build --release --bin envyou --manifest-path src-tauri/Cargo.toml` then
  `scripts/build-mcpb.sh --binary <bin> --os <darwin|win32>` produces
  `dist/envyou-<os>.mcpb` + `.sha256`
- [ ] `npx @anthropic-ai/mcpb validate packaging/mcpb/manifest.json` passes
- [ ] Double-clicking the `.mcpb` installs it into Claude Desktop and the tools work
- [ ] With signing secrets set, the `mcpb.yml` workflow signs/notarizes; without them
  it still produces an (unsigned) bundle and the job stays green
