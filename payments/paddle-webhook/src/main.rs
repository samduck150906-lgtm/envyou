//! envyou Paddle Billing webhook.
//!
//! Flow: Paddle sends `transaction.completed` -> we verify the HMAC signature
//! (and reject stale timestamps) -> dedupe on the transaction id -> map the
//! purchased price to a plan -> look up the buyer's email -> mint an
//! Ed25519-signed license with envyou-core (same format the app verifies) ->
//! email it via Resend.
//!
//! Reliability contract with Paddle:
//! * A transient failure (Paddle/Resend 5xx, timeout, network) returns **HTTP
//!   503** so Paddle retries — a paid license is never silently dropped.
//! * A permanent/ignorable outcome (wrong event, no Pro price, already handled)
//!   returns **200** so Paddle stops retrying.
//! * Every purchase is deduped on the Paddle transaction id, so those retries
//!   (and Paddle's occasional duplicate deliveries) never mint a second license.
//!
//! ALL secrets come from environment variables — nothing is baked into the
//! binary or the repo. See README.md for the full list and deploy steps.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use envyou_core::core::license::{issue_license, LicenseClaims, PRODUCT};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

/// Hard cap on the request body we will read before authenticating it, so an
/// unauthenticated caller cannot exhaust memory with a huge/slow POST.
const MAX_BODY_BYTES: usize = 256 * 1024;

/// Default max age (seconds) of a `Paddle-Signature` timestamp. Deliberately
/// generous — Paddle re-delivers the *same* signature on delayed retries (up to
/// a few days), so a tight window would reject legitimate retries. The real
/// replay defense is idempotency; this only bounds ancient captured replays.
const DEFAULT_MAX_SIGNATURE_AGE_SECS: u64 = 5 * 24 * 60 * 60; // 5 days

/// Runtime configuration, all from env.
struct Config {
    webhook_secret: String, // PADDLE_WEBHOOK_SECRET (the destination's secret key)
    signing_seed: [u8; 32], // ENVYOU_SIGNING_KEY_B64 (base64 of the 32-byte private key)
    paddle_api_key: String, // PADDLE_API_KEY (server-side, to look up customer email)
    paddle_api_base: String, // PADDLE_API_BASE (prod default; sandbox: https://sandbox-api.paddle.com)
    resend_api_key: String,  // RESEND_API_KEY
    email_from: String,      // EMAIL_FROM, e.g. "envyou <licenses@envyou.dev>"
    price_lifetime: String,  // PRICE_LIFETIME (pri_...)
    price_annual: String,    // PRICE_ANNUAL (pri_...)
    max_sig_age: u64,        // PADDLE_MAX_SIGNATURE_AGE_SECS
}

/// A required secret. Fails fast if the var is missing **or empty** — an empty
/// `PADDLE_WEBHOOK_SECRET` would key the HMAC on "" and make signatures forgeable.
fn require(name: &str) -> String {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => {
            eprintln!("FATAL: missing or empty required env var {name}");
            std::process::exit(1);
        }
    }
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name)
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| default.to_string())
}

fn load_config() -> Config {
    let signing_seed_b64 = require("ENVYOU_SIGNING_KEY_B64");
    let seed_bytes = B64.decode(signing_seed_b64.trim()).unwrap_or_else(|_| {
        eprintln!("FATAL: ENVYOU_SIGNING_KEY_B64 is not valid base64");
        std::process::exit(1);
    });
    let signing_seed: [u8; 32] = seed_bytes.as_slice().try_into().unwrap_or_else(|_| {
        eprintln!("FATAL: ENVYOU_SIGNING_KEY_B64 must decode to exactly 32 bytes");
        std::process::exit(1);
    });

    Config {
        webhook_secret: require("PADDLE_WEBHOOK_SECRET"),
        signing_seed,
        paddle_api_key: require("PADDLE_API_KEY"),
        paddle_api_base: env_or("PADDLE_API_BASE", "https://api.paddle.com"),
        resend_api_key: require("RESEND_API_KEY"),
        email_from: env_or("EMAIL_FROM", "envyou <onboarding@resend.dev>"),
        price_lifetime: require("PRICE_LIFETIME"),
        // Optional: only set if you also sell an annual plan. Empty = lifetime only.
        price_annual: env_or("PRICE_ANNUAL", ""),
        max_sig_age: env_or(
            "PADDLE_MAX_SIGNATURE_AGE_SECS",
            &DEFAULT_MAX_SIGNATURE_AGE_SECS.to_string(),
        )
        .parse()
        .unwrap_or(DEFAULT_MAX_SIGNATURE_AGE_SECS),
    }
}

/// A handler failure, tagged so `route` can pick the HTTP status: transient →
/// 503 (Paddle retries), permanent → 200 (don't retry; log for the operator).
struct HErr {
    transient: bool,
    msg: String,
}
impl HErr {
    fn transient(m: impl Into<String>) -> Self {
        HErr {
            transient: true,
            msg: m.into(),
        }
    }
    fn permanent(m: impl Into<String>) -> Self {
        HErr {
            transient: false,
            msg: m.into(),
        }
    }
}

fn ureq_transient(e: &ureq::Error) -> bool {
    match e {
        // 5xx / rate-limit / request-timeout are worth retrying; other 4xx aren't.
        ureq::Error::Status(code, _) => *code >= 500 || *code == 429 || *code == 408,
        // DNS / connect / read timeout / reset — all transient.
        ureq::Error::Transport(_) => true,
    }
}

/// Idempotency store keyed on the Paddle transaction id. Backed by a JSON file
/// so it survives process restarts **when the path is on a persistent volume**
/// (set `IDEMPOTENCY_FILE` to a mounted volume path on Railway; otherwise it
/// dedupes within the container's lifetime, which already covers Paddle's burst
/// retries). Falls back to in-memory only if the file can't be written.
struct Idempotency {
    path: Option<PathBuf>,
    seen: Mutex<HashSet<String>>,
}

impl Idempotency {
    fn load(path: Option<PathBuf>) -> Self {
        let mut seen = HashSet::new();
        if let Some(p) = &path {
            if let Ok(txt) = std::fs::read_to_string(p) {
                if let Ok(v) = serde_json::from_str::<HashSet<String>>(&txt) {
                    seen = v;
                }
            }
        }
        Idempotency {
            path,
            seen: Mutex::new(seen),
        }
    }

    fn contains(&self, id: &str) -> bool {
        self.seen.lock().map(|s| s.contains(id)).unwrap_or(false)
    }

    fn mark(&self, id: &str) {
        let mut guard = match self.seen.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        guard.insert(id.to_string());
        if let Some(p) = &self.path {
            match serde_json::to_string(&*guard) {
                Ok(txt) => {
                    // Write then rename so a crash mid-write can't corrupt the file.
                    let tmp = p.with_extension("json.tmp");
                    if std::fs::write(&tmp, txt)
                        .and_then(|_| std::fs::rename(&tmp, p))
                        .is_err()
                    {
                        eprintln!(
                            "warning: could not persist idempotency file {}",
                            p.display()
                        );
                    }
                }
                Err(e) => eprintln!("warning: could not serialize idempotency set: {e}"),
            }
        }
    }
}

/// ureq agent with connect + overall timeouts so a hung Paddle/Resend call can't
/// stall the (single-threaded) server indefinitely.
fn http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
}

fn main() {
    let cfg = load_config();
    let agent = http_agent();
    let idem_path = std::env::var("IDEMPOTENCY_FILE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| Some(PathBuf::from("processed_transactions.json")));
    if let Some(p) = &idem_path {
        eprintln!("idempotency store: {}", p.display());
    }
    let idem = Idempotency::load(idem_path);

    let port = env_or("PORT", "8080");
    let addr = format!("0.0.0.0:{port}");
    let server = tiny_http::Server::http(addr.as_str()).unwrap_or_else(|e| {
        eprintln!("FATAL: could not bind {addr}: {e}");
        std::process::exit(1);
    });
    eprintln!("paddle-webhook listening on {addr}");

    for mut request in server.incoming_requests() {
        let (status, body) = route(&mut request, &cfg, &agent, &idem);
        let response =
            tiny_http::Response::from_string(body).with_status_code(tiny_http::StatusCode(status));
        let _ = request.respond(response);
    }
}

fn route(
    request: &mut tiny_http::Request,
    cfg: &Config,
    agent: &ureq::Agent,
    idem: &Idempotency,
) -> (u16, String) {
    let method = request.method().as_str().to_string();
    let url = request.url().to_string();

    // Health check for the platform (Railway etc.).
    if url == "/health" || url == "/" {
        return (200, "ok".into());
    }
    if method != "POST" || !url.starts_with("/webhook/paddle") {
        return (404, "not found".into());
    }

    // Grab the signature header before consuming the body.
    let signature = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Paddle-Signature"))
        .map(|h| h.value.as_str().to_string());

    // Bounded body read: reject oversized up front (Content-Length) and cap the
    // actual read so a lying/absent length can't blow past the limit either.
    if let Some(len) = request.body_length() {
        if len > MAX_BODY_BYTES {
            return (413, "payload too large".into());
        }
    }
    let mut buf: Vec<u8> = Vec::new();
    {
        let reader = request.as_reader();
        let mut chunk = [0u8; 8192];
        loop {
            match reader.read(&mut chunk) {
                Ok(0) => break,
                Ok(n) => {
                    if buf.len() + n > MAX_BODY_BYTES {
                        return (413, "payload too large".into());
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                Err(_) => return (400, "could not read body".into()),
            }
        }
    }
    // Keep the raw body byte-exact for HMAC; Paddle sends UTF-8 JSON.
    let body = match String::from_utf8(buf) {
        Ok(s) => s,
        Err(_) => return (400, "body is not valid UTF-8".into()),
    };

    let signature = match signature {
        Some(s) => s,
        None => return (400, "missing Paddle-Signature".into()),
    };
    if !verify_signature(
        &cfg.webhook_secret,
        &signature,
        &body,
        now_secs(),
        cfg.max_sig_age,
    ) {
        eprintln!("rejected: bad Paddle signature");
        return (401, "invalid signature".into());
    }

    let event: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => return (400, "invalid json".into()),
    };

    // Only act on completed transactions; acknowledge everything else so Paddle
    // does not retry.
    let event_type = event
        .get("event_type")
        .and_then(Value::as_str)
        .unwrap_or("");
    if event_type != "transaction.completed" {
        return (200, format!("ignored event {event_type}"));
    }

    match handle_transaction(cfg, agent, idem, &event) {
        Ok(msg) => (200, msg),
        Err(e) if e.transient => {
            // 503 → Paddle retries; combined with idempotency this is safe and
            // means a transient Resend/API blip never drops a paid license.
            eprintln!("transient error, asking Paddle to retry: {}", e.msg);
            (503, "temporary error; please retry".into())
        }
        Err(e) => {
            // Retrying won't help; surface loudly so the operator can re-issue.
            eprintln!("ALERT permanent handling error (will NOT retry): {}", e.msg);
            (200, "handled with permanent error".into())
        }
    }
}

fn handle_transaction(
    cfg: &Config,
    agent: &ureq::Agent,
    idem: &Idempotency,
    event: &Value,
) -> Result<String, HErr> {
    let data = event
        .get("data")
        .ok_or_else(|| HErr::permanent("event has no data"))?;

    // One license per Paddle transaction. If we've already fully handled this
    // transaction, ack without minting/emailing again.
    let tx_id = data
        .get("id")
        .and_then(Value::as_str)
        .or_else(|| event.get("event_id").and_then(Value::as_str))
        .map(|s| s.to_string());
    if let Some(id) = &tx_id {
        if idem.contains(id) {
            return Ok(format!("transaction {id} already processed; ignored"));
        }
    }

    // Determine the plan from the purchased price ids.
    let mut is_lifetime = false;
    let mut is_annual = false;
    if let Some(items) = data.get("items").and_then(Value::as_array) {
        for item in items {
            let pid = item
                .get("price")
                .and_then(|p| p.get("id"))
                .and_then(Value::as_str)
                .unwrap_or("");
            if pid == cfg.price_lifetime {
                is_lifetime = true;
            } else if !cfg.price_annual.is_empty() && pid == cfg.price_annual {
                is_annual = true;
            }
        }
    }
    if !is_lifetime && !is_annual {
        return Ok("no envyou Pro price in this transaction; ignored".into());
    }

    let (plan, expires_at) = if is_lifetime {
        ("pro-lifetime".to_string(), None)
    } else {
        // Annual: valid ~1 year + a short grace window. The app re-checks expiry
        // on every load, so this actually lapses.
        ("pro".to_string(), Some(add_days_iso(372)))
    };

    let email = customer_email(cfg, agent, data)?;

    let claims = LicenseClaims {
        product: PRODUCT.to_string(),
        plan,
        hardware_id: None, // floating license — works on any of the buyer's machines
        issued_at: now_iso8601(),
        expires_at,
        features: vec![
            "unlimited_projects".to_string(),
            "unlimited_variables".to_string(),
        ],
    };

    let token = issue_license(&cfg.signing_seed, &claims)
        .map_err(|e| HErr::permanent(format!("issue_license failed: {e}")))?;
    send_license_email(cfg, agent, &email, &token)?;

    // Record only after a successful send, so a transient failure that returns
    // 503 gets retried instead of being marked done.
    if let Some(id) = &tx_id {
        idem.mark(id);
    }

    eprintln!("issued license to {email}");
    Ok(format!("license issued to {email}"))
}

/// Look up the buyer's email. Prefer an email already present on the event;
/// otherwise fetch the customer via the Paddle API.
fn customer_email(cfg: &Config, agent: &ureq::Agent, data: &Value) -> Result<String, HErr> {
    // Some payloads include the customer object inline.
    if let Some(email) = data
        .get("customer")
        .and_then(|c| c.get("email"))
        .and_then(Value::as_str)
    {
        return Ok(email.to_string());
    }
    let customer_id = data
        .get("customer_id")
        .and_then(Value::as_str)
        .ok_or_else(|| HErr::permanent("transaction has no customer_id"))?;

    let url = format!("{}/customers/{}", cfg.paddle_api_base, customer_id);
    let resp = agent
        .get(&url)
        .set("Authorization", &format!("Bearer {}", cfg.paddle_api_key))
        .call()
        .map_err(|e| HErr {
            transient: ureq_transient(&e),
            msg: format!("paddle customer lookup failed: {e}"),
        })?;
    let body: Value = resp
        .into_json()
        .map_err(|e| HErr::permanent(format!("paddle customer response parse failed: {e}")))?;
    body.get("data")
        .and_then(|d| d.get("email"))
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .ok_or_else(|| HErr::permanent("customer has no email"))
}

fn send_license_email(
    cfg: &Config,
    agent: &ureq::Agent,
    to: &str,
    token: &str,
) -> Result<(), HErr> {
    let text = format!(
        "Thanks for buying envyou Pro!\n\n\
         Your license key:\n\n{token}\n\n\
         To activate: open envyou, click \"Upgrade to Pro\", and paste the key above.\n\
         Keep this email — it is your proof of purchase.\n",
    );
    let resp = agent
        .post("https://api.resend.com/emails")
        .set("Authorization", &format!("Bearer {}", cfg.resend_api_key))
        .send_json(json!({
            "from": cfg.email_from,
            "to": [to],
            "subject": "Your envyou Pro license",
            "text": text,
        }));
    match resp {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(code, r)) => {
            let detail = r.into_string().unwrap_or_default();
            let transient = code >= 500 || code == 429 || code == 408;
            Err(HErr {
                transient,
                msg: format!("resend returned {code}: {detail}"),
            })
        }
        Err(e) => Err(HErr::transient(format!("resend request failed: {e}"))),
    }
}

/// Verify a Paddle Billing `Paddle-Signature` header of the form
/// `ts=<unix>;h1=<hex hmac>`. The signed payload is `"<ts>:<raw body>"`,
/// HMAC-SHA256 with the destination's secret key. Also rejects a timestamp more
/// than `max_age` seconds from `now` (in either direction) to bound replay.
fn verify_signature(secret: &str, header: &str, body: &str, now: u64, max_age: u64) -> bool {
    let mut ts = "";
    let mut h1 = "";
    for part in header.split(';') {
        if let Some(v) = part.strip_prefix("ts=") {
            ts = v.trim();
        } else if let Some(v) = part.strip_prefix("h1=") {
            h1 = v.trim();
        }
    }
    if ts.is_empty() || h1.is_empty() {
        return false;
    }
    // Reject non-numeric or stale/future timestamps before the constant-time MAC.
    match ts.parse::<u64>() {
        Ok(t) => {
            let age = now.abs_diff(t);
            if age > max_age {
                eprintln!("rejected: Paddle-Signature timestamp {age}s from now (max {max_age}s)");
                return false;
            }
        }
        Err(_) => return false,
    }
    let signed = format!("{ts}:{body}");
    let mut mac = match HmacSha256::new_from_slice(secret.as_bytes()) {
        Ok(m) => m,
        Err(_) => return false,
    };
    mac.update(signed.as_bytes());
    let expected = hex_encode(&mac.finalize().into_bytes());
    constant_time_eq(expected.as_bytes(), h1.as_bytes())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

// ---- time helpers (std only) --------------------------------------------

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn now_iso8601() -> String {
    iso_from_secs(now_secs())
}

fn add_days_iso(days: u64) -> String {
    iso_from_secs(now_secs() + days * 86_400)
}

fn iso_from_secs(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, mon, d) = civil_from_days(days);
    format!("{y:04}-{mon:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
}

fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let mon = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if mon <= 2 { y + 1 } else { y }, mon, d)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sign(secret: &str, ts: u64, body: &str) -> String {
        let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(format!("{ts}:{body}").as_bytes());
        format!("ts={ts};h1={}", hex_encode(&mac.finalize().into_bytes()))
    }

    #[test]
    fn valid_signature_accepted() {
        let now = 1_700_000_000;
        let hdr = sign("sec", now, "{\"a\":1}");
        assert!(verify_signature("sec", &hdr, "{\"a\":1}", now, 300));
    }

    #[test]
    fn wrong_secret_rejected() {
        let now = 1_700_000_000;
        let hdr = sign("sec", now, "body");
        assert!(!verify_signature("other-secret", &hdr, "body", now, 300));
    }

    #[test]
    fn tampered_body_rejected() {
        let now = 1_700_000_000;
        let hdr = sign("sec", now, "body");
        assert!(!verify_signature("sec", &hdr, "body-TAMPERED", now, 300));
    }

    #[test]
    fn stale_and_future_timestamps_rejected() {
        let now = 1_700_000_000;
        assert!(!verify_signature(
            "sec",
            &sign("sec", now - 10_000, "b"),
            "b",
            now,
            300
        ));
        assert!(!verify_signature(
            "sec",
            &sign("sec", now + 10_000, "b"),
            "b",
            now,
            300
        ));
        // Within the window it verifies.
        assert!(verify_signature(
            "sec",
            &sign("sec", now - 100, "b"),
            "b",
            now,
            300
        ));
    }

    #[test]
    fn malformed_headers_rejected() {
        assert!(!verify_signature("sec", "h1=abc", "b", 1_700_000_000, 300));
        assert!(!verify_signature(
            "sec",
            "ts=1700000000",
            "b",
            1_700_000_000,
            300
        ));
        assert!(!verify_signature("sec", "", "b", 1_700_000_000, 300));
        assert!(!verify_signature(
            "sec",
            "ts=notanumber;h1=abc",
            "b",
            1_700_000_000,
            300
        ));
    }

    #[test]
    fn idempotency_dedupes() {
        let idem = Idempotency::load(None);
        assert!(!idem.contains("txn_1"));
        idem.mark("txn_1");
        assert!(idem.contains("txn_1"));
        assert!(!idem.contains("txn_2"));
    }

    #[test]
    fn idempotency_persists_across_reload() {
        let dir = std::env::temp_dir().join(format!("envyou-idem-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("processed.json");
        {
            let idem = Idempotency::load(Some(path.clone()));
            idem.mark("txn_persist");
        }
        // A fresh store loading the same file must still see the id.
        let reloaded = Idempotency::load(Some(path.clone()));
        assert!(reloaded.contains("txn_persist"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
