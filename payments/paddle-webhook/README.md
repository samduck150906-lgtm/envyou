# envyou Paddle webhook

A tiny Rust service that turns a Paddle Billing purchase into an emailed,
Ed25519-signed envyou Pro license. It reuses `envyou-core` (with the `issuer`
feature) so the token format is **identical** to what the app verifies.

```
Paddle transaction.completed
  → verify Paddle-Signature (HMAC-SHA256)
  → map price id → plan (lifetime / annual)
  → look up buyer email (Paddle API)
  → mint signed license (envyou-core, private key from env)
  → email it (Resend)
```

## Endpoints
- `POST /webhook/paddle` — Paddle notification destination target
- `GET /health` — health check

## Environment variables (set these in Railway, never in the repo)

| Var | Required | Notes |
| --- | --- | --- |
| `ENVYOU_SIGNING_KEY_B64` | ✅ | Base64 of your 32-byte private key — i.e. the **contents** of `envyou-signing.key`. This is the crown jewel; store it only as a Railway secret. |
| `PADDLE_WEBHOOK_SECRET` | ✅ | The notification destination's **secret key** (`pdl_ntfset_...`), used to verify signatures. |
| `PADDLE_API_KEY` | ✅ | Server-side API key, used to look up the buyer's email. |
| `RESEND_API_KEY` | ✅ | Resend API key for sending the license email. |
| `PRICE_LIFETIME` | ✅ | The `pri_...` id of your Lifetime price. |
| `PRICE_ANNUAL` | — | The `pri_...` id of your Annual price. Omit if you only sell the lifetime plan. |
| `EMAIL_FROM` | — | e.g. `envyou <licenses@envyou.dev>` (must be a Resend-verified domain). Default is Resend's test sender. |
| `PADDLE_API_BASE` | — | `https://api.paddle.com` (default) or `https://sandbox-api.paddle.com` for Sandbox testing. |
| `IDEMPOTENCY_FILE` | — | Path to the JSON file recording already-processed transaction ids. Default `processed_transactions.json` (relative). **Put this on a persistent volume** (see below) so dedup survives redeploys. |
| `PADDLE_MAX_SIGNATURE_AGE_SECS` | — | Max age of a `Paddle-Signature` timestamp before it's rejected. Default `432000` (5 days) — generous so Paddle's own delayed retries still verify; idempotency is the primary replay defense. |
| `PORT` | — | Set automatically by Railway. |

## Reliability & idempotency

- The webhook **dedupes on the Paddle transaction id**, so Paddle's retries and
  occasional duplicate deliveries never mint or email a second license.
- A **transient** failure (Paddle/Resend `5xx`, `429`, timeout, network) returns
  **HTTP 503** so Paddle retries — a paid license is never silently dropped. A
  transaction is only marked processed *after* the license email succeeds.
- A **permanent** failure (no Pro price, missing buyer email) returns `200` and
  logs `ALERT ...` so you can notice and re-issue manually.
- Persist `IDEMPOTENCY_FILE` on a Railway **Volume** (Service → Settings →
  Volumes; mount e.g. `/data`, then set
  `IDEMPOTENCY_FILE=/data/processed_transactions.json`). Without a volume, dedup
  only holds within a container's lifetime — which still covers Paddle's burst
  retries, but not dedup across a redeploy.

## Before going live — prove the app will accept your licenses

The desktop app embeds `LICENSE_PUBLIC_KEY_B64`; this webhook signs with
`ENVYOU_SIGNING_KEY_B64`. If they don't correspond, buyers get keys the app
rejects. Confirm the match against the exact build you ship (exits non-zero on
mismatch, or if the app still ships the placeholder key):

```bash
ENVYOU_SIGNING_KEY_B64=... cargo run -p envyou-core --features issuer \
    --example license_tool -- checkkey
```

## Deploy on Railway

1. **New Project → Deploy from GitHub repo** → pick this repo.
2. In the service **Settings**:
   - **Root Directory:** `/` (repo root — the Dockerfile needs `crates/envyou-core`)
   - **Dockerfile Path:** `payments/paddle-webhook/Dockerfile`
3. **Variables:** add every required var above. Paste `ENVYOU_SIGNING_KEY_B64`
   as the base64 contents of your `envyou-signing.key` file.
4. Deploy. Railway gives you a public URL like
   `https://<name>.up.railway.app`.
5. In **Paddle → Developer Tools → Notifications**, set the destination URL to
   `https://<name>.up.railway.app/webhook/paddle` and subscribe to
   `transaction.completed`.

## Test in Sandbox first

- Set `PADDLE_API_BASE=https://sandbox-api.paddle.com` and use your Sandbox
  webhook secret / API key / price ids.
- Make a test purchase; confirm the email arrives and the token activates Pro in
  the app.
- Then switch the env vars to production values and repeat once for real.

## Local run

```bash
cd payments/paddle-webhook
ENVYOU_SIGNING_KEY_B64=... PADDLE_WEBHOOK_SECRET=... PADDLE_API_KEY=... \
RESEND_API_KEY=... PRICE_LIFETIME=pri_... PRICE_ANNUAL=pri_... \
cargo run
# expose it with: ngrok http 8080  → use that URL in Paddle for testing
```

> ⚠️ The private key gates every paid license. Keep it only in your secret
> manager and Railway's env — never commit it. `*-signing.key` is gitignored.
