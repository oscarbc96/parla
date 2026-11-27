//! The `fake` message catalog of [`FakeFactory`](super::FakeFactory): stands in
//! for a provider's catalog in core tests.

i18n::namespace!("fake");

/// The fake provider's catalog (namespace `fake`).
pub fn fake_catalog() -> &'static i18n::Catalog {
    &CATALOG
}
