# envyou activation flow

The end-to-end journey from "Buy" to a working Pro unlock, and every way a
buyer can get from their code to activated. Companion to
[`LICENSE_SYSTEM.md`](LICENSE_SYSTEM.md) (the *what*); this is the *how it
flows*.

---

## The happy path, end to end

```
┌─────────────┐   1. enter email    ┌──────────────┐
│  landing    │────────────────────▶│  Paddle       │
│ index.html  │  (email modal)      │  checkout     │
└─────────────┘                     └──────┬────────┘
      ▲                                    │ 2. transaction.completed
      │                                    ▼
      │                            ┌────────────────┐
      │                            │ paddle-webhook  │  3. generate code + sign cert
      │                            │   (Railway)     │──────────────┐
      │                            └───────┬─────────┘              │
      │                                    │ 4. email short code    │ 3b. INSERT row
      │                                    ▼                        ▼
      │                            ┌────────────────┐        ┌──────────────┐
      │  5a. success page polls    │  buyer inbox    │        │  Supabase     │
      └────────────────────────────│  (Resend email) │        │  licenses     │
         license_status_by_txn     └───────┬─────────┘        └──────┬───────┘
                                           │ 5b. code + deep link    │
                                           ▼                         │
                                   ┌────────────────┐  6. activate_license(code,email)
                                   │  envyou app     │─────────────────┘
                                   │ Activate Pro    │◀──── signed_certificate
                                   └───────┬─────────┘
                                           │ 7. verify cert OFFLINE, store
                                           ▼
                                     ✅ Pro unlocked (persists offline)
```

1. **Email-first checkout.** On the landing page, clicking a Pro **Buy** button
   opens a small email modal (`#emailOverlay`) that collects and confirms the
   buyer's email *before* Paddle opens. That email is passed to Paddle as both
   `customer.email` and `customData.license_email`, and the checkout's
   `successUrl` is set to `https://envyou.dev/success`.
2. **Paddle fires `transaction.completed`** to the webhook.
3. **The webhook issues** a short code + a signed certificate and **inserts**
   the row into Supabase (idempotent on the transaction id).
4. **The webhook emails** the buyer their short code, with a one-click
   `envyou://activate` deep link, via Resend.
5. The buyer now has **three** ways to get their code (any one works):
   - **5a. Success page** (`landing/success.html`): reads `_ptxn` from the
     redirect URL and polls `license_status_by_txn` (~75s) so the code appears
     on-screen seconds after purchase, with **Copy** and an **Activate in
     envyou** deep-link button.
   - **5b. Email**: the same code + deep link, as a durable proof of purchase.
   - **Deep link**: `envyou://activate?email=…&code=…` — from either the
     success page or the email.
6. **Activation.** In the app, **Activate Pro** (or the deep link) calls the
   Supabase `activate_license` RPC with the code + email using the public anon
   key. The RPC validates and returns the stored `signed_certificate`.
7. **Offline verification.** The app verifies that certificate against its
   embedded public key, checks it grants Pro, and stores it. **Done** — Pro now
   works with no network, and every load re-verifies the stored certificate.

---

## Three ways to activate (all land in the same place)

| Path | Trigger | What the app does |
| --- | --- | --- |
| **Type it in** | Buyer opens **Activate Pro**, enters email + code. | `activate_pro(email, code)` → RPC → verify cert offline → store. |
| **Deep link** | Buyer clicks `envyou://activate?email=…&code=…` (email or success page). | App receives the URL, emits `deep-link-activate`, the UI pre-fills the modal and activates. |
| **Advanced: paste certificate** | Buyer expands *Advanced* and pastes a `<payload>.<signature>` certificate. | `activate_certificate(cert)` → verify offline → store. No network — useful for support / air-gapped re-activation. |

The code input **auto-formats** as the buyer types (`formatLicenseCode`):
uppercases, strips non-alphanumerics, and regroups in fours, up to 20 chars.
Whatever they paste, `normalize_license_code` canonicalizes it on submit, so
`envy k7m4 9q2p…`, `ENVYK7M4…`, and the exact `ENVY-K7M4-…` all match.

---

## What the buyer sees when something's off

Errors are surfaced as friendly, human messages; the machine-readable reason is
only logged. From `activate_license`:

| Situation | Message |
| --- | --- |
| Unknown code | "We couldn't find this license code. Please check the code or contact support." |
| Right code, wrong email | "This license code is registered to a different email address." |
| Deactivated license | "This license is no longer active. Please contact support." |
| Over the device limit | "This license has reached its activation limit. Contact support to reset it." |
| Network / server down | "Couldn't reach the activation server. Check your internet connection and try again." |
| Cert won't verify locally | "We activated your license but couldn't verify it on this device. Please update envyou…" |

The activation limit defaults to **3**. Support can reset it with
`paddle-webhook reset-activations <code>` (see
[`PADDLE_WEBHOOK.md`](PADDLE_WEBHOOK.md)).

---

## If the code never arrives

The success page falls back after ~75s (or when there's no `_ptxn`) to a
message telling the buyer their code is on its way by email (check spam), with
a download button and a support contact. The webhook's HTTP-503-on-transient
behavior means Paddle keeps retrying until the email actually sends, so a code
is never silently lost — worst case it's delayed, not dropped.

Support can always look up or re-send a code by email:

```bash
paddle-webhook lookup-license <email>   # show code, tier, activations
paddle-webhook resend-license <email>   # re-send the license email
```

---

## Why this is safe

- The app **cannot mint** a certificate — it holds only the public key. A
  guessed code returns a certificate that's still bound to the right email and
  cryptographically inert to alter.
- Pro state is **re-verified on every load** from the stored certificate, so
  hand-editing the local state file to set `isPro:true` does nothing.
- No secret ships in the app or the landing page: the Supabase URL and anon key
  are public and only reach the two `SECURITY DEFINER` RPCs; the signing key
  lives only on the webhook host.

See [`LICENSE_SYSTEM.md`](LICENSE_SYSTEM.md) for the cryptographic details and
[`PADDLE_WEBHOOK.md`](PADDLE_WEBHOOK.md) for issuance + operations.
