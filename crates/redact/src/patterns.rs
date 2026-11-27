//! Provider-neutral patterns shared by the redactor and the leak scanner.

use std::sync::LazyLock;

use regex::Regex;

fn re(pattern: &str) -> Regex {
    Regex::new(pattern).expect("static pattern compiles")
}

/// `Bearer <credential>` header values.
pub static BEARER: LazyLock<Regex> =
    LazyLock::new(|| re(r"(?i)\bbearer\s+[A-Za-z0-9._~+/=%-]{8,}"));

pub static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| re(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9-]+(?:\.[A-Za-z0-9-]+)*\.[A-Za-z]{2,}"));

/// An ID candidate counts as real only with a digit after its first character, which
/// skips plain upper-case words. Synthetic IDs (`T0000001`) have a 7-digit body and
/// are expected not to match the provider's ID shape at all.
pub fn is_real_id(candidate: &str) -> bool {
    candidate.chars().skip(1).any(|c| c.is_ascii_digit())
}

/// Email domains that mark an already-synthetic address.
pub fn is_fake_email(email: &str) -> bool {
    let domain = email.rsplit('@').next().unwrap_or("").to_ascii_lowercase();
    matches!(
        domain.as_str(),
        "example.test" | "example.com" | "example.org"
    )
}

pub const SECRET_PLACEHOLDER: &str = "REDACTED";

/// An absolute `http(s)` URL; group 1 is the authority (`user@host:port`).
pub static URL: LazyLock<Regex> =
    LazyLock::new(|| re(r#"(?i)\bhttps?://([^\s/?#"'`<>\\)\]|]+)[^\s"'`<>\\)\]|]*"#));

/// `"key": "value"` on one line of JSON; group 1 the key, group 2 the (escaped) value.
pub static TEXT_FIELD: LazyLock<Regex> =
    LazyLock::new(|| re(r#""([A-Za-z_]+)"\s*:\s*"((?:[^"\\]|\\.)*)""#));

#[cfg(test)]
mod tests {
    use super::is_real_id;

    #[test]
    fn is_real_id_never_panics_on_odd_candidates() {
        assert!(!is_real_id(""));
        assert!(!is_real_id("\u{e9}ABC"));
        assert!(is_real_id("\u{e9}A1"));
        assert!(is_real_id("T0123"));
        assert!(!is_real_id("TEAM"));
    }
}
