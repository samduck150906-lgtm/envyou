//! envyou Paddle Billing webhook.
//!
//! Flow: Paddle sends `transaction.completed` -> we verify the HMAC signature ->
//! map the purchased price to a plan -> look up the buyer's email -> mint an
//! Ed25519-signed license with envyou-core (same format the app verifies) ->
//! email it via Resend.
//!
//! ALL secrets come from environment variables — nothing is baked into the
//! binary or the repo. See README.md for the full list and deploy steps.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use envyou_core::core::license::{issue_license, LicenseClaims, PRODUCT};
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;
use std::time::{SystemTime, UNIX_EPOCH};

type HmacSha256 = Hmac<Sha256>;

/// Runtime configuration, all from env.
struct Config {
    webhook_secret: String,   // PADDLE_WEBHOOK_SECRET (the destination's secret key)
    signing_seed: [u8; 32],   // ENVYOU_SIGNING_KEY_B64 (base64 of the 32-byte private key)
    paddle_api_key: String,   // PADDLE_API_KEY (server-side, to look up customer email)
    paddle_api_base: String,  // PADDLE_API_BASE (prod default; sandbox: https://sandbox-api.paddle.com)
    resend_api_key: String,   // RESEND_API_KEY
    email_from: String,       // EMAIL_FROM, e.g. "envyou <licenses@envyou.dev>"
    price_lifetime: String,   // PRICE_LIFETIME (pri_...)
    price_annual: String,     // PRICE_ANNUAL (pri_...)
}

fn require(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| {
        eprintln!("FATAL: missing required env var {name}");
        std::process::exit(1);
    })
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

fn load_config() -> Config {
    let signing_seed_b64 = require("ENVYOU_SIGNING_KEY_B64");
    let seed_bytes = B64
        .decode(signing_seed_b64.trim())
        .unwrap_or_else(|_| {
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
        price_annual: env_or("PRICE_ANNUAL", String::new().as_str()),
    }
}

fn main() {
    let cfg = load_config();
    let port = env_or("PORT", "8080");
    let addr = format!("0.0.0.0:{port}");
    let server = tiny_http::Server::http(addr.as_str()).unwrap_or_else(|e| {
        eprintln!("FATAL: could not bind {addr}: {e}");
        std::process::exit(1);
    });
    eprintln!("paddle-webhook listening on {addr}");

    for mut request in server.incoming_requests() {
        let (status, body) = route(&mut request, &cfg);
        let response = tiny_http::Response::from_string(body)
            .with_status_code(tiny_http::StatusCode(status));
        let _ = request.respond(response);
    }
}

fn route(request: &mut tiny_http::Request, cfg: &Config) -> (u16, String) {
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

    let mut body = String::new();
    if request.as_reader().read_to_string(&mut body).is_err() {
        return (400, "could not read body".into());
    }

    let signature = match signature {
        Some(s) => s,
        None => return (400, "missing Paddle-Signature".into()),
    };
    if !verify_signature(&cfg.webhook_secret, &signature, &body) {
        eprintln!("rejected: bad Paddle signature");
        return (401, "invalid signature".into());
    }

    let event: Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(_) => return (400, "invalid json".into()),
    };

    // Only act on completed transactions; acknowledge everything else so Paddle
    // does not retry.
    let event_type = event.get("event_type").and_then(Value::as_str).unwrap_or("");
    if event_type != "transaction.completed" {
        return (200, format!("ignored event {event_type}"));
    }

    match handle_transaction(cfg, &event) {
        Ok(msg) => (200, msg),
        Err(e) => {
            // 200 so Paddle doesn't hammer retries on a permanent error; the log
            // is where you notice and re-issue manually if needed.
            eprintln!("handling error: {e}");
            (200, format!("handled with error: {e}"))
        }
    }
}

fn handle_transaction(cfg: &Config, event: &Value) -> Result<String, String> {
    let data = event.get("data").ok_or("event has no data")?;

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
        // Annual: valid ~1 year + a short grace window.
        ("pro".to_string(), Some(add_days_iso(372)))
    };

    let email = customer_email(cfg, data)?;

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

    let token = issue_license(&cfg.signing_seed, &claims).map_err(|e| e.to_string())?;
    send_license_email(cfg, &email, &token)?;

    eprintln!("issued license to {email}");
    Ok(format!("license issued to {email}"))
}

/// Look up the buyer's email. Prefer an email already present on the event;
/// otherwise fetch the customer via the Paddle API.
fn customer_email(cfg: &Config, data: &Value) -> Result<String, String> {
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
        .ok_or("transaction has no customer_id")?;

    let url = format!("{}/customers/{}", cfg.paddle_api_base, customer_id);
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {}", cfg.paddle_api_key))
        .call()
        .map_err(|e| format!("paddle customer lookup failed: {e}"))?;
    let body: Value = resp
        .into_json()
        .map_err(|e| format!("paddle customer response parse failed: {e}"))?;
    body.get("data")
        .and_then(|d| d.get("email"))
        .and_then(Value::as_str)
        .map(|s| s.to_string())
        .ok_or_else(|| "customer has no email".to_string())
}

fn send_license_email(cfg: &Config, to: &str, token: &str) -> Result<(), String> {
    let text = format!(
        "Thanks for buying envyou Pro!\n\n\
         Your license key:\n\n{token}\n\n\
         To activate: open envyou, click \"Upgrade to Pro\", and paste the key above.\n\
         Keep this email — it is your proof of purchase.\n",
    );
    let resp = ureq::post("https://api.resend.com/emails")
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
            Err(format!("resend returned {code}: {detail}"))
        }
        Err(e) => Err(format!("resend request failed: {e}")),
    }
}

/// Verify a Paddle Billing `Paddle-Signature` header of the form
/// `ts=<unix>;h1=<hex hmac>`. The signed payload is `"<ts>:<raw body>"`,
/// HMAC-SHA256 with the destination's secret key.
fn verify_signature(secret: &str, header: &str, body: &str) -> bool {
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
