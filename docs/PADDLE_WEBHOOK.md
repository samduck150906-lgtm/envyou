# envyou Paddle webhook — issuance & operations

The Rust service in `payments/paddle-webhook` turns a Paddle Billing purchase
into a stored, signed envyou Pro license and emails the buyer a short code. It
reuses `envyou-core` (with the `issuer` feature) so the certificate format is
**identical** to what the app verifies.

> The webhook **issues**; it does **not** activate. Activation is the app
> calling Supabase's `activate_license` RPC directly. See
> [`ACTIVATION_FLOW.md`](ACTIVATION_FLOW.md).

```
Paddle transaction.completed
  → verify Paddle-Signature (HMAC-SHA256, reject stale timestamps)
  → map price id → plan (pro-lifetime / pro-annual)
  → resolve buyer email (custom_data.license_email → inline → Paddle API)
  → generate short code + mint Ed25519-signed certificate (envyou-core, key from env)
  → INSERT into Supabase licenses (idempotent on paddle_transaction_id)
  → email the SHORT CODE + envyou://activate deep link (Resend)

Paddle adjustment.created (action: refund | chargeback)
  → verify Paddle-Signature
  → look up the license by the adjustment's transaction_id
  → PATCH licenses.status = 'revoked' (blocks further activate_license calls)
```

Revocation only blocks **new** activations — a device that already activated
keeps its offline-verified certificate until the app re-checks online (it
currently doesn't). `credit` adjustments (partial goodwill credits, not full
refunds) do not revoke anything.

## Endpoints

- `POST /webhook/paddle` — Paddle notification destination target
- `GET /health` — health check

## Environment variables (set these in Railway, never in the repo)

| Var | Required | Notes |
| --- | --- | --- |
| `ENVYOU_SIGNING_KEY_B64` | ✅ | Base64 of the 32-byte Ed25519 **private** key — the contents of `envyou-signing.key`. The crown jewel; store only as a Railway secret. Its public half must equal the app's `LICENSE_PUBLIC_KEY_B64` (prove with `checkkey`, below). |
| `PADDLE_WEBHOOK_SECRET` | ✅ | The notification destination's secret (`pdl_ntfset_…`), used to verify signatures. |
| `PADDLE_API_KEY` | ✅ | Server-side API key, used to look up the buyer's email when `custom_data`/inline email is absent. |
| `RESEND_API_KEY` | ✅ | Resend API key for sending the license email. |
| `PRICE_LIFETIME` | ✅ | The `pri_…` id of your Lifetime price → issues `pro-lifetime`. |
| `PRICE_ANNUAL` | — | The `pri_…` id of your Annual price → issues `pro` with a ~372-day expiry. Omit if you only sell lifetime. |
| `SUPABASE_URL` | ✅ | e.g. `https://dfslueqzfmvtpdencasw.supabase.co`. Where licenses are stored. |
| `SUPABASE_SERVICE_ROLE_KEY` | ✅ | Service-role key — bypasses RLS to INSERT/lookup licenses. Secret; never ship it in the app (the app uses the **anon** key + RPCs only). |
| `EMAIL_FROM` | — | e.g. `envyou <licenses@envyou.dev>` (Resend-verified domain). Defaults to Resend's test sender. |
| `PADDLE_API_BASE` | — | `https://api.paddle.com` (default) or `https://sandbox-api.paddle.com` for Sandbox. |
| `PADDLE_MAX_SIGNATURE_AGE_SECS` | — | Max age of a `Paddle-Signature` timestamp. Default `432000` (5 days) — generous so Paddle's delayed retries still verify; idempotency is the real replay defense. |
| `PORT` | — | Set automatically by Railway. |

The service **fails fast at startup** if any required var is missing/empty, if
`ENVYOU_SIGNING_KEY_B64` isn't valid base64, or if it doesn't decode to exactly
32 bytes.

## Reliability & idempotency

- **Dedup on `paddle_transaction_id`** (UNIQUE in the DB): Paddle's retries and
  duplicate deliveries never mint or email a second license — a repeat just
  re-emails the existing code. A same-transaction race that trips the unique
  constraint re-sends the stored code and stops.
- **Code-collision safety:** on the astronomically rare `license_code` clash the
  webhook regenerates (up to 6 attempts); the DB `unique` constraint is the
  final guard.
- **Transient failures return HTTP 503** (Paddle/Resend/Supabase 5xx, 429, 408,
  timeout, network) so Paddle retries — a paid license is never silently
  dropped. The row is inserted and the email sent before returning 200.
- **Permanent failures return 200** and log `ALERT …` (e.g. no Pro price in the
  transaction) so you can notice and re-issue manually rather than have Paddle
  retry forever.
- **Body cap:** requests over 256 KiB are rejected (413) before authentication.
- **HTTP timeouts:** all outbound calls (Paddle, Resend, Supabase) use a 10s
  connect / 20s total timeout so a hung dependency can't wedge the handler.

Because licenses now live in Supabase (not a local JSON file), **no Railway
volume is needed** for idempotency — the DB's unique constraints hold across
redeploys and multiple instances.

## Buyer email resolution

To make sure the code goes exactly where the buyer expects, `customer_email`
prefers, in order:

1. `data.custom_data.license_email` — the address the buyer typed in the
   landing page's email modal (passed through Paddle).
2. `data.customer.email` — the inline customer object, if present.
3. The Paddle API (`GET /customers/{id}`) — fallback lookup.

If the typed `license_email` differs from Paddle's customer email, it logs a
note and uses the typed one.

## Admin CLI (same binary)

Run with the same env (`SUPABASE_URL` + `SUPABASE_SERVICE_ROLE_KEY` at minimum;
`resend-license` also needs `RESEND_API_KEY`/`EMAIL_FROM`):

```bash
paddle-webhook lookup-license     <email>        # code, tier, status, activations, created
paddle-webhook resend-license     <email>        # re-send the license email
paddle-webhook reset-activations  <license-code> # set activation_count back to 0
paddle-webhook revoke-license     <license-code> # set status = 'revoked' (manual refund/support)
paddle-webhook reactivate-license <license-code> # set status = 'active' (undo a mistaken revoke)
```

## Before going live — prove the app will accept your licenses

The desktop app embeds `LICENSE_PUBLIC_KEY_B64`; this webhook signs with
`ENVYOU_SIGNING_KEY_B64`. If they don't correspond, buyers get certificates the
app rejects with `invalid license signature`. Confirm the match against the
exact build you ship (exits non-zero on mismatch, or if the app still ships the
placeholder key):

```bash
ENVYOU_SIGNING_KEY_B64=<railway value> \
  cargo run -p envyou-core --features issuer --example license_tool -- checkkey
```

## Deploy on Railway

1. **New Project → Deploy from GitHub repo** → pick this repo.
2. Service **Settings**:
   - **Root Directory:** `/` (repo root — the Dockerfile needs `crates/envyou-core`)
   - **Dockerfile Path:** `payments/paddle-webhook/Dockerfile`
3. **Variables:** add every required var above (including `SUPABASE_URL` and
   `SUPABASE_SERVICE_ROLE_KEY`). Paste `ENVYOU_SIGNING_KEY_B64` as the base64
   contents of `envyou-signing.key`.
4. Deploy → Railway gives a public URL like `https://<name>.up.railway.app`.
5. In **Paddle → Developer Tools → Notifications**, set the destination URL to
   `https://<name>.up.railway.app/webhook/paddle` and subscribe to
   `transaction.completed` **and `adjustment.created`** (the latter revokes a
   license's `status` on refund/chargeback — without it, refunded buyers keep
   a working license).

## Test in Sandbox first

- Set `PADDLE_API_BASE=https://sandbox-api.paddle.com` and use Sandbox webhook
  secret / API key / price ids.
- Make a test purchase; confirm the email arrives with a short `ENVY-…` code and
  that entering it (with the same email) in **Activate Pro** unlocks Pro.
- Then switch env vars to production values and repeat once for real.

## Local run

```bash
cd payments/paddle-webhook
ENVYOU_SIGNING_KEY_B64=… PADDLE_WEBHOOK_SECRET=… PADDLE_API_KEY=… \
RESEND_API_KEY=… PRICE_LIFETIME=pri_… \
SUPABASE_URL=https://….supabase.co SUPABASE_SERVICE_ROLE_KEY=… \
cargo run
# expose with: ngrok http 8080  → use that URL in Paddle for testing
```

> ⚠️ The private key gates every paid license. Keep it only in your secret
> manager and Railway's env — never commit it. `*-signing.key` is gitignored.
> If it's ever exposed, rotate it (see [`LICENSE_SYSTEM.md`](LICENSE_SYSTEM.md)
> → *Key management*).
