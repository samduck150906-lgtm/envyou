//! Local, name-based heuristic for flagging high-risk variable names in the
//! approval UI (spec §7). This never inspects a variable's **value** — only its
//! name — and it never sends anything anywhere; it is a pure, offline hint so
//! the approval dialog can warn harder for names that usually hold credentials.

/// Substrings that, when present in a variable name, mark it as likely-sensitive.
/// Deliberately conservative and name-only.
const SENSITIVE_MARKERS: &[&str] = &[
    "SECRET",
    "PASSWORD",
    "PASSWD",
    "TOKEN",
    "PRIVATE_KEY",
    "PRIVATEKEY",
    "API_KEY",
    "APIKEY",
    "ACCESS_KEY",
    "SECRET_KEY",
    "CLIENT_SECRET",
    "SERVICE_ROLE",
    "DATABASE_URL",
    "DB_URL",
    "DSN",
    "CREDENTIAL",
    "AUTH",
    "SESSION",
    "SIGNING",
    "CERT",
    "SSN",
];

/// Whether a variable name looks like it holds a credential/secret, by a simple
/// case-insensitive substring match. Used only to strengthen the approval
/// warning — enforcement is always the human's decision plus the never-share
/// list.
pub fn is_sensitive_name(key: &str) -> bool {
    let upper = key.to_ascii_uppercase();
    SENSITIVE_MARKERS.iter().any(|m| upper.contains(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_common_credential_names() {
        for k in [
            "STRIPE_SECRET",
            "db_password",
            "GITHUB_TOKEN",
            "AWS_SECRET_ACCESS_KEY",
            "DATABASE_URL",
            "MY_PRIVATE_KEY",
            "app_client_secret",
            "SUPABASE_SERVICE_ROLE_KEY",
        ] {
            assert!(is_sensitive_name(k), "expected {k} to be flagged sensitive");
        }
    }

    #[test]
    fn does_not_flag_obviously_public_names() {
        assert!(!is_sensitive_name("NODE_ENV"));
        assert!(!is_sensitive_name("PORT"));
        assert!(!is_sensitive_name("NEXT_PUBLIC_URL"));
        assert!(!is_sensitive_name("LOG_LEVEL"));
    }

    #[test]
    fn is_case_insensitive() {
        assert!(is_sensitive_name("stripe_secret"));
        assert!(is_sensitive_name("Api_Key"));
    }
}
