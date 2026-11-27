//! Shared helpers of the components tests.
//!
//! Every test target that declares `mod common` compiles all of it, so a
//! helper only some targets use is allowed to sit unused in the others.

use i18n::{LocalePref, LocaleSet, Localizer, chain};

/// A localizer for `code` (falling back to English), with the components catalog.
#[allow(dead_code)]
pub fn localizer(code: &str) -> Localizer {
    let pref = LocalePref::Code(code.parse().expect("language tag"));
    Localizer::new(
        chain(&pref, &[], LocaleSet::embedded()),
        &[components::catalog()],
    )
    .expect("catalogs build")
}

#[allow(dead_code)]
pub fn en() -> Localizer {
    localizer("en")
}
