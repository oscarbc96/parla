use std::path::{Path, PathBuf};

use i18n::LocaleSet;
use i18n::check::{check_crate, check_workspace};

fn case(name: &str) -> Vec<String> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/catalogs")
        .join(name);
    let locales =
        LocaleSet::parse(&std::fs::read_to_string(dir.join("locales.toml")).unwrap()).unwrap();
    check_crate(&dir, &locales)
}

#[test]
fn ok_case_is_clean() {
    assert_eq!(case("ok"), Vec::<String>::new());
}

#[test]
fn missing_id() {
    assert_eq!(case("missing-id"), ["i18n/es.ftl: missing message \"b\""]);
}

#[test]
fn orphan_id() {
    assert_eq!(
        case("orphan-id"),
        ["i18n/es.ftl: message \"c\" is not in en.ftl"]
    );
}

#[test]
fn orphan_locale_file() {
    assert_eq!(
        case("orphan-locale-file"),
        ["i18n/fr.ftl: locale is not listed in locales.toml"]
    );
}

#[test]
fn missing_locale_file() {
    assert_eq!(
        case("missing-locale-file"),
        ["i18n/es.ftl: file is missing"]
    );
}

#[test]
fn unknown_variable() {
    assert_eq!(
        case("unknown-variable"),
        ["i18n/es.ftl: message \"a\" uses unknown variable \"$who\""]
    );
}

#[test]
fn parse_error() {
    let errs = case("parse-error");
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert!(
        errs[0].starts_with("i18n/es.ftl: parse error at line 1: "),
        "{errs:?}"
    );
}

#[test]
fn missing_attribute() {
    assert_eq!(
        case("missing-attribute"),
        ["i18n/es.ftl: message \"a\" is missing attribute \".title\""]
    );
}

#[test]
fn unknown_t_id() {
    assert_eq!(
        case("unknown-t-id"),
        ["src/lib.rs: t! id \"nope\" is not in en.ftl"]
    );
}

#[test]
fn unused_id() {
    assert_eq!(
        case("unused-id"),
        ["i18n/en.ftl: message \"spare\" is never used (add it to `# unused-ok:` if intended)"]
    );
}

#[test]
fn unused_ok_silences_unused() {
    assert_eq!(case("unused-ok"), Vec::<String>::new());
}

#[test]
fn real_workspace_is_complete() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let errs = check_workspace(&root);
    assert!(errs.is_empty(), "{}", errs.join("\n"));
}
