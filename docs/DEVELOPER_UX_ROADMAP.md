# envyou — Developer Convenience Roadmap

Turning envyou from a "safe secret box" into a daily-driver developer tool,
while keeping every core principle intact: **local-first, no cloud sync, no
server storage, no raw-secret exfiltration, AES-256-GCM + Argon2id, Tauri (not
Electron), $59 lifetime Pro.**

## Architecture note — where the new logic lives

The P0 features are pure, side-effect-free transforms over key/value pairs the
frontend already holds (values are decrypted only for display/copy, exactly as
before). That logic lives in **`src/js/devtools.js`** — a dependency-free module
that:

- runs identically in the Tauri webview and the browser preview,
- **never** touches the encrypted vault or the network,
- is dual-exported (`window.EnvyouDev` + CommonJS) so it is unit-tested under
  Node (`test/devtools.test.js`) and can be reused by the planned CLI.

No new Tauri commands and no storage migration were needed, so the encryption
boundary is unchanged.

---

## P0 — shipped

### 1. Smart Import (`⇪`)
Paste **any** of these and envyou parses it to a key/value list:

- `.env` (`KEY=value`, `# comments`, quoted values)
- shell `export KEY="value"` / `export KEY='value'`
- JSON `{ "KEY": "value" }`
- code references: `process.env.KEY`, `import.meta.env.KEY` → key with empty value

Then a **preview before saving** shows, per entry: NEW vs UPDATE, and tags for
`empty`, `public` (`NEXT_PUBLIC_`/`PUBLIC_`/`VITE_`/…), `secret`
(`SECRET`/`TOKEN`/`KEY`/`PASSWORD`/`PRIVATE`/`DATABASE_URL`), and `dup`.

**Conflict resolution** per row: overwrite · keep existing · save-as (rename) ·
skip. New keys can be added/skipped/renamed. Sensitive keys are stored masked by
default. The Free 10-vars/project cap is respected — overflow is skipped and the
Pro upsell is shown.

### 2. Smart Export (`⧉`)
Export a project in six formats: `.env`, `.env.local`, `export KEY=…`, JSON,
Docker Compose `environment:`, GitHub Actions `env:` (references
`${{ secrets.KEY }}` — never raw), and `.env.example` (blanks secrets, keeps
public values, annotates required/public). Options: values / keys-only /
masked, and per-variable selection. Copy to clipboard or download a file. A
**raw-secret warning** appears when the output would contain real secrets, and a
confirm gate protects clipboard/file writes of raw secrets.

### 3. Environment Diff (`⇄`)
Compare two projects: keys only in A, only in B, values that differ, and
identical keys. Values are **masked by default**; "Show values" reveals them and
prompts an extra confirm when either project name looks like production.
One-click copy of a missing key into the other project (cap-aware).

> Note: the spec's *in-project* environments (local/staging/prod inside one
> project) require a data-model change and migration — see P1 below. The shipped
> Diff compares **projects**, which covers the same workflow today
> (`myapp-staging` vs `myapp-prod`).

### 4. Command Palette (`⌕`, `Ctrl/Cmd+K`, or `/`)
Keyboard-first launcher: search projects, search variables (copy on Enter),
and run actions (new var/project, Smart Import, Export, Diff, Secret Generator,
Settings, Upgrade). `↑`/`↓` to move, `Enter` to run, `Esc` to close. Sensitive
values are never shown in the list (keys only).

### 5. Secret Generator (`🔑 Generate` in the variable editor, or the palette)
Generates `hex`, `base64`, `URL-safe token`, `UUID v4`, `strong password`
(optional symbols), and `JWT secret` using the platform CSPRNG
(`crypto.getRandomValues`). Copy, or "Use value" to drop it straight into the
variable being edited. Nothing leaves the machine.

---

## P1 — designed, not yet built

- **In-project Environments (#4 full):** add `environments: [{name, variables}]`
  to `ProjectItem` with a safe migration (existing `variables` → a `default`
  environment). Touches `envyou-core` model/storage, Tauri commands, MCP, and
  the sidebar. Diff already accepts any two var lists, so it will extend cleanly.
- **Project Templates (#7):** ship a static template table (Next.js, Supabase,
  Prisma, Stripe/Paddle, OpenAI/Anthropic, Tauri, Node API, Docker) that seeds
  empty keys; preview before apply; Free cap-aware. Pure data + a picker in the
  New Project modal.
- **`.env.example` generator (#8):** already available as the `example` export
  format; a dedicated one-click action + required/optional annotations is a thin
  wrapper.
- **Git Safety Helper (#9):** read-only checks on a chosen folder (`.gitignore`
  covers `.env*`, tracked secret files, `.env.example` presence). Needs a Tauri
  command with **read-only** fs access — never deletes or writes.
- **CLI Companion (#10):** see `docs/CLI_PLAN.md`. Reuses `envyou-core`; raw
  output gated behind `--reveal` + master-password/app approval.
- **MCP Developer UX (#11):** approval policies (ask / session / 10-min /
  read-only) with stronger gating for write/delete, plus an MCP Activity Log
  (no raw secret values stored). Lives in `envyou-core/src/mcp` +
  `src-tauri/src/mcp_runtime.rs`.

## P2 — later
Variable metadata (description/required/tags/source), variable history &
restore (encrypted), first-run onboarding, ongoing UI/UX polish.

---

## Security notes

- Import/export/generate/diff are pure client transforms; the vault stays
  AES-256-GCM + Argon2id and nothing is sent anywhere.
- Secrets are **masked by default** everywhere new: import preview, diff,
  palette, export.
- Raw-secret export (clipboard or file) is gated by an explicit warning +
  confirm. GitHub Actions and `.env.example` formats never emit raw secrets.
- Secret generation uses the OS CSPRNG; generated values are never transmitted.

## Free / Pro impact

Free keeps 3 projects / 10 vars per project. All new convenience features
(Smart Import/Export, Diff, Palette, Secret Generator) are **free** — they are
the "try it and love it" surface. Pro ($59 lifetime) still unlocks unlimited
projects/vars, custom env colors, and Claude MCP linking; hitting any limit
surfaces the soft upsell: *"envyou Pro unlocks unlimited projects, unlimited
secrets, MCP integration, and lifetime updates. One payment. No subscription."*

## TODO (next up)

1. In-project Environments model + migration (unblocks true env Diff).
2. Project Templates picker.
3. Git Safety Helper (read-only fs command).
4. CLI phase 1 (`list`, `get`, `export`, `import`, `diff`) — reuse devtools/core.
5. MCP approval policies + Activity Log.
6. Port `devtools.js` logic into `envyou-core` (Rust) so the app and CLI share
   one implementation.
