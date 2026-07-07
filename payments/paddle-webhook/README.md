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
| `PRICE_ANNUAL` | ✅ | The `pri_...` id of your Annual price. |
| `EMAIL_FROM` | — | e.g. `envyou <licenses@envyou.dev>` (must be a Resend-verified domain). Default is Resend's test sender. |
| `PADDLE_API_BASE` | — | `https://api.paddle.com` (default) or `https://sandbox-api.paddle.com` for Sandbox testing. |
| `PORT` | — | Set automatically by Railway. |

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
