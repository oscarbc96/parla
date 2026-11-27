use i18n::{Arg, HourCycle, LocalizedText, Localizer};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use unic_langid::LanguageIdentifier;

i18n::namespace!("demo", folder = "tests/fixtures/demo/i18n");

fn id(s: &str) -> LanguageIdentifier {
    s.parse().unwrap()
}

fn loc(code: &str) -> Localizer {
    let mut chain = vec![id(code)];
    if code != "en" {
        chain.push(id("en"));
    }
    Localizer::new(chain, &[&CATALOG]).unwrap()
}

/// ICU uses narrow no-break spaces; compare with plain spaces.
fn plain(s: String) -> String {
    s.replace(['\u{202f}', '\u{a0}'], " ")
}

fn instant() -> Timestamp {
    "2026-11-13T14:05:09Z".parse().unwrap()
}

#[test]
fn numbers_group_per_locale() {
    assert_eq!(loc("en").number(12345), "12,345");
    assert_eq!(loc("es").number(12345), "12.345");
    assert_eq!(loc("ca").number(12345), "12.345");
}

#[test]
fn fluent_numbers_use_icu() {
    let t = |l: &Localizer| {
        l.text(&LocalizedText::new("demo", "thread-replies").with("count", Arg::from(12345)))
            .into_owned()
    };
    assert_eq!(t(&loc("en")), "12,345 replies");
    assert_eq!(t(&loc("es")), "12.345 respuestas");
}

#[test]
fn fluent_number_function_formats_per_locale() {
    let t = |l: &Localizer| {
        l.text(&LocalizedText::new("demo", "size-kb").with("value", Arg::from(1234.5)))
            .into_owned()
    };
    assert_eq!(t(&loc("en")), "1,234.5 KB");
    assert_eq!(t(&loc("es")), "1234,5 KB");
    let one = l_text(&loc("en"), 1.0);
    assert_eq!(one, "1.0 KB");
}

fn l_text(l: &Localizer, v: f64) -> String {
    l.text(&LocalizedText::new("demo", "size-kb").with("value", Arg::from(v)))
        .into_owned()
}

#[test]
fn clock_overrides_locale() {
    let utc = TimeZone::UTC;
    let t = |l: &Localizer, hc| plain(l.time(instant(), &utc, hc));
    assert_eq!(t(&loc("en"), HourCycle::Locale), "2:05 PM");
    assert_eq!(t(&loc("en"), HourCycle::H24), "14:05");
    assert_eq!(t(&loc("es"), HourCycle::Locale), "14:05");
    assert!(t(&loc("es"), HourCycle::H12).starts_with("2:05"));
    assert_eq!(t(&loc("ca"), HourCycle::Locale), "14:05");
}

#[test]
fn time_uses_the_zone() {
    let tz = TimeZone::get("Europe/Madrid").unwrap();
    assert_eq!(loc("es").time(instant(), &tz, HourCycle::H24), "15:05");
}

#[test]
fn relative_buckets() {
    let now = instant();
    let ago = |secs: i64| now - jiff::SignedDuration::from_secs(secs);
    let cases: [(&str, [&str; 5]); 3] = [
        (
            "en",
            [
                "just now",
                "just now",
                "5 minutes ago",
                "3 hours ago",
                "2 days ago",
            ],
        ),
        (
            "es",
            [
                "ahora mismo",
                "ahora mismo",
                "hace 5 minutos",
                "hace 3 horas",
                "hace 2 días",
            ],
        ),
        (
            "ca",
            [
                "ara mateix",
                "ara mateix",
                "fa 5 minuts",
                "fa 3 hores",
                "fa 2 dies",
            ],
        ),
    ];
    for (code, want) in cases {
        let l = loc(code);
        let got: Vec<String> = [0, 59, 300, 3 * 3600, 2 * 86400]
            .iter()
            .map(|s| l.relative(ago(*s), now))
            .collect();
        assert_eq!(got, want, "{code}");
    }
    assert_eq!(loc("en").relative(ago(60), now), "1 minute ago");
    assert_eq!(loc("en").relative(now, ago(10)), "just now");
}

#[test]
fn dates_snapshot() {
    let utc = TimeZone::UTC;
    let mut out = String::new();
    for code in ["en", "es", "ca"] {
        let l = loc(code);
        out.push_str(&format!(
            "{code}: date={} time={} date_time={}\n",
            plain(l.date(instant(), &utc)),
            plain(l.time(instant(), &utc, HourCycle::Locale)),
            plain(l.date_time(instant(), &utc, HourCycle::Locale)),
        ));
    }
    insta::assert_snapshot!(out);
}
