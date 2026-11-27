use i18n::{Arg, LocalePref, LocaleSet, LocalizedText, Localizer, chain};
use unic_langid::LanguageIdentifier;

i18n::namespace!("demo", folder = "tests/fixtures/demo/i18n");

fn id(s: &str) -> LanguageIdentifier {
    s.parse().unwrap()
}

fn loc(codes: &[&str]) -> Localizer {
    Localizer::new(codes.iter().map(|c| id(c)).collect(), &[&CATALOG]).unwrap()
}

fn os(tags: &[&str]) -> Vec<String> {
    tags.iter().map(|s| s.to_string()).collect()
}

#[test]
fn falls_back_per_message() {
    let ca_en = loc(&["ca", "en"]);
    assert_eq!(t!(ca_en, "sidebar-only-en-es"), "Only in the source");
    assert_eq!(
        t!(ca_en, "sidebar-only-en"),
        "Only in the source and Spanish"
    );
    let ca_es_en = loc(&["ca", "es", "en"]);
    assert_eq!(
        t!(ca_es_en, "sidebar-only-en"),
        "Solo en el origen y en español"
    );
    assert_eq!(t!(ca_es_en, "sidebar-unreads"), "No llegits");
}

#[test]
fn chain_from_os_list() {
    let set = LocaleSet::embedded();
    let c = |tags: &[&str]| chain(&LocalePref::System, &os(tags), set);
    assert_eq!(c(&["ca-ES", "es-ES"]), vec![id("ca"), id("es"), id("en")]);
    assert_eq!(c(&["fr-FR"]), vec![id("en")]);
    assert_eq!(c(&["es-MX"]), vec![id("es"), id("en")]);
    assert_eq!(c(&[]), vec![id("en")]);
}

#[test]
fn setting_overrides_os() {
    let set = LocaleSet::embedded();
    let c = chain(&LocalePref::Code(id("ca")), &os(&["es-ES"]), set);
    assert_eq!(c, vec![id("ca"), id("en")]);
    let c = chain(&LocalePref::Code(id("en-XA")), &os(&["es-ES"]), set);
    assert_eq!(c, vec![id("en-XA"), id("en")]);
}

#[test]
fn plurals_per_locale() {
    for (code, one, other) in [
        ("en", "1 reply", "2 replies"),
        ("es", "1 respuesta", "2 respuestas"),
        ("ca", "1 resposta", "2 respostes"),
    ] {
        let l = loc(&[code, "en"]);
        assert_eq!(t!(l, "thread-replies", count = 1), one);
        assert_eq!(t!(l, "thread-replies", count = 2), other);
    }
}

#[test]
fn no_isolation_marks() {
    let l = loc(&["es", "en"]);
    let s = t!(l, "greeting", name = "Ana");
    assert_eq!(s, "Hola, Ana");
    assert!(!s.contains(['\u{2068}', '\u{2069}']));
}

#[test]
fn pseudo_is_longer_and_accented() {
    let en = loc(&["en"]);
    let xa = loc(&["en-XA", "en"]);
    for id in ["sidebar-unreads", "sidebar-only-en-es", "sidebar-only-en"] {
        let a = en.format("demo", id, None).into_owned();
        let b = xa.format("demo", id, None).into_owned();
        assert!(
            b.chars().count() * 10 >= a.chars().count() * 13,
            "{id}: {a:?} -> {b:?}"
        );
        assert!(!b.chars().any(|c| c.is_ascii_alphabetic()), "{id}: {b:?}");
    }
    let s = t!(xa, "greeting", name = "123");
    assert!(s.ends_with("123]") || s.contains("123"), "{s}");
}

#[test]
fn missing_everywhere_returns_id() {
    let l = loc(&["ca", "en"]);
    assert_eq!(l.format("demo", "nope", None), "demo.nope");
    assert_eq!(l.format("other", "nope", None), "other.nope");
}

#[test]
fn localized_text_roundtrip() {
    let l = loc(&["ca", "en"]);
    let t = LocalizedText::new("demo", "thread-replies").with("count", 2);
    assert_eq!(t.args[0].1, Arg::Num(2.0));
    assert_eq!(l.text(&t), "2 respostes");
    assert_eq!(l.text(&t.clone()), l.text(&t));
    assert_eq!(l.locale(), &id("ca"));
}
