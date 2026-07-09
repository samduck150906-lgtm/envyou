# envyou license system

How an envyou Pro purchase becomes a working, offline-verifiable Pro unlock —
and why the design cannot be forged on the client or broken by an email client.

> **TL;DR** — Buyers get a short, pretty **license code**
> (`ENVY-K7M4-9Q2P-D8X6-R3TA`). That code is only a database lookup key. When
> the buyer activates, the app trades the code + email for a **signed
> certificate** and verifies that certificate **offline** against a public key
> baked into the app. The private signing key never ships and never touches
> this repo.

---

## Two layers, on purpose

| Layer | What it is | Who sees it | Who checks it |
| --- | --- | --- | --- |
| **License code** | `ENVY-XXXX-XXXX-XXXX-XXXX` — 16 random chars over an unambiguous alphabet, grouped in fours. ~78 bits of entropy. | The buyer (email, success page, deep link). | The **server** (looks it up in the DB). |
| **Signed certificate** | `<payload>.<signature>` — a JSON [`LicenseClaims`] doc + a 64-byte Ed25519 signature, URL-safe base64. | Nobody, normally (stored inside the app; also pasteable in the *Advanced* box). | The **app**, fully offline, against the embedded public key. |

Keeping them separate is what makes the visible key short and typo-friendly
*and* the verification cryptographic. A short code alone could be guessed or
faked; a raw signed token is long and gets mangled by email clients. Splitting
them gets both properties.

### Why not just email the signed token? (the bug this replaces)

The previous scheme emailed the whole `<payload>.<signature>` token and asked
the app to verify it directly. Two real failures came out of that:

1. **`invalid license signature` on paste.** Email clients hard-wrap long
   lines, injecting `\r\n` and spaces into the token. The signature is over the
   *exact* payload bytes, so any injected whitespace broke it.
2. **Key-mismatch risk.** If the shipped public key didn't correspond to the
   webhook's signing key, every real license was rejected with the same opaque
   error.

Both are designed out now: buyers copy a short code (nothing to wrap), the
certificate is delivered *by the server to the app*, so the keys are matched by
construction, and verification strips **all** whitespace as a belt-and-braces
measure (`verify_license_with_key` in `license.rs`).

---

## Anatomy of a license code

```
ENVY - K7M4 - 9Q2P - D8X6 - R3TA
└┬─┘   └──────────┬──────────┘
prefix        16 body chars
```

- **Alphabet:** `ABCDEFGHJKMNPQRSTUVWXYZ23456789` — A–Z and 2–9 with the
  ambiguous glyphs `I L O 0 1` removed, so a code is safe to read aloud and
  retype.
- **Generation:** `generate_license_code()` (issuer-only) uses the OS CSPRNG
  with **rejection sampling**, so there is no modulo bias — every symbol is
  equally likely.
- **Normalization:** `normalize_license_code()` uppercases, drops anything that
  isn't a letter or digit, and regroups in fours. It is tolerant (missing/extra
  hyphens, spaces, lowercase, a missing `ENVY` prefix) and idempotent, so a
  lookup matches no matter how the buyer typed it.
- **Uniqueness:** a `unique` constraint on `license_code` in the DB is the
  ultimate collision guard; the webhook regenerates on the (astronomically
  rare) clash. See [Issuance](#issuance-paddle-webhook).

---

## Anatomy of a certificate

The certificate is exactly the token `license.rs` has always verified, now
carrying a few v2 fields. `LicenseClaims`:

| Field (JSON) | Meaning |
| --- | --- |
| `product` | Must equal `"envyou"`. |
| `plan` | `"pro-lifetime"` (one-time) or `"pro"` (annual). Only these grant Pro. |
| `hardwareId` | *(optional)* machine binding. Currently unset — licenses float across a buyer's devices, capped by the activation limit instead. |
| `issuedAt` | ISO-8601 issue time. |
| `expiresAt` | *(optional)* ISO-8601 expiry. Omitted for lifetime; set ~372 days out for annual. Past expiry ⇒ rejected. |
| `features` | e.g. `unlimited_projects`, `unlimited_variables`, `mcp`, `custom_environment_colors`, `lifetime_updates`. |
| `licenseId` *(v2)* | The DB row id. |
| `emailHash` *(v2)* | `SHA-256(normalized_email)` — binds the certificate to the buyer without embedding their raw email. |
| `codeHash` *(v2)* | `SHA-256(license_code)` — binds the certificate to its code. |
| `schemaVersion` *(v2)* | `2` for short-code + server activation. |

Verification (`verify_license`) checks, in order: **signature validity →
product scope → hardware binding (if any) → expiry (if any)**. `grants_pro`
then requires a Pro-tier `plan`, so a validly-signed non-Pro token never flips
the app into Pro. `is_pro_active` is the single predicate the app consults on
every load — editing the local state file to flip `isPro` grants nothing,
because the stored certificate is re-verified each time.

---

## Data store (Supabase)

Project `dfslueqzfmvtpdencasw` (`envyou`), table `public.licenses`:

| Column | Type | Notes |
| --- | --- | --- |
| `id` | uuid PK | Also embedded in the cert as `licenseId`. |
| `license_code` | text **unique** | `ENVY-…`, the buyer-visible key. |
| `email` | text | As entered at checkout (for re-sending). |
| `normalized_email` | text | Lookup/compare key (lowercased, whitespace-stripped). |
| `product` | text | `envyou`. |
| `tier` | text | `pro_lifetime` / `pro_annual`. |
| `paddle_transaction_id` | text **unique** | Idempotency key — one license per transaction. |
| `paddle_customer_id` | text | For support lookups. |
| `status` | text | `active` / anything else ⇒ activation refused. |
| `activation_count` / `max_activations` | int | Device/activation cap (default **3**). |
| `signed_certificate` | text | The `<payload>.<signature>` the app verifies. |
| `created_at` / `updated_at` / `last_activated_at` | timestamptz | |

**RLS is ON with no table policies**, so `anon`/`authenticated` cannot read or
write rows directly. All access goes through two `SECURITY DEFINER` RPCs
(granted `EXECUTE` to `anon`) or the webhook's `service_role` key:

- **`activate_license(p_license_code, p_email) → jsonb`** — the app calls this
  with the public anon key. It validates code + email, checks `status` and the
  activation limit, atomically increments `activation_count`, and returns the
  stored `signed_certificate`. On any failure it returns
  `{ok:false, code, message}` with a friendly message; the machine-readable
  `code` is one of `LICENSE_NOT_FOUND`, `EMAIL_MISMATCH`, `LICENSE_INACTIVE`,
  `TOO_MANY_ACTIVATIONS`.
- **`license_status_by_txn(p_txn) → jsonb`** — the checkout **success page**
  polls this by Paddle transaction id to show the code the instant the webhook
  has written it. Returns `{found:false}` or
  `{found:true, license_code, email, tier}`.

Neither RPC exposes anything an attacker couldn't already attempt by guessing a
code, and the certificate they'd receive is still useless without also knowing
the matching email — and is cryptographically inert to modify.

---

## Issuance (Paddle webhook)

On `transaction.completed` the webhook (`payments/paddle-webhook`, service-role
key) does, idempotently:

1. **Dedup** on `paddle_transaction_id` — a retry/duplicate just re-emails the
   existing code, never mints a second license.
2. Map the purchased **price id → plan** (`pro-lifetime` or `pro`).
3. Resolve the **buyer email** — preferring the `license_email` the buyer typed
   on the landing page (carried in Paddle `custom_data`), then the inline
   customer object, then the Paddle API.
4. `generate_license_code()` and `issue_license()` → a fresh code + a signed
   certificate (schema v2, with `emailHash`/`codeHash`).
5. Insert the row (unique constraints guard both the code and the transaction;
   a code clash regenerates, a transaction clash re-sends).
6. Email the **short code** (+ a one-click `envyou://activate` deep link) via
   Resend.

A transient failure (Paddle/Resend/Supabase 5xx, timeout) returns **HTTP 503**
so Paddle retries; a paid license is never silently dropped. See
[`docs/PADDLE_WEBHOOK.md`](PADDLE_WEBHOOK.md).

---

## Key management

- **Private signing key** (`ENVYOU_SIGNING_KEY_B64`, base64 of 32 bytes) lives
  **only** in the issuer's secret store (Railway). It is the crown jewel: it
  can mint any license. It must never be committed, pasted into chat, or
  bundled into the app. `*-signing.key` is gitignored.
- **Public key** (`LICENSE_PUBLIC_KEY_B64` in `license.rs`) ships in the app
  and only *verifies*. Rotating = generate a new keypair, replace this constant,
  set the new seed as the webhook's `ENVYOU_SIGNING_KEY_B64`, and confirm they
  correspond.

**Generate a keypair** (offline, one time):

```bash
cargo run -p envyou-core --features issuer --example license_tool -- \
    keygen envyou-signing.key
# → writes the PRIVATE key to a 0600 file, prints the PUBLIC key to paste
#   into LICENSE_PUBLIC_KEY_B64.
```

**Prove the app will accept the webhook's licenses** before every release
(exits non-zero on mismatch, or if the app still ships the placeholder):

```bash
ENVYOU_SIGNING_KEY_B64=<railway value> \
  cargo run -p envyou-core --features issuer --example license_tool -- checkkey
```

If `checkkey` says `MATCH ✓`, every certificate the webhook signs will verify
in the shipped build. If it doesn't, **do not release** — buyers would get
`invalid license signature`.

> ⚠️ If a signing seed is ever exposed (typed into a shell that logs, pasted
> into chat, committed), treat it as compromised: rotate immediately (new
> keypair, new public key in the app, new release) so old signatures stop being
> honored.

---

## Files

| Concern | File |
| --- | --- |
| Codes, certificate, verify/issue, normalize | `crates/envyou-core/src/core/license.rs` |
| Offline keygen / issue / **checkkey** | `crates/envyou-core/examples/license_tool.rs` |
| Issuance + admin CLI | `payments/paddle-webhook/src/main.rs` |
| App activation commands | `src-tauri/src/commands.rs` (`activate_pro`, `activate_certificate`) |
| Deep-link handler | `src-tauri/src/lib.rs` |
| Activate Pro UI | `src/js/app.js`, `src/js/api.js` |
| Checkout + success page | `landing/index.html`, `landing/success.html` |
| DB schema + RPCs | Supabase migrations `create_licenses`, `activate_license_rpc`, `license_status_by_txn_rpc` |

See [`ACTIVATION_FLOW.md`](ACTIVATION_FLOW.md) for the end-to-end buyer journey.
