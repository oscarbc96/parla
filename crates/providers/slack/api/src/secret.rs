//! Secret newtypes. Values never reach `Debug`, `Display` or serialization.

use std::fmt;

use zeroize::Zeroizing;

/// A secret string, zeroized on drop. Prints as `Secret(REDACTED)`; not `Serialize`.
pub struct Secret(Zeroizing<String>);

impl Secret {
    pub fn new(value: String) -> Secret {
        Secret(Zeroizing::new(value))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }

    /// An explicit copy, itself zeroized on drop. Secrets are deliberately not `Clone`.
    fn duplicate(&self) -> Secret {
        Secret::new(self.0.to_string())
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(REDACTED)")
    }
}

impl fmt::Display for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(REDACTED)")
    }
}

/// Why a credential string was rejected. Messages never contain the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CredentialError {
    #[error("token must start with xoxc-")]
    NotXoxc,
    #[error("cookie must contain a d value starting with xoxd-")]
    NotXoxd,
    #[error("credential is empty")]
    Empty,
}

/// A browser session token (`xoxc-...`).
#[derive(Debug)]
pub struct XoxcToken(Secret);

impl XoxcToken {
    pub fn parse(s: &str) -> Result<XoxcToken, CredentialError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(CredentialError::Empty);
        }
        if !s.starts_with("xoxc-") || !s.is_ascii() || s.chars().any(char::is_whitespace) {
            return Err(CredentialError::NotXoxc);
        }
        Ok(XoxcToken(Secret::new(s.to_string())))
    }

    pub fn expose(&self) -> &str {
        self.0.expose()
    }

    /// An explicit copy; the secret stays zeroized on drop.
    pub fn duplicate(&self) -> XoxcToken {
        XoxcToken(self.0.duplicate())
    }
}

/// The `d` session cookie (URL-encoded form) and optional `d-s`.
#[derive(Debug)]
pub struct SessionCookie {
    d: Secret,
    d_s: Option<Secret>,
}

impl SessionCookie {
    /// Accepts `xoxd-...`, `d=xoxd-...` or a full `Cookie:` line containing `d=`.
    pub fn parse(s: &str) -> Result<SessionCookie, CredentialError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(CredentialError::Empty);
        }
        let s = match s.get(..7) {
            Some(p) if p.eq_ignore_ascii_case("cookie:") => s[7..].trim(),
            _ => s,
        };
        let mut d: Option<&str> = None;
        let mut d_s: Option<&str> = None;
        for (i, part) in s.split(';').enumerate() {
            let part = part.trim();
            if i == 0 && part.starts_with("xoxd-") {
                d = Some(part);
                continue;
            }
            match part.split_once('=') {
                Some(("d", v)) => d = Some(v.trim()),
                Some(("d-s", v)) => d_s = Some(v.trim()),
                _ => {}
            }
        }
        let d = d
            .filter(|v| v.starts_with("xoxd-") && v.is_ascii())
            .ok_or(CredentialError::NotXoxd)?;
        Ok(SessionCookie {
            d: Secret::new(encode_cookie(d)),
            d_s: d_s
                .filter(|v| !v.is_empty())
                .map(|v| Secret::new(v.to_string())),
        })
    }

    /// An explicit copy; the secrets stay zeroized on drop.
    pub fn duplicate(&self) -> SessionCookie {
        SessionCookie {
            d: self.d.duplicate(),
            d_s: self.d_s.as_ref().map(Secret::duplicate),
        }
    }

    /// `d=<encoded>` or `d=<encoded>; d-s=<value>`.
    pub fn header_value(&self) -> String {
        match &self.d_s {
            Some(ds) => format!("d={}; d-s={}", self.d.expose(), ds.expose()),
            None => format!("d={}", self.d.expose()),
        }
    }

    /// The `d` cookie value (URL-encoded form).
    pub fn d_value(&self) -> &str {
        self.d.expose()
    }

    /// The `d-s` cookie value, when the session holds one.
    pub fn d_s_value(&self) -> Option<&str> {
        self.d_s.as_ref().map(|s| s.expose())
    }
}

fn has_percent_escape(s: &str) -> bool {
    s.as_bytes()
        .windows(3)
        .any(|w| w[0] == b'%' && w[1].is_ascii_hexdigit() && w[2].is_ascii_hexdigit())
}

fn encode_cookie(s: &str) -> String {
    if has_percent_escape(s) {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 8);
    for c in s.chars() {
        match c {
            '/' => out.push_str("%2F"),
            '+' => out.push_str("%2B"),
            '=' => out.push_str("%3D"),
            c => out.push(c),
        }
    }
    out
}
