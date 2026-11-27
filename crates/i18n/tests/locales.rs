use i18n::{LocaleSet, LocalesError, PSEUDO};

fn id(s: &str) -> unic_langid::LanguageIdentifier {
    s.parse().unwrap()
}

#[test]
fn embedded_set_is_valid() {
    let set = LocaleSet::embedded();
    assert!(set.get(&set.source).is_some(), "source is listed");
    let mut seen = std::collections::HashSet::new();
    for l in &set.locales {
        assert!(seen.insert(l.code.clone()), "duplicate {}", l.code);
        assert!(!l.name.trim().is_empty());
        assert_ne!(l.code.to_string(), PSEUDO);
    }
}

#[test]
fn source_is_en() {
    assert_eq!(LocaleSet::embedded().source, id("en"));
    assert_eq!(
        LocaleSet::embedded().get(&id("es")).unwrap().name,
        "Español"
    );
    assert_eq!(LocaleSet::embedded().get(&id("ca")).unwrap().name, "Català");
}

#[test]
fn parse_rejects_duplicates() {
    let t = "source=\"en\"\n[[locales]]\ncode=\"en\"\nname=\"A\"\n[[locales]]\ncode=\"en\"\nname=\"B\"\n";
    assert!(matches!(
        LocaleSet::parse(t),
        Err(LocalesError::Duplicate(_))
    ));
}

#[test]
fn parse_rejects_missing_source() {
    let t = "source=\"fr\"\n[[locales]]\ncode=\"en\"\nname=\"A\"\n";
    assert!(matches!(
        LocaleSet::parse(t),
        Err(LocalesError::MissingSource(_))
    ));
}

#[test]
fn parse_rejects_bad_tag() {
    let t = "source=\"en\"\n[[locales]]\ncode=\"en\"\nname=\"A\"\n[[locales]]\ncode=\"not a tag\"\nname=\"B\"\n";
    assert!(matches!(LocaleSet::parse(t), Err(LocalesError::BadTag(_))));
}

#[test]
fn parse_rejects_pseudo_and_empty_name() {
    let t = "source=\"en\"\n[[locales]]\ncode=\"en\"\nname=\"A\"\n[[locales]]\ncode=\"en-XA\"\nname=\"P\"\n";
    assert!(matches!(LocaleSet::parse(t), Err(LocalesError::Pseudo)));
    let t = "source=\"en\"\n[[locales]]\ncode=\"en\"\nname=\" \"\n";
    assert!(matches!(
        LocaleSet::parse(t),
        Err(LocalesError::EmptyName(_))
    ));
}
