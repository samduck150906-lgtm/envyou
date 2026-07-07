# envyou Test Checklist

## Automated

| Suite | Command | Covers |
|---|---|---|
| Core (Rust) | `cargo test -p envyou-core` | model, crypto, license, free-tier caps |
| Core + issuer | `cargo test -p envyou-core --features issuer` | license issue↔verify round-trip |
| Dev tools (JS) | `node test/devtools.test.js` | .env/export/JSON parser, export formatters, secret generator, diff |
| App smoke (headless) | Playwright against `src/index.html` + the `api.js` mock | free-tier Pro locks, Smart Import preview/import, Export formats, Secret Generator insert, Command Palette, Diff |

`node test/devtools.test.js` is dependency-free (no framework) and exits
non-zero on the first failure — safe to add to CI.

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
