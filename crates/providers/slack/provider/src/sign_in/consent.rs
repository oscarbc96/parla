//! Consent notice shown before sign-in.

use components::sign_in::SignInText;

use super::text;

/// The consent statements, shown before either capture method.
pub fn notice() -> Vec<SignInText> {
    [
        "signin-notice-unofficial",
        "signin-notice-session",
        "signin-notice-token",
        "signin-notice-employer",
        "signin-notice-breakage",
        "signin-notice-privacy",
        "signin-notice-terms",
        "signin-notice-age",
    ]
    .map(text)
    .to_vec()
}
