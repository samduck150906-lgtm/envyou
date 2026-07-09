# envyou v0.1.3 — short license codes + one-click activation

This release replaces the old "paste the whole signed token" activation with a
short, human-friendly **license code** and a server-delivered signed
certificate. It fixes the `invalid license signature` error and makes buying →
activating Pro a few clicks.

## ✨ What's new

- **Short license codes.** Buyers now get a code like `ENVY-K7M4-9Q2P-D8X6-R3TA`
  instead of a long token — nothing to line-wrap, easy to read and retype.
- **One-click activation.** Activate from the purchase success page, the email,
  or the `envyou://activate` deep link — or just type your email + code into
  **Activate Pro**. The code field auto-formats as you type.
- **Works offline after activation.** The app verifies a signed certificate
  against its embedded public key and re-checks it on every launch, so Pro keeps
  working with no network — and can't be faked by editing local state.
- **Email-first checkout.** The landing page collects your license email up
  front so the code lands exactly where you expect.

## 🐛 Fixes

- **`invalid license signature` on activation.** Root cause was email clients
  wrapping the long token and a possible app/issuer key mismatch. Both are
  designed out: codes don't wrap, the certificate is delivered server→app (keys
  matched by construction), and verification now strips all whitespace.

## 🔒 Security

- The app holds only the **public** verification key — it can verify a license
  but never mint one. The signing key never ships.
- No client-side secrets: activation goes through locked-down Supabase RPCs
  using a public anon key.
- Activation is limited per license (default 3 devices); contact support to
  reset.

## Install

Download the asset for your platform below. On first run, open **Activate Pro**
and enter the license email + code from your purchase email.

---

_Details: `docs/LICENSE_SYSTEM.md`, `docs/ACTIVATION_FLOW.md`,
`docs/PADDLE_WEBHOOK.md`._
