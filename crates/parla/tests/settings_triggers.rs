//! The on-screen ways into Settings open the Settings window, like the menu
//! bar: the rail's gear and the workspace menu's Settings entry, clicked
//! through the rendered main window. No network.

use std::sync::Arc;

use domain::test_support::{FakeFactory, FakeProvider, conv, member, msg, uid};
use gpui::{TestAppContext, VisualTestContext};
use parla::providers::ProviderRegistry;
use parla::root_view::{Phase, RootView};
use parla::settings_window::SettingsView;
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

/// A ready main window, active and drawn.
fn main_window(cx: &mut TestAppContext) -> &mut VisualTestContext {
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.set_workspace_name("Example Workspace");
    provider.add_conversation(conv("C1", "general"));
    provider.add_member(member("u2", "Other"));
    provider.add_message(msg("C1", "1.000000", "hi"));
    let mut registry = ProviderRegistry::new();
    registry.add(FakeFactory::default().with_provider(provider));
    let store: Arc<dyn Store> = Arc::new(MemoryStore::default());
    let (view, vcx) = cx.add_window_view(|window, cx| RootView::new(&registry, store, window, cx));
    vcx.run_until_parked();
    view.read_with(vcx, |v, _| assert_eq!(*v.phase(), Phase::Ready));
    vcx.update(|window, _| window.activate_window());
    draw(vcx);
    vcx
}

fn draw(vcx: &mut VisualTestContext) {
    vcx.run_until_parked();
    vcx.update(|window, cx| window.draw(cx).clear(cx));
}

fn click(vcx: &mut VisualTestContext, selector: &'static str) {
    let b = vcx
        .debug_bounds(selector)
        .unwrap_or_else(|| panic!("{selector} drawn"));
    vcx.simulate_click(b.center(), gpui::Modifiers::none());
    vcx.run_until_parked();
}

fn settings_windows(vcx: &mut VisualTestContext) -> usize {
    vcx.update(|_, cx| {
        cx.windows()
            .into_iter()
            .filter(|w| w.downcast::<SettingsView>().is_some())
            .count()
    })
}

#[gpui::test]
fn the_rail_gear_opens_the_settings_window(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let vcx = main_window(cx);
    assert_eq!(settings_windows(vcx), 0);
    click(vcx, "rail-settings");
    assert_eq!(settings_windows(vcx), 1, "the gear opens Settings");
}

#[gpui::test]
fn the_workspace_menu_settings_entry_opens_the_settings_window(cx: &mut TestAppContext) {
    let _dir = init(cx);
    let vcx = main_window(cx);
    assert_eq!(settings_windows(vcx), 0);
    click(vcx, "sidebar-workspace-name");
    draw(vcx);
    // Settings sits just above Sign out, the last entry.
    let sel =
        |ix: usize| -> &'static str { Box::leak(format!("notify-extra-{ix}").into_boxed_str()) };
    let last = (0..32)
        .filter(|ix| vcx.debug_bounds(sel(*ix)).is_some())
        .max()
        .expect("workspace menu entries drawn");
    click(vcx, sel(last - 1));
    assert_eq!(settings_windows(vcx), 1, "the menu entry opens Settings");
}
