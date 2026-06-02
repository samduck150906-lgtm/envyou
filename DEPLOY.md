# Deploying the envyou landing page to Vercel (`envyou.dev`)

The marketing site lives in [`landing/`](./landing) as a single self-contained
`index.html` (inline CSS/JS) plus static assets (`og.png`, `robots.txt`,
`sitemap.xml`). The repo-root [`vercel.json`](./vercel.json) sets
`"outputDirectory": "landing"`, so Vercel serves **only** the landing folder as
a static site — the Rust/Tauri code is never built or exposed.

---

## 1. Connect the repo (one-time)

**Recommended — Git integration (auto-deploy on push):**

1. Go to <https://vercel.com/new>.
2. **Import Git Repository** → select `samduck150906-lgtm/envyou`.
3. Framework Preset: **Other** (it's static — no build step).
4. Leave Build Command empty; Output Directory is read from `vercel.json`
   (`landing`). Click **Deploy**.

Every push to `main` now triggers a production deploy; PRs get preview URLs.

**Alternative — Vercel CLI (manual):**

```bash
# from the repo root, on a machine with network + a logged-in Vercel account
npx vercel login
npx vercel --prod      # uses vercel.json → serves landing/
```

---

## 2. Add the custom domain `envyou.dev`

1. In the Vercel project: **Settings → Domains → Add**.
2. Enter `envyou.dev` and also add `www.envyou.dev`.
3. Vercel shows the DNS records to set. Pick **one** of the options below at
   your domain registrar (where `envyou.dev` was bought).

### Option A — Use Vercel nameservers (simplest, recommended)

At the registrar, set the domain's **nameservers** to:

```
ns1.vercel-dns.com
ns2.vercel-dns.com
```

Vercel then manages all DNS automatically (apex + www + SSL). Best when the
domain is dedicated to this site.

### Option B — Keep your current DNS, add records

Add these records at your DNS provider:

| Type  | Name / Host | Value                  | Notes                     |
| ----- | ----------- | ---------------------- | ------------------------- |
| A     | `@`         | `76.76.21.21`          | Apex (`envyou.dev`)       |
| CNAME | `www`       | `cname.vercel-dns.com` | `www.envyou.dev`          |

> Some registrars don't allow CNAME on the apex. If so, use Option A, or use
> your provider's ALIAS/ANAME record pointing the apex at
> `cname.vercel-dns.com`.

4. In **Settings → Domains**, set `envyou.dev` as the **Primary** domain and let
   Vercel **redirect `www.envyou.dev` → `envyou.dev`** (or vice-versa — pick one
   canonical host; the page's `<link rel="canonical">` already points at the
   apex `https://envyou.dev/`).

---

## 3. SSL / HTTPS

Vercel issues and auto-renews a free Let's Encrypt certificate once DNS resolves
to Vercel. No action needed; just wait for **"Valid Configuration"** in the
Domains panel (usually minutes, up to ~48h for full DNS propagation).

---

## 4. Verify

```bash
# DNS points at Vercel
dig +short envyou.dev            # → 76.76.21.21 (Option B) or Vercel IPs (Option A)
dig +short www.envyou.dev        # → cname.vercel-dns.com.

# Site + OG image are live
curl -sI https://envyou.dev/            | head -n1   # HTTP/2 200
curl -sI https://envyou.dev/og.png      | head -n1   # HTTP/2 200
```

Then validate the social preview (the OG image only resolves once the domain is
live, since the tags use the absolute `https://envyou.dev/og.png`):

- Open Graph / general: <https://www.opengraph.xyz/url/https%3A%2F%2Fenvyou.dev>
- Twitter/X card: <https://cards-dev.twitter.com/validator>
- Facebook (force re-scrape): <https://developers.facebook.com/tools/debug/>
- KakaoTalk caches aggressively — re-share after deploy to refresh.

---

## Files reference

| File                   | Purpose                                             |
| ---------------------- | --------------------------------------------------- |
| `vercel.json`          | `outputDirectory: landing` + security headers       |
| `landing/index.html`   | The entire site (inline CSS/JS)                     |
| `landing/og.png`       | 1200×630 retro social share image                   |
| `landing/robots.txt`   | Allow all + sitemap pointer                         |
| `landing/sitemap.xml`  | Single-URL sitemap for `envyou.dev`                 |
