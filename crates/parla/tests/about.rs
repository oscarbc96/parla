//! About box shows required links and notices.

mod support;

use domain::test_support::fake_catalog;
use i18n::{LocalePref, Localizer};

fn localizer(code: &str) -> Localizer {
    support::localizer(code, &[parla::catalog(), fake_catalog()])
}

#[test]
fn about_links_privacy_terms_licences_source_security() {
    let catalogs = [fake_catalog()];
    let text = parla::about::about_text(&localizer("en"), &catalogs, "1.2.3");
    assert!(text.contains("Privacy"), "{text}");
    assert!(text.contains("Terms"), "{text}");
    assert!(text.contains("Third-party licences"), "{text}");
    assert!(text.contains("Source code"), "{text}");
    assert!(text.contains("Report a security issue"), "{text}");
    assert!(
        text.contains("https://oscarbc96.github.io/parla/docs/privacy/"),
        "{text}"
    );
    assert!(
        text.contains("https://oscarbc96.github.io/parla/docs/terms/"),
        "{text}"
    );
    assert!(
        text.contains("https://github.com/oscarbc96/parla"),
        "{text}"
    );
    assert!(text.contains("SECURITY.md"), "{text}");
    assert!(
        text.contains("This is the fake provider used in tests."),
        "{text}"
    );
}

fn open_about(
    cx: &mut gpui::TestAppContext,
) -> (
    gpui::Entity<parla::about::About>,
    &mut gpui::VisualTestContext,
) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        i18n::gpui::init(
            cx,
            &[components::catalog(), parla::catalog()],
            LocalePref::Code("en".parse().unwrap()),
        );
    });
    let (view, vcx) = cx.add_window_view(|_, _| parla::about::view(Vec::new(), "1.2.3"));
    vcx.run_until_parked();
    (view, vcx)
}

fn expected(vcx: &mut gpui::VisualTestContext) -> String {
    vcx.update(|_, cx| parla::about::about_text(&i18n::gpui::localizer(cx), &[], "1.2.3"))
}

#[gpui::test]
fn clicking_copy_version_info_copies_the_text_and_says_copied(cx: &mut gpui::TestAppContext) {
    let (view, vcx) = open_about(cx);
    assert!(vcx.debug_bounds("about-copy").is_some());
    assert!(vcx.read_from_clipboard().is_none());
    let b = vcx.debug_bounds("about-copy").unwrap();
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    vcx.run_until_parked();
    let want = expected(vcx);
    assert!(want.contains("Parla 1.2.3"), "{want}");
    assert_eq!(vcx.read_from_clipboard().and_then(|c| c.text()), Some(want));
    assert!(view.read_with(vcx, |v, _| v.copied()), "label says Copied");
}

#[gpui::test]
fn enter_on_the_focused_copy_button_copies_and_says_copied(cx: &mut gpui::TestAppContext) {
    let (view, vcx) = open_about(cx);
    view.update_in(vcx, |v, w, cx| v.focus_copy(w, cx));
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    let want = expected(vcx);
    assert_eq!(vcx.read_from_clipboard().and_then(|c| c.text()), Some(want));
    assert!(view.read_with(vcx, |v, _| v.copied()), "label says Copied");
}
