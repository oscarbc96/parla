//! Every action a keymap can bind has a title in a registered catalog, in every locale.

use std::collections::BTreeSet;
use std::path::Path;

mod support;

use i18n::{LocaleSet, Localizer};
use settings::parse_keymap;

fn localizer(code: &str) -> Localizer {
    support::localizer(code, &[components::catalog(), parla::catalog()])
}

fn keymap_actions() -> BTreeSet<String> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/keymaps");
    let mut out = BTreeSet::new();
    for file in ["default-macos", "default-windows", "default-linux"] {
        let path = dir.join(format!("{file}.json"));
        let text = std::fs::read_to_string(&path).unwrap();
        for block in parse_keymap(&path, &text).unwrap() {
            out.extend(block.bindings.into_values());
        }
    }
    out
}

#[test]
fn every_action_has_a_title() {
    let mut names = keymap_actions();
    names.extend(parla::actions::KNOWN.iter().map(|s| s.to_string()));
    assert!(names.len() > 25, "{names:?}");
    for locale in LocaleSet::embedded()
        .locales
        .iter()
        .map(|l| l.code.to_string())
    {
        let l = localizer(&locale);
        for name in &names {
            let title = parla::actions::title(&l, name)
                .unwrap_or_else(|| panic!("{locale}: no title for {name}"));
            assert!(!title.is_empty(), "{locale}: empty title for {name}");
        }
    }
}

#[test]
fn titles_follow_the_active_language() {
    assert_eq!(
        parla::actions::title(&localizer("en"), "app::OpenSettings").as_deref(),
        Some("Open settings")
    );
    assert_eq!(
        parla::actions::title(&localizer("ca"), "app::OpenSettings").as_deref(),
        Some("Obre els paràmetres")
    );
    assert_eq!(parla::actions::title(&localizer("en"), "app::Nope"), None);
}

#[test]
fn known_actions_include_every_default_binding() {
    for name in keymap_actions() {
        assert!(
            parla::actions::is_known(&name),
            "{name} is bound but unknown"
        );
    }
}
