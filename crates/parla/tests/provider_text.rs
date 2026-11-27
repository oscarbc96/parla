//! Provider text reaches the window through the provider's own catalog.

mod support;

use domain::ProviderError;
use domain::test_support::{FakeFactory, fake_catalog};
use i18n::{LocalePref, LocalizedText, Localizer};
use parla::providers::ProviderRegistry;

fn localizer(code: &str) -> Localizer {
    support::localizer(
        code,
        &[components::catalog(), parla::catalog(), fake_catalog()],
    )
}

fn shown(l: &Localizer, e: &ProviderError) -> String {
    l.text(&parla::error_text::provider_error(e)).into_owned()
}

#[test]
fn provider_error_uses_provider_catalog() {
    let quota =
        ProviderError::Localized(LocalizedText::new("fake", "error-quota").with("limit", 5));
    assert_eq!(
        shown(&localizer("en"), &quota),
        "The fake workspace is over its quota of 5 messages."
    );
    assert_eq!(
        shown(&localizer("es"), &quota),
        "El espacio de trabajo falso supera su cuota de 5 mensajes."
    );
    assert_eq!(
        shown(&localizer("ca"), &quota),
        "L'espai de treball fals supera la seva quota de 5 missatges."
    );
}

#[test]
fn neutral_provider_errors_use_the_app_catalog() {
    let (en, ca) = (localizer("en"), localizer("ca"));
    // English keeps the wording of the old Display strings word for word.
    for e in [
        ProviderError::SessionExpired,
        ProviderError::RateLimited,
        ProviderError::NotAllowed("nope".into()),
        ProviderError::Network("down".into()),
        ProviderError::Other("odd".into()),
    ] {
        assert_eq!(shown(&en, &e), e.to_string());
    }
    assert_eq!(shown(&ca, &ProviderError::RateLimited), "massa peticions");
    assert_eq!(
        shown(&ca, &ProviderError::Network("down".into())),
        "error de xarxa: down"
    );
}

#[test]
fn about_shows_each_provider_notice() {
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default());
    let catalogs = registry.catalogs();
    assert_eq!(catalogs.len(), 1);
    let en = parla::about::about_text(&localizer("en"), &catalogs, "1.2.3");
    assert!(en.contains("Parla "), "{en}");
    assert!(
        en.contains("This is the fake provider used in tests."),
        "{en}"
    );
    let ca = parla::about::about_text(&localizer("ca"), &catalogs, "1.2.3");
    assert!(ca.contains("Aquest és el proveïdor fals"), "{ca}");
    assert!(ca.contains("Llicència"), "{ca}");
}

#[gpui::test]
fn registry_registers_provider_catalogs_with_the_localizer(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        i18n::gpui::init(
            cx,
            &[parla::catalog()],
            LocalePref::Code("ca".parse().unwrap()),
        );
        let mut registry = ProviderRegistry::new();
        registry.add(FakeFactory::default());
        registry.register_catalogs(cx);
        // Twice is harmless.
        registry.register_catalogs(cx);
        let l = i18n::gpui::localizer(cx);
        assert_eq!(
            l.format("fake", "unofficial-notice", None),
            "Aquest és el proveïdor fals que s'usa a les proves."
        );
    });
}
