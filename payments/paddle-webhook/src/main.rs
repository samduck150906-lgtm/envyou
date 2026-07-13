//! envyou Paddle Billing webhook + license issuer.
//!
//! Flow: Paddle sends `transaction.completed` -> we verify the HMAC signature
//! (and reject stale timestamps) -> map the price to a plan -> look up the
//! buyer's email -> generate a short human-friendly license code -> mint an
//! Ed25519-signed **certificate** (the app verifies this offline) -> store both
//! in Supabase -> email the short code to the buyer.
//!
//! Activation itself is NOT handled here: the desktop app calls the Supabase
//! `activate_license` RPC directly (anon key), which validates the code + email,
//! enforces the activation limit, and returns the stored certificate.
//!
//! Idempotency & reliability:
//! * `paddle_transaction_id` is UNIQUE in the DB, so retries/duplicate
//!   deliveries never mint a second license — a repeat just re-emails the code.
//! * A transient failure (Paddle/Resend/Supabase 5xx, timeout, network) returns
//!   HTTP 503 so Paddle retries; a paid license is never silently dropped.
//!
//! ALL secrets come from environment variables — nothing is baked into the
//! binary or the repo. See README.md.
//!
//! Admin CLI (same binary):
//!   paddle-webhook lookup-license    <email>
//!   paddle-webhook resend-license    <email>
//!   paddle-webhook reset-activations <license-code>
//!   paddle-webhook revoke-license    <license-code>
//!   paddle-webhook reactivate-license <license-code>

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use envyou_core::core::license::{
    generate_license_code, issue_license, normalize_email, sha256_hex, LicenseClaims, PRODUCT,
};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

/// Hard cap on the request body we read before authenticating it.
const MAX_BODY_BYTES: usize = 256 * 1024;
/// Generous max age for a Paddle-Signature timestamp (idempotency is the real
/// replay defense; this only bounds ancient captured replays).
const DEFAULT_MAX_SIGNATURE_AGE_SECS: u64 = 5 * 24 * 60 * 60;
/// Retries if a freshly generated code happens to collide (astronomically rare).
const MAX_CODE_RETRIES: u32 = 6;
/// Default activation limit per license.
const MAX_ACTIVATIONS: i64 = 3;

struct Config {
    webhook_secret: String,
    signing_seed: [u8; 32],
    paddle_api_key: String,
    paddle_api_base: String,
    resend_api_key: String,
    email_from: String,
    price_lifetime: String,
    price_annual: String,
    max_sig_age: u64,
    supabase_url: String,
    supabase_service_key: String,
}

/// A required secret. Fails fast if missing or empty.
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
        price_annual: env_or("PRICE_ANNUAL", ""),
        max_sig_age: env_or(
            "PADDLE_MAX_SIGNATURE_AGE_SECS",
            &DEFAULT_MAX_SIGNATURE_AGE_SECS.to_string(),
        )
        .parse()
        .unwrap_or(DEFAULT_MAX_SIGNATURE_AGE_SECS),
        supabase_url: require("SUPABASE_URL").trim_end_matches('/').to_string(),
        supabase_service_key: require("SUPABASE_SERVICE_ROLE_KEY"),
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
        ureq::Error::Status(code, _) => *code >= 500 || *code == 429 || *code == 408,
        ureq::Error::Transport(_) => true,
    }
}

fn http_agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
}

// ---- Supabase (PostgREST) helpers, service-role ------------------------------

fn sb_get(agent: &ureq::Agent, cfg: &Config, path_and_query: &str) -> Result<Vec<Value>, HErr> {
    let url = format!("{}/rest/v1/{}", cfg.supabase_url, path_and_query);
    let resp = agent
        .get(&url)
        .set("apikey", &cfg.supabase_service_key)
        .set(
            "Authorization",
            &format!("Bearer {}", cfg.supabase_service_key),
        )
        .call()
        .map_err(|e| HErr {
            transient: ureq_transient(&e),
            msg: format!("supabase GET failed: {e}"),
        })?;
    let body: Value = resp
        .into_json()
        .map_err(|e| HErr::permanent(format!("supabase GET parse failed: {e}")))?;
    Ok(body.as_array().cloned().unwrap_or_default())
}

/// Outcome of an INSERT into `licenses`.
enum Insert {
    Created,
    ConflictTxn,  // paddle_transaction_id already present (duplicate delivery / race)
    ConflictCode, // license_code collision — regenerate and retry
}

fn sb_insert_license(agent: &ureq::Agent, cfg: &Config, row: &Value) -> Result<Insert, HErr> {
    let url = format!("{}/rest/v1/licenses", cfg.supabase_url);
    let resp = agent
        .post(&url)
        .set("apikey", &cfg.supabase_service_key)
        .set(
            "Authorization",
            &format!("Bearer {}", cfg.supabase_service_key),
        )
        .set("Content-Type", "application/json")
        .set("Prefer", "return=minimal")
        .send_json(row.clone());
    match resp {
        Ok(_) => Ok(Insert::Created),
        Err(ureq::Error::Status(409, _)) => {
            // Distinguish which unique constraint tripped without parsing the
            // error body: if the txn now exists it was the txn constraint.
            let txn = row
                .get("paddle_transaction_id")
                .and_then(Value::as_str)
                .unwrap_or("");
            let existing = sb_get(
                agent,
                cfg,
                &format!("licenses?paddle_transaction_id=eq.{}&select=id", enc(txn)),
            )?;
            if existing.is_empty() {
                Ok(Insert::ConflictCode)
            } else {
                Ok(Insert::ConflictTxn)
            }
        }
        Err(e) => Err(HErr {
            transient: ureq_transient(&e),
            msg: format!("supabase INSERT failed: {e}"),
        }),
    }
}

fn sb_get_by_txn(agent: &ureq::Agent, cfg: &Config, txn: &str) -> Result<Option<Value>, HErr> {
    let rows = sb_get(
        agent,
        cfg,
        &format!("licenses?paddle_transaction_id=eq.{}&select=*", enc(txn)),
    )?;
    Ok(rows.into_iter().next())
}

fn sb_revoke_by_txn(agent: &ureq::Agent, cfg: &Config, txn: &str) -> Result<(), HErr> {
    let url = format!(
        "{}/rest/v1/licenses?paddle_transaction_id=eq.{}",
        cfg.supabase_url,
        enc(txn)
    );
    agent
        .request("PATCH", &url)
        .set("apikey", &cfg.supabase_service_key)
        .set(
            "Authorization",
            &format!("Bearer {}", cfg.supabase_service_key),
        )
        .set("Content-Type", "application/json")
        .set("Prefer", "return=minimal")
        .send_json(json!({ "status": "revoked" }))
        .map_err(|e| HErr {
            transient: ureq_transient(&e),
            msg: format!("supabase PATCH (revoke) failed: {e}"),
        })?;
    Ok(())
}

/// Percent-encode a value for a PostgREST query string / URL.
fn enc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

// ---- Issuance ----------------------------------------------------------------

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("lookup-license") => cli(|cfg, agent| cli_lookup(cfg, agent, args.get(1))),
        Some("resend-license") => cli(|cfg, agent| cli_resend(cfg, agent, args.get(1))),
        Some("reset-activations") => cli(|cfg, agent| cli_reset(cfg, agent, args.get(1))),
        Some("revoke-license") => {
            cli(|cfg, agent| cli_set_status(cfg, agent, args.get(1), "revoked"))
        }
        Some("reactivate-license") => {
            cli(|cfg, agent| cli_set_status(cfg, agent, args.get(1), "active"))
        }
        _ => run_server(),
    }
}

fn run_server() {
    let cfg = load_config();
    let agent = http_agent();
    let port = env_or("PORT", "8080");
    let addr = format!("0.0.0.0:{port}");
    let server = tiny_http::Server::http(addr.as_str()).unwrap_or_else(|e| {
        eprintln!("FATAL: could not bind {addr}: {e}");
        std::process::exit(1);
    });
    eprintln!(
        "paddle-webhook listening on {addr} (PRICE_LIFETIME={lifetime} PRICE_ANNUAL={annual})",
        lifetime = cfg.price_lifetime,
        annual = if cfg.price_annual.is_empty() {
            "(unset)"
        } else {
            cfg.price_annual.as_str()
        },
    );

    for mut request in server.incoming_requests() {
        let (status, body) = route(&mut request, &cfg, &agent);
        let response =
            tiny_http::Response::from_string(body).with_status_code(tiny_http::StatusCode(status));
        let _ = request.respond(response);
    }
}

fn route(request: &mut tiny_http::Request, cfg: &Config, agent: &ureq::Agent) -> (u16, String) {
    let method = request.method().as_str().to_string();
    let url = request.url().to_string();

    if url == "/health" || url == "/" {
        return (200, "ok".into());
    }
    if method != "POST" || !url.starts_with("/webhook/paddle") {
        return (404, "not found".into());
    }

    let signature = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Paddle-Signature"))
        .map(|h| h.value.as_str().to_string());

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
    let event_type = event
        .get("event_type")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();

    let result = match event_type.as_str() {
        "transaction.completed" => handle_transaction(cfg, agent, &event),
        // Paddle Billing reports refunds and chargebacks as `adjustment.created`
        // (action: "refund" | "chargeback" | "credit") referencing the original
        // transaction. Revoking `status` blocks *new* activations immediately;
        // devices that already activated keep their offline-verified certificate
        // until the app re-checks online (see docs/LICENSE_SYSTEM.md).
        "adjustment.created" => handle_adjustment(cfg, agent, &event),
        _ => return (200, format!("ignored event {event_type}")),
    };

    match result {
        Ok(msg) => (200, msg),
        Err(e) if e.transient => {
            eprintln!("transient error, asking Paddle to retry: {}", e.msg);
            (503, "temporary error; please retry".into())
        }
        Err(e) => {
            eprintln!("ALERT permanent handling error (will NOT retry): {}", e.msg);
            (200, "handled with permanent error".into())
        }
    }
}

fn handle_transaction(cfg: &Config, agent: &ureq::Agent, event: &Value) -> Result<String, HErr> {
    let data = event
        .get("data")
        .ok_or_else(|| HErr::permanent("event has no data"))?;

    let txn = data
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| HErr::permanent("transaction has no id"))?
        .to_string();

    // Idempotency: if this transaction already issued a license, just re-send it.
    if let Some(existing) = sb_get_by_txn(agent, cfg, &txn)? {
        let code = existing
            .get("license_code")
            .and_then(Value::as_str)
            .unwrap_or("");
        let email = existing.get("email").and_then(Value::as_str).unwrap_or("");
        if !code.is_empty() && !email.is_empty() {
            send_license_email(cfg, agent, email, code)?;
        }
        return Ok(format!("transaction {txn} already issued; re-sent"));
    }

    // Plan from the purchased price ids.
    let seen_prices: Vec<&str> = data
        .get("items")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|item| {
                    item.get("price")
                        .and_then(|p| p.get("id"))
                        .and_then(Value::as_str)
                })
                .collect()
        })
        .unwrap_or_default();
    let mut is_lifetime = false;
    let mut is_annual = false;
    for &pid in &seen_prices {
        if pid == cfg.price_lifetime {
            is_lifetime = true;
        } else if !cfg.price_annual.is_empty() && pid == cfg.price_annual {
            is_annual = true;
        }
    }
    if !is_lifetime && !is_annual {
        // A completed purchase that matches none of our configured prices is
        // almost always a config drift: the checkout's price id changed but the
        // webhook's PRICE_LIFETIME / PRICE_ANNUAL env vars weren't updated to
        // match. That silently drops paid licenses, so log LOUDLY with the ids —
        // this is the first thing to check when "I paid but got no code/email".
        // Still return 200 so Paddle doesn't retry a pure config error forever.
        eprintln!(
            "ALERT no configured price matched transaction {txn}: saw {seen:?}, expected \
             PRICE_LIFETIME={lifetime} PRICE_ANNUAL={annual}. Fix the webhook env to match the \
             checkout price id, then re-issue this buyer with `paddle-webhook resend-license`.",
            seen = seen_prices,
            lifetime = cfg.price_lifetime,
            annual = if cfg.price_annual.is_empty() {
                "(unset)"
            } else {
                cfg.price_annual.as_str()
            },
        );
        return Ok("no envyou Pro price in this transaction; ignored".into());
    }
    let (tier, plan, expires_at) = if is_lifetime {
        ("pro_lifetime", "pro-lifetime", None)
    } else {
        ("pro_annual", "pro", Some(add_days_iso(372)))
    };

    let email = customer_email(cfg, agent, data)?;
    let norm_email = normalize_email(&email);
    let customer_id = data.get("customer_id").and_then(Value::as_str);

    // Generate a code + certificate and insert; retry on the (rare) code clash.
    let mut attempt = 0;
    let code = loop {
        attempt += 1;
        if attempt > MAX_CODE_RETRIES {
            return Err(HErr::permanent("could not generate a unique license code"));
        }
        let id = Uuid::new_v4().to_string();
        let code = generate_license_code();
        let claims = LicenseClaims {
            product: PRODUCT.to_string(),
            plan: plan.to_string(),
            hardware_id: None,
            issued_at: now_iso8601(),
            expires_at: expires_at.clone(),
            features: vec![
                "unlimited_projects".to_string(),
                "unlimited_variables".to_string(),
                "mcp".to_string(),
                "custom_environment_colors".to_string(),
                "lifetime_updates".to_string(),
            ],
            license_id: Some(id.clone()),
            email_hash: Some(sha256_hex(&norm_email)),
            code_hash: Some(sha256_hex(&code)),
            schema_version: Some(2),
        };
        let cert = issue_license(&cfg.signing_seed, &claims)
            .map_err(|e| HErr::permanent(format!("issue certificate failed: {e}")))?;

        let row = json!({
            "id": id,
            "license_code": code,
            "email": email,
            "normalized_email": norm_email,
            "product": PRODUCT,
            "tier": tier,
            "paddle_transaction_id": txn,
            "paddle_customer_id": customer_id,
            "status": "active",
            "max_activations": MAX_ACTIVATIONS,
            "signed_certificate": cert,
        });

        match sb_insert_license(agent, cfg, &row)? {
            Insert::Created => break code,
            Insert::ConflictCode => continue, // regenerate
            Insert::ConflictTxn => {
                // Raced with another delivery of the same transaction — re-send
                // whatever is stored and stop.
                if let Some(existing) = sb_get_by_txn(agent, cfg, &txn)? {
                    let c = existing
                        .get("license_code")
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    if !c.is_empty() {
                        send_license_email(cfg, agent, &email, c)?;
                    }
                }
                return Ok(format!("transaction {txn} already issued (raced); re-sent"));
            }
        }
    };

    send_license_email(cfg, agent, &email, &code)?;
    eprintln!("issued license {code} to {email}");
    Ok(format!("license issued to {email}"))
}

/// Refund or chargeback on a transaction we issued a license for: flip
/// `status` to `revoked` so `activate_license` refuses further activations.
/// A `credit` adjustment (partial goodwill credit, not a full refund) is left
/// alone.
fn handle_adjustment(cfg: &Config, agent: &ureq::Agent, event: &Value) -> Result<String, HErr> {
    let data = event
        .get("data")
        .ok_or_else(|| HErr::permanent("adjustment event has no data"))?;

    let action = data.get("action").and_then(Value::as_str).unwrap_or("");
    if action != "refund" && action != "chargeback" {
        return Ok(format!("adjustment action {action} does not revoke a license; ignored"));
    }

    let txn = data
        .get("transaction_id")
        .and_then(Value::as_str)
        .ok_or_else(|| HErr::permanent("adjustment has no transaction_id"))?;

    let Some(existing) = sb_get_by_txn(agent, cfg, txn)? else {
        // No license was ever issued for this transaction (e.g. it never
        // matched a configured price) — nothing to revoke.
        return Ok(format!("no license for transaction {txn}; nothing to revoke"));
    };
    let code = existing
        .get("license_code")
        .and_then(Value::as_str)
        .unwrap_or("");
    let status = existing.get("status").and_then(Value::as_str).unwrap_or("");
    if status == "revoked" {
        return Ok(format!("license {code} already revoked"));
    }

    sb_revoke_by_txn(agent, cfg, txn)?;
    eprintln!("revoked license {code} for transaction {txn} ({action})");
    Ok(format!("license {code} revoked ({action})"))
}

/// Coarse but sane email-shape check: exactly one `@`, non-empty local and
/// domain parts, no whitespace, and a `.` in the domain. Not a full RFC 5322
/// validator (Resend rejects genuinely malformed addresses anyway) — this
/// just stops obvious junk like `"foo@"` or `"@bar"` from slipping past the
/// old `.contains('@')` check.
fn looks_like_email(s: &str) -> bool {
    if s.chars().any(|c| c.is_whitespace()) {
        return false;
    }
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && !domain.is_empty()
        && !domain.contains('@')
        && domain.contains('.')
        && !domain.starts_with('.')
}

/// Look up the buyer's email. Prefer the `license_email` the buyer typed on the
/// landing page (carried in `custom_data`) so the code goes exactly where they
/// expect, then the inline customer object, then the Paddle API.
fn customer_email(cfg: &Config, agent: &ureq::Agent, data: &Value) -> Result<String, HErr> {
    let inline = data
        .get("customer")
        .and_then(|c| c.get("email"))
        .and_then(Value::as_str);
    if let Some(license_email) = data
        .get("custom_data")
        .and_then(|c| c.get("license_email"))
        .and_then(Value::as_str)
        .filter(|e| looks_like_email(e))
    {
        if inline.is_some_and(|c| !c.eq_ignore_ascii_case(license_email)) {
            eprintln!("note: custom_data.license_email differs from Paddle customer email; using custom_data");
        }
        return Ok(license_email.to_string());
    }
    if let Some(email) = inline {
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

fn send_license_email(cfg: &Config, agent: &ureq::Agent, to: &str, code: &str) -> Result<(), HErr> {
    let deep_link = format!("envyou://activate?email={}&code={}", enc(to), enc(code));
    let text = format!(
        "Thanks for purchasing envyou Pro!\n\n\
         License email:\n{to}\n\n\
         License code:\n{code}\n\n\
         How to activate:\n\
         1. Download and open envyou:  https://envyou.dev/#download\n\
         2. Click \"Activate Pro\".\n\
         3. Enter the license email and license code above.\n\n\
         One-click activation (if envyou is installed):\n{deep_link}\n\n\
         Keep this email — it is your proof of purchase.\n\
         Need help? Contact ceo@eternalsix.com\n\n\
         — — —\n\
         envyou Pro 평생 라이선스를 구매해주셔서 감사합니다.\n\
         라이선스 이메일: {to}\n\
         라이선스 코드: {code}\n\
         활성화: envyou를 다운로드해 실행하고 \"Activate Pro\"에서 위 이메일과 코드를 입력하세요.\n",
    );
    let html = format!(
        "<div style=\"font-family:system-ui,Segoe UI,Arial,sans-serif;max-width:520px;margin:auto;color:#111\">\
         <h2>Thanks for purchasing envyou Pro!</h2>\
         <p>Your lifetime license is ready.</p>\
         <p style=\"margin:4px 0\"><b>License email</b><br>{to}</p>\
         <p style=\"margin:4px 0\"><b>License code</b><br>\
           <span style=\"font-size:20px;font-family:ui-monospace,Menlo,Consolas,monospace;letter-spacing:1px;background:#f2f2f2;padding:6px 10px;border-radius:6px;display:inline-block\">{code}</span></p>\
         <p style=\"margin:18px 0\">\
           <a href=\"https://envyou.dev/#download\" style=\"background:#000080;color:#fff;text-decoration:none;padding:10px 16px;border-radius:6px;display:inline-block\">Download envyou</a>\
           &nbsp;\
           <a href=\"{deep_link}\" style=\"background:#007000;color:#fff;text-decoration:none;padding:10px 16px;border-radius:6px;display:inline-block\">Activate envyou Pro</a>\
         </p>\
         <p style=\"color:#555;font-size:13px\">If the Activate button does not work, open envyou, click <b>Activate Pro</b>, and paste your license email and code.</p>\
         <p style=\"color:#555;font-size:13px\">Need help? <a href=\"mailto:ceo@eternalsix.com\">ceo@eternalsix.com</a></p>\
         </div>",
    );
    let resp = agent
        .post("https://api.resend.com/emails")
        .set("Authorization", &format!("Bearer {}", cfg.resend_api_key))
        .send_json(json!({
            "from": cfg.email_from,
            "to": [to],
            "subject": "Your envyou Pro license",
            "text": text,
            "html": html,
        }));
    match resp {
        Ok(_) => Ok(()),
        Err(ureq::Error::Status(code_status, r)) => {
            let detail = r.into_string().unwrap_or_default();
            let transient = code_status >= 500 || code_status == 429 || code_status == 408;
            Err(HErr {
                transient,
                msg: format!("resend returned {code_status}: {detail}"),
            })
        }
        Err(e) => Err(HErr::transient(format!("resend request failed: {e}"))),
    }
}

// ---- Admin CLI ---------------------------------------------------------------

fn cli(f: impl FnOnce(&Config, &ureq::Agent) -> Result<(), String>) {
    let cfg = load_config();
    let agent = http_agent();
    if let Err(e) = f(&cfg, &agent) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

fn cli_lookup(cfg: &Config, agent: &ureq::Agent, email: Option<&String>) -> Result<(), String> {
    let email = email.ok_or("usage: lookup-license <email>")?;
    let norm = normalize_email(email);
    let rows = sb_get(
        agent,
        cfg,
        &format!(
            "licenses?normalized_email=eq.{}&select=license_code,tier,status,activation_count,max_activations,created_at",
            enc(&norm)
        ),
    )
    .map_err(|e| e.msg)?;
    if rows.is_empty() {
        println!("no licenses for {email}");
        return Ok(());
    }
    for r in rows {
        println!(
            "{}  tier={}  status={}  activations={}/{}  created={}",
            r.get("license_code").and_then(Value::as_str).unwrap_or("?"),
            r.get("tier").and_then(Value::as_str).unwrap_or("?"),
            r.get("status").and_then(Value::as_str).unwrap_or("?"),
            r.get("activation_count")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            r.get("max_activations")
                .and_then(Value::as_i64)
                .unwrap_or(0),
            r.get("created_at").and_then(Value::as_str).unwrap_or("?"),
        );
    }
    Ok(())
}

fn cli_resend(cfg: &Config, agent: &ureq::Agent, email: Option<&String>) -> Result<(), String> {
    let email = email.ok_or("usage: resend-license <email>")?;
    let norm = normalize_email(email);
    let rows = sb_get(
        agent,
        cfg,
        &format!(
            "licenses?normalized_email=eq.{}&select=email,license_code",
            enc(&norm)
        ),
    )
    .map_err(|e| e.msg)?;
    if rows.is_empty() {
        return Err(format!("no licenses for {email}"));
    }
    for r in rows {
        let to = r.get("email").and_then(Value::as_str).unwrap_or(email);
        let code = r.get("license_code").and_then(Value::as_str).unwrap_or("");
        send_license_email(cfg, agent, to, code).map_err(|e| e.msg)?;
        println!("re-sent {code} to {to}");
    }
    Ok(())
}

fn cli_reset(cfg: &Config, agent: &ureq::Agent, code: Option<&String>) -> Result<(), String> {
    let code = code.ok_or("usage: reset-activations <license-code>")?;
    let url = format!(
        "{}/rest/v1/licenses?license_code=eq.{}",
        cfg.supabase_url,
        enc(code)
    );
    agent
        .request("PATCH", &url)
        .set("apikey", &cfg.supabase_service_key)
        .set(
            "Authorization",
            &format!("Bearer {}", cfg.supabase_service_key),
        )
        .set("Content-Type", "application/json")
        .set("Prefer", "return=minimal")
        .send_json(json!({ "activation_count": 0 }))
        .map_err(|e| format!("reset failed: {e}"))?;
    println!("reset activations for {code}");
    Ok(())
}

fn cli_set_status(
    cfg: &Config,
    agent: &ureq::Agent,
    code: Option<&String>,
    status: &str,
) -> Result<(), String> {
    let code = code.ok_or_else(|| format!("usage: {status}-license <license-code>"))?;
    let url = format!(
        "{}/rest/v1/licenses?license_code=eq.{}",
        cfg.supabase_url,
        enc(code)
    );
    agent
        .request("PATCH", &url)
        .set("apikey", &cfg.supabase_service_key)
        .set(
            "Authorization",
            &format!("Bearer {}", cfg.supabase_service_key),
        )
        .set("Content-Type", "application/json")
        .set("Prefer", "return=minimal")
        .send_json(json!({ "status": status }))
        .map_err(|e| format!("set-status failed: {e}"))?;
    println!("license {code} status set to {status}");
    Ok(())
}

// ---- Paddle signature --------------------------------------------------------

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

// ---- time helpers (std only) -------------------------------------------------

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
    fn enc_percent_encodes_reserved() {
        assert_eq!(enc("a b@c.com"), "a%20b%40c.com");
        assert_eq!(enc("ENVY-K7M4-9Q2P"), "ENVY-K7M4-9Q2P");
        assert_eq!(enc("x+y/z"), "x%2By%2Fz");
    }

    #[test]
    fn looks_like_email_accepts_sane_addresses() {
        assert!(looks_like_email("buyer@example.com"));
        assert!(looks_like_email("a.b+tag@sub.example.co"));
    }

    #[test]
    fn looks_like_email_rejects_junk() {
        assert!(!looks_like_email("foo@"));
        assert!(!looks_like_email("@bar.com"));
        assert!(!looks_like_email("no-at-sign"));
        assert!(!looks_like_email("two@@at.com"));
        assert!(!looks_like_email("has space@example.com"));
        assert!(!looks_like_email("foo@bar")); // no dot in domain
        assert!(!looks_like_email("foo@.com")); // domain starts with dot
    }
}
