//! The main window at a small size keeps its fixed parts on screen, and every
//! app window has a minimum size.

use std::sync::Arc;

use domain::test_support::{FakeFactory, FakeProvider, cid, conv, member, msg, uid};
use gpui::{TestAppContext, px, size};
use parla::providers::ProviderRegistry;
use parla::root_view::RootView;
use settings::ConfigPaths;
use store::{MemoryStore, Store};

fn init(cx: &mut TestAppContext) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    cx.update(|cx| {
        parla::init_without_watcher(cx, paths);
        i18n::gpui::set_pref(cx, i18n::LocalePref::Code("en".parse().unwrap()));
    });
    dir
}

#[gpui::test]
async fn composer_stays_inside_a_short_window(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.add_conversation(conv("C1", "general"));
    for n in 2..=80 {
        provider.add_conversation(conv(&format!("C{n}"), &format!("channel-{n}")));
    }
    provider.add_member(member("u2", "Other"));
    for n in 1..=60 {
        provider.add_message(msg("C1", &format!("{n}.000000"), &format!("message {n}")));
    }
    // Many workspaces: the rail is taller than the window.
    let mut factory = FakeFactory::default().with_provider(provider);
    for n in 2..=16 {
        let p = Arc::new(FakeProvider::new(&format!("w{n}"), uid("u1")));
        p.add_conversation(conv("C1", "general"));
        factory = factory.with_provider(p);
    }
    let mut registry = ProviderRegistry::new();
    registry.add(factory);
    let store: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (view, cx) = cx.add_window_view(|window, cx| RootView::new(&registry, store, window, cx));
    cx.simulate_resize(size(px(1000.0), px(500.0)));
    cx.run_until_parked();
    let session = view.read_with(cx, |v, _| v.session().unwrap().clone());
    session.update(cx, |s, cx| s.open(cid("C1"), cx));
    for _ in 0..3 {
        cx.run_until_parked();
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }
    let frame = cx.debug_bounds("composer-frame").expect("composer drawn");
    assert!(
        frame.bottom() <= px(500.0),
        "composer bottom {:?} is below the 500px window",
        frame.bottom()
    );
    let rail = cx.debug_bounds("workspace-rail").expect("rail drawn");
    assert!(rail.bottom() <= px(500.0), "rail overflows: {:?}", rail);
}

#[test]
fn every_window_has_a_minimum_size() {
    use themes::metrics::*;
    // The main window fits rail, sidebar and a usable centre.
    assert!(LAYOUT_WINDOW_MIN_WIDTH >= LAYOUT_RAIL + LAYOUT_SIDEBAR + LAYOUT_CENTER_MIN);
    assert!(LAYOUT_WINDOW_MIN_WIDTH < LAYOUT_WINDOW_WIDTH);
    assert!(LAYOUT_WINDOW_MIN_HEIGHT < LAYOUT_WINDOW_HEIGHT);
    assert!(LAYOUT_SIGN_IN_MIN_WIDTH <= LAYOUT_SIGN_IN_WIDTH);
    assert!(LAYOUT_SIGN_IN_MIN_HEIGHT <= LAYOUT_SIGN_IN_HEIGHT);
    assert!(LAYOUT_WINDOW_ABOUT_MIN_HEIGHT <= LAYOUT_WINDOW_ABOUT_HEIGHT);
    assert!(LAYOUT_WINDOW_POPOUT_MIN_WIDTH < LAYOUT_WINDOW_POPOUT_WIDTH);
    assert!(LAYOUT_WINDOW_AUDIT_MIN_WIDTH < LAYOUT_WINDOW_AUDIT_WIDTH);
    // The Settings window fits its navigation and a usable page.
    assert!(LAYOUT_WINDOW_SETTINGS_MIN_WIDTH < LAYOUT_WINDOW_SETTINGS_WIDTH);
    assert!(LAYOUT_WINDOW_SETTINGS_MIN_HEIGHT < LAYOUT_WINDOW_SETTINGS_HEIGHT);
    assert!(LAYOUT_WINDOW_SETTINGS_MIN_WIDTH >= LAYOUT_SETTINGS_NAV + LAYOUT_SETTINGS_LABEL);
}

#[test]
fn window_options_carry_the_minimum_size() {
    use themes::metrics::*;
    let main = parla::window_options::main(gpui::Bounds::default());
    assert_eq!(
        main.window_min_size,
        Some(size(LAYOUT_WINDOW_MIN_WIDTH, LAYOUT_WINDOW_MIN_HEIGHT))
    );
    let sign_in = parla::window_options::sign_in(gpui::Bounds::default());
    assert_eq!(
        sign_in.window_min_size,
        Some(size(LAYOUT_SIGN_IN_MIN_WIDTH, LAYOUT_SIGN_IN_MIN_HEIGHT))
    );
    let about = parla::window_options::about(gpui::Bounds::default());
    assert!(about.window_min_size.is_some());
    let settings_window = parla::window_options::settings(gpui::Bounds::default());
    assert_eq!(
        settings_window.window_min_size,
        Some(size(
            LAYOUT_WINDOW_SETTINGS_MIN_WIDTH,
            LAYOUT_WINDOW_SETTINGS_MIN_HEIGHT
        ))
    );
    let audit = parla::window_options::audit_log(gpui::Bounds::default());
    assert!(audit.window_min_size.is_some());
    let thread = parla::window_options::popout(gpui::Bounds::default(), true);
    assert!(thread.window_min_size.is_some());
    let conversation = parla::window_options::popout(gpui::Bounds::default(), false);
    assert!(conversation.window_min_size.is_some());
}
