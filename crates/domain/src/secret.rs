//! A secret string that is wiped on drop and never printed.

use std::fmt;

use zeroize::Zeroize;

/// Secret text (a session cookie value, for example). Zeroized on drop,
/// `Debug` is redacted, and there is no `Clone`, `Display` or `Serialize`.
pub struct SecretText(Box<str>);

impl SecretText {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into().into_boxed_str())
    }

    /// The secret itself; callers must not log it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("SecretText(<redacted>)")
    }
}

impl Drop for SecretText {
    fn drop(&mut self) {
        // `Box<str>` has no `Zeroize` impl; wipe the bytes in place.
        let mut bytes = std::mem::take(&mut self.0).into_boxed_bytes().into_vec();
        bytes.zeroize();
    }
}
