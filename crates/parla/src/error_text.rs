//! Provider errors as text in the active language.

use domain::ProviderError;
use i18n::LocalizedText;

/// The message for `e`: the provider's own text when it brought one, else the
/// app catalog's wording for the neutral variants.
pub fn provider_error(e: &ProviderError) -> LocalizedText {
    match e {
        ProviderError::Localized(t) => t.clone(),
        ProviderError::SessionExpired => LocalizedText::new("parla", "error-session-expired"),
        ProviderError::RateLimited => LocalizedText::new("parla", "error-rate-limited"),
        ProviderError::NotAllowed(d) => {
            LocalizedText::new("parla", "error-not-allowed").with("detail", d.as_str())
        }
        ProviderError::Network(d) => {
            LocalizedText::new("parla", "error-network").with("detail", d.as_str())
        }
        ProviderError::Unsupported => LocalizedText::new("parla", "error-unsupported"),
        ProviderError::TooLarge => LocalizedText::new("parla", "error-too-large"),
        ProviderError::Other(d) => {
            LocalizedText::new("parla", "error-detail").with("detail", d.as_str())
        }
    }
}
