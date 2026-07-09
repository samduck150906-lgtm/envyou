# envyou Paddle webhook

A tiny Rust service that turns a Paddle Billing purchase into a stored,
Ed25519-signed envyou Pro license and **emails the buyer a short license code**
(`ENVY-XXXX-XXXX-XXXX-XXXX`). It reuses `envyou-core` (with the `issuer`
feature) so the signed certificate format is **identical** to what the app
verifies offline.

```
Paddle transaction.completed
  → verify Paddle-Signature (HMAC-SHA256, reject stale timestamps)
  → map price id → plan (pro-lifetime / pro-annual)
  → resolve buyer email (custom_data.license_email → inline → Paddle API)
  → generate short code + mint signed certificate (private key from env)
  → INSERT into Supabase licenses (idempotent on paddle_transaction_id)
  → email the SHORT CODE + envyou://activate deep link (Resend)
```

The webhook **issues**; the app **activates** by calling Supabase's
`activate_license` RPC directly, which returns the stored certificate for the
app to verify offline. It never emails the raw signed token anymore — buyers
copy a short code, so nothing gets mangled by an email client.

## 📖 Full documentation

Architecture, the complete env-var table, idempotency/reliability, the admin
CLI, Railway deploy steps, and Sandbox testing all live in:

**[`../../docs/PADDLE_WEBHOOK.md`](../../docs/PADDLE_WEBHOOK.md)**

See also [`docs/LICENSE_SYSTEM.md`](../../docs/LICENSE_SYSTEM.md) (the code +
certificate design and key management) and
[`docs/ACTIVATION_FLOW.md`](../../docs/ACTIVATION_FLOW.md) (the end-to-end buyer
journey).

## Quick reference

Required env (Railway secrets — never in the repo):
`ENVYOU_SIGNING_KEY_B64`, `PADDLE_WEBHOOK_SECRET`, `PADDLE_API_KEY`,
`RESEND_API_KEY`, `PRICE_LIFETIME`, `SUPABASE_URL`, `SUPABASE_SERVICE_ROLE_KEY`.

Prove the app will accept these licenses before every release:

```bash
ENVYOU_SIGNING_KEY_B64=<railway value> \
  cargo run -p envyou-core --features issuer --example license_tool -- checkkey
```

Admin CLI (same binary):

```bash
paddle-webhook lookup-license    <email>
paddle-webhook resend-license    <email>
paddle-webhook reset-activations <license-code>
```

> ⚠️ The private key gates every paid license. Keep it only in your secret
> manager and Railway's env — never commit it. `*-signing.key` is gitignored.
