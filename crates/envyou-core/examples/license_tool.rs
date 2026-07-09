//! Offline license tool for envyou maintainers.
//!
//! Build/run with the `issuer` feature (the shipped app never enables it):
//!
//! ```bash
//! # 1. Generate a signing keypair. Writes the PRIVATE key to a file (keep it
//! #    secret, never commit it) and prints the PUBLIC key to paste into
//! #    crates/envyou-core/src/core/license.rs -> LICENSE_PUBLIC_KEY_B64.
//! cargo run -p envyou-core --features issuer --example license_tool -- \
//!     keygen envyou-signing.key
//!
//! # 2. Print the public key (from a file, a base64 seed, or $ENVYOU_SIGNING_KEY_B64).
//! cargo run -p envyou-core --features issuer --example license_tool -- \
//!     pubkey envyou-signing.key
//!
//! # 2b. Prove the shipped app will accept licenses signed by your seed:
//! #     compares the seed's public key to the build's LICENSE_PUBLIC_KEY_B64
//! #     and exits non-zero on mismatch (run it where the secret lives).
//! ENVYOU_SIGNING_KEY_B64=... cargo run -p envyou-core --features issuer \
//!     --example license_tool -- checkkey
//!
//! # 3. Mint a license token for a buyer.
//! cargo run -p envyou-core --features issuer --example license_tool -- \
//!     issue envyou-signing.key --plan pro \
//!     --hardware-id <machine-id> \
//!     --expires 2027-07-06T00:00:00Z \
//!     --features unlimited_projects,unlimited_variables
//! ```
//!
//! ⚠️ The private key gates all paid licenses — store it in a secret manager or
//! hardware token, and NEVER commit it to this repository.

#[cfg(not(feature = "issuer"))]
fn main() {
    eprintln!("license_tool requires the `issuer` feature:");
    eprintln!("  cargo run -p envyou-core --features issuer --example license_tool -- <cmd>");
    std::process::exit(2);
}

#[cfg(feature = "issuer")]
fn main() {
    if let Err(e) = issuer::run() {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

#[cfg(feature = "issuer")]
mod issuer {
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    use ed25519_dalek::SigningKey;
    use envyou_core::core::license::{
        is_license_key_configured, issue_license, LicenseClaims, LICENSE_PUBLIC_KEY_B64, PRODUCT,
    };
    use rand::RngCore;
    use std::time::{SystemTime, UNIX_EPOCH};

    pub fn run() -> Result<(), String> {
        let args: Vec<String> = std::env::args().skip(1).collect();
        match args.first().map(String::as_str) {
            Some("keygen") => keygen(
                args.get(1)
                    .map(String::as_str)
                    .unwrap_or("envyou-signing.key"),
            ),
            Some("pubkey") => pubkey(args.get(1).map(String::as_str)),
            Some("checkkey") => checkkey(args.get(1).map(String::as_str)),
            Some("issue") => issue(&args[1..]),
            _ => {
                eprintln!(
                    "commands:\n  \
                     keygen [out-file]                 generate a signing keypair\n  \
                     pubkey [key-file | b64-seed]      print the public key (else $ENVYOU_SIGNING_KEY_B64)\n  \
                     checkkey [key-file | b64-seed]    verify the seed matches this build's LICENSE_PUBLIC_KEY_B64\n  \
                     issue <key-file> [opts]           mint a license token"
                );
                Err("unknown or missing command".into())
            }
        }
    }

    /// Resolve a signing key from (in priority order): an explicit key file, an
    /// explicit base64 seed argument, or the `ENVYOU_SIGNING_KEY_B64` env var —
    /// the exact same secret the Paddle webhook signs with.
    fn resolve_signing_key(arg: Option<&str>) -> Result<SigningKey, String> {
        if let Some(a) = arg {
            if std::path::Path::new(a).exists() {
                return load_key(a);
            }
            return signing_key_from_b64(a);
        }
        match std::env::var("ENVYOU_SIGNING_KEY_B64") {
            Ok(v) => signing_key_from_b64(&v),
            Err(_) => {
                Err("provide a key file, a base64 seed, or set ENVYOU_SIGNING_KEY_B64".to_string())
            }
        }
    }

    fn signing_key_from_b64(b64: &str) -> Result<SigningKey, String> {
        let bytes = B64
            .decode(b64.trim())
            .map_err(|_| "seed is not valid base64".to_string())?;
        let seed: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| "seed must decode to exactly 32 bytes".to_string())?;
        Ok(SigningKey::from_bytes(&seed))
    }

    /// B1 gate: derive the public key from the signing seed and check it against
    /// the `LICENSE_PUBLIC_KEY_B64` compiled into this build (the same source the
    /// released app is built from). Exits non-zero on mismatch so it can gate a
    /// release in CI. Run it where the webhook's secret lives:
    ///   ENVYOU_SIGNING_KEY_B64=... cargo run -p envyou-core --features issuer \
    ///       --example license_tool -- checkkey
    fn checkkey(arg: Option<&str>) -> Result<(), String> {
        let sk = resolve_signing_key(arg)?;
        let derived = B64.encode(sk.verifying_key().to_bytes());
        let embedded = LICENSE_PUBLIC_KEY_B64.trim();
        println!("seed-derived public key       : {derived}");
        println!("build LICENSE_PUBLIC_KEY_B64   : {embedded}");
        println!(
            "build key configured (not placeholder): {}",
            is_license_key_configured()
        );
        if !is_license_key_configured() {
            return Err(
                "this build still ships the placeholder public key — paste your production key \
                 into crates/envyou-core/src/core/license.rs (LICENSE_PUBLIC_KEY_B64) and rebuild"
                    .into(),
            );
        }
        if derived == embedded {
            println!("MATCH ✓  the released app will accept licenses signed by this seed");
            Ok(())
        } else {
            Err(
                "MISMATCH ✗  this seed does NOT match the build's public key; every license it \
                 signs would be rejected by the app"
                    .into(),
            )
        }
    }

    fn keygen(out: &str) -> Result<(), String> {
        if std::path::Path::new(out).exists() {
            return Err(format!("refusing to overwrite existing key file: {out}"));
        }
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        let sk = SigningKey::from_bytes(&seed);

        std::fs::write(out, B64.encode(seed)).map_err(|e| e.to_string())?;
        harden_permissions(out);

        println!("Private key written to: {out}  (keep secret — never commit)");
        println!();
        println!("Paste this PUBLIC key into LICENSE_PUBLIC_KEY_B64:");
        println!("{}", B64.encode(sk.verifying_key().to_bytes()));
        Ok(())
    }

    fn pubkey(arg: Option<&str>) -> Result<(), String> {
        let sk = resolve_signing_key(arg)?;
        println!("{}", B64.encode(sk.verifying_key().to_bytes()));
        Ok(())
    }

    fn issue(args: &[String]) -> Result<(), String> {
        let key_file = args.first().ok_or("usage: issue <key-file> [opts]")?;
        let sk = load_key(key_file)?;
        let seed = sk.to_bytes();

        let mut plan = "pro".to_string();
        let mut hardware_id: Option<String> = None;
        let mut expires_at: Option<String> = None;
        let mut features: Vec<String> = Vec::new();

        let mut i = 1;
        while i < args.len() {
            let val = || {
                args.get(i + 1)
                    .cloned()
                    .ok_or(format!("missing value for {}", args[i]))
            };
            match args[i].as_str() {
                "--plan" => plan = val()?,
                "--hardware-id" => hardware_id = Some(val()?),
                "--expires" => expires_at = Some(val()?),
                "--features" => {
                    features = val()?
                        .split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                }
                other => return Err(format!("unknown option: {other}")),
            }
            i += 2;
        }

        let claims = LicenseClaims {
            product: PRODUCT.to_string(),
            plan,
            hardware_id,
            issued_at: now_iso8601(),
            expires_at,
            features,
            ..Default::default()
        };
        let token = issue_license(&seed, &claims).map_err(|e| e.to_string())?;
        println!("{token}");
        Ok(())
    }

    fn load_key(path: &str) -> Result<SigningKey, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("reading {path}: {e}"))?;
        let bytes = B64
            .decode(raw.trim())
            .map_err(|_| "key file is not valid base64".to_string())?;
        let seed: [u8; 32] = bytes
            .as_slice()
            .try_into()
            .map_err(|_| "key file must decode to 32 bytes".to_string())?;
        Ok(SigningKey::from_bytes(&seed))
    }

    #[cfg(unix)]
    fn harden_permissions(path: &str) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    fn harden_permissions(_path: &str) {}

    fn now_iso8601() -> String {
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let days = (secs / 86_400) as i64;
        let rem = secs % 86_400;
        let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
        let z = days + 719_468;
        let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
        let doe = (z - era * 146_097) as u64;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let y = yoe as i64 + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let mon = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
        let year = if mon <= 2 { y + 1 } else { y };
        format!("{year:04}-{mon:02}-{d:02}T{h:02}:{m:02}:{s:02}Z")
    }
}
