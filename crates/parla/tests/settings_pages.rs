//! The Settings window's MCP, Calls, Workspaces and privacy, and Advanced
//! pages: each control writes its key, the app reads it back, and the
//! destructive actions ask first.

mod support;

use gpui::{Modifiers, TestAppContext, VisualTestContext};
use i18n::LocalePref;
use parla::settings_window::{
    AdvancedChange, CallsChange, McpChange, McpWorkspaceField, Page, PrivacyChange, SettingsView,
    StorageChange, mcp_client_config, step,
};
use settings::{ConfigPaths, JoinWith, SettingsStore, WorkspaceKey};

fn setup(cx: &mut TestAppContext) -> (tempfile::TempDir, ConfigPaths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = ConfigPaths::at(dir.path());
    let p = paths.clone();
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        i18n::gpui::init(
            cx,
            &[components::catalog(), parla::catalog()],
            LocalePref::Code("en".parse().unwrap()),
        );
        cx.set_global(SettingsStore::load(p));
    });
    (dir, paths)
}

fn open_on<'a>(
    cx: &'a mut TestAppContext,
    paths: &ConfigPaths,
    page: Page,
) -> (gpui::Entity<SettingsView>, &'a mut VisualTestContext) {
    let path = paths.settings.clone();
    let (view, vcx) =
        cx.add_window_view(move |_, _| parla::settings_window::view_on(path, "1.2.3".into(), page));
    vcx.run_until_parked();
    (view, vcx)
}

/// Reloads the store from the file, as the file watcher does after a write.
fn reload(vcx: &mut VisualTestContext) {
    vcx.update(|_, cx| {
        cx.global_mut::<SettingsStore>().reload();
    });
    vcx.run_until_parked();
}

fn click(vcx: &mut VisualTestContext, id: &str) {
    let b = vcx
        .debug_bounds(Box::leak(id.to_string().into_boxed_str()))
        .unwrap_or_else(|| panic!("{id} is drawn"));
    vcx.simulate_click(b.center(), Modifiers::none());
    vcx.run_until_parked();
}

/// The settings the app reads now (the store after the watcher's reload).
fn live(vcx: &mut VisualTestContext) -> settings::Settings {
    vcx.update(|_, cx| cx.global::<SettingsStore>().current().clone())
}

#[test]
fn page_changes_name_their_key_and_json_value() {
    let c = |path: Vec<String>, raw: String| (path.join("."), raw);
    let m = McpChange::Workspace("fake/w1".into(), McpWorkspaceField::AllowWrites, true);
    assert_eq!(
        c(m.key_path(), m.raw_json()),
        ("mcp.workspaces.fake/w1.allow_writes".into(), "true".into())
    );
    let k = CallsChange::JoinWith(JoinWith::ProviderApp);
    assert_eq!(
        c(k.key_path(), k.raw_json()),
        ("calls.join_with".into(), "\"provider_app\"".into())
    );
    let k = CallsChange::Ringtone("none".into());
    assert_eq!(
        c(k.key_path(), k.raw_json()),
        ("calls.ringtone".into(), "\"none\"".into())
    );
    let k = CallsChange::InputDevice(Some("USB Mic".into()));
    assert_eq!(
        c(k.key_path(), k.raw_json()),
        ("calls.input_device".into(), "\"USB Mic\"".into())
    );
    let k = CallsChange::OutputDevice(None);
    assert_eq!(
        c(k.key_path(), k.raw_json()),
        ("calls.output_device".into(), "null".into())
    );
    let s = StorageChange::RetentionDays(30);
    assert_eq!(
        c(s.key_path(), s.raw_json()),
        ("storage.retention_days".into(), "30".into())
    );
    let l = AdvancedChange::LogLevel(settings::LogLevel::Debug);
    assert_eq!(
        c(l.key_path(), l.raw_json()),
        ("log.level".into(), "\"debug\"".into())
    );
    let a = AdvancedChange::MessageWindow(500);
    assert_eq!(
        c(a.key_path(), a.raw_json()),
        ("performance.message_window".into(), "500".into())
    );
}

#[test]
fn steppers_walk_a_ladder_and_snap_odd_values() {
    let ladder = [8, 16, 32];
    assert_eq!(step(&ladder, 8, true), 16);
    assert_eq!(step(&ladder, 32, true), 32, "stays at the top");
    assert_eq!(step(&ladder, 8, false), 8, "stays at the bottom");
    assert_eq!(step(&ladder, 20, true), 32);
    assert_eq!(step(&ladder, 20, false), 16);
}

#[test]
fn client_config_runs_the_app_binary_with_mcp() {
    let text = mcp_client_config(std::path::Path::new("/Apps/Parla.app/Contents/MacOS/parla"));
    let v: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        v["mcpServers"]["parla"]["command"],
        "/Apps/Parla.app/Contents/MacOS/parla"
    );
    assert_eq!(v["mcpServers"]["parla"]["args"][0], "mcp");
}

#[gpui::test]
fn mcp_controls_write_their_keys_and_apply_live(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    // A workspace already in the policy shows a row even without a session.
    std::fs::write(
        &paths.settings,
        r#"{ "mcp": { "workspaces": { "fake/w1": { "enabled": false } } } }"#,
    )
    .unwrap();
    let (_view, vcx) = open_on(cx, &paths, Page::Mcp);
    reload(vcx);
    assert!(vcx.debug_bounds("settings-mcp-tool-send_message").is_some());
    click(vcx, "settings-mcp-enabled");
    click(vcx, "settings-mcp-ws-0-enabled");
    click(vcx, "settings-mcp-ws-0-writes");
    click(vcx, "settings-mcp-ws-0-confirm");
    reload(vcx);
    let s = live(vcx);
    assert!(s.mcp.enabled);
    let ws = s.mcp.workspace(&WorkspaceKey::new("fake", "w1"));
    assert!(ws.enabled && ws.allow_writes);
    assert!(!ws.confirm_writes, "the default was on");

    click(vcx, "settings-mcp-copy-config");
    let copied = vcx
        .read_from_clipboard()
        .and_then(|c| c.text())
        .unwrap_or_default();
    assert!(copied.contains("\"mcp\""), "{copied}");
}

#[gpui::test]
fn gif_controls_write_their_keys_and_apply_live(cx: &mut TestAppContext) {
    if !parla::plugins::builtin::all().iter().any(|b| b.bundled()) {
        eprintln!("skipped: no GIF plugin is bundled; run `mise run plugins:build`");
        return;
    }
    let (_dir, paths) = setup(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Plugins);
    reload(vcx);
    assert!(
        vcx.debug_bounds("settings-gif-giphy-enabled").is_some(),
        "the GIPHY row is drawn"
    );
    assert!(vcx.debug_bounds("settings-gif-giphy-key").is_some());
    assert!(vcx.debug_bounds("settings-gif-giphy-save").is_some());
    click(vcx, "settings-gif-giphy-enabled");
    click(vcx, "settings-gif-rating-g");
    click(vcx, "settings-gif-send-upload");
    reload(vcx);
    let s = live(vcx);
    assert!(s.plugins.entry("giphy").is_some_and(|e| e.enabled));
    assert_eq!(s.gifs.rating, "g");
    assert_eq!(s.gifs.send_as, settings::GifSendAs::Upload);
}

#[gpui::test]
fn calls_controls_write_their_keys_and_apply_live(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Calls);
    assert!(
        vcx.debug_bounds("settings-calls-camera").is_none(),
        "no camera control until video exists"
    );
    click(vcx, "settings-calls-ring");
    click(vcx, "settings-calls-ringtone-none");
    click(vcx, "settings-calls-join-browser");
    click(vcx, "settings-calls-mute");
    reload(vcx);
    let s = live(vcx);
    assert!(!s.calls.ring);
    assert_eq!(s.calls.ringtone, "none");
    assert_eq!(s.calls.join_with, JoinWith::Browser);
    assert!(!s.calls.mute_on_join);
    click(vcx, "settings-calls-join-native");
    reload(vcx);
    assert_eq!(live(vcx).calls.join_with, JoinWith::Native);
}

#[gpui::test]
fn calls_device_pickers_save_the_choice(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    cx.update(|cx| {
        cx.set_global(parla::call_manager::DeviceLister(std::sync::Arc::new(
            || {
                (
                    vec!["Built-in Microphone".into(), "USB Mic".into()],
                    vec!["Built-in Speakers".into()],
                )
            },
        )))
    });
    let (_view, vcx) = open_on(cx, &paths, Page::Calls);
    // Closed, the selects show the system default without listing devices.
    assert!(vcx.debug_bounds("settings-calls-input-1").is_none());
    click(vcx, "settings-calls-input");
    click(vcx, "settings-calls-input-1");
    click(vcx, "settings-calls-output");
    click(vcx, "settings-calls-output-0");
    reload(vcx);
    let s = live(vcx);
    assert_eq!(s.calls.input_device.as_deref(), Some("USB Mic"));
    assert_eq!(s.calls.output_device.as_deref(), Some("Built-in Speakers"));
    click(vcx, "settings-calls-input");
    click(vcx, "settings-calls-input-default");
    reload(vcx);
    assert_eq!(live(vcx).calls.input_device, None);
}

/// A fake device list for the Calls page selects.
fn fake_devices(cx: &mut TestAppContext) {
    cx.update(|cx| {
        cx.set_global(parla::call_manager::DeviceLister(std::sync::Arc::new(
            || {
                (
                    vec!["Built-in Microphone".into(), "USB Mic".into()],
                    vec!["Built-in Speakers".into()],
                )
            },
        )))
    });
}

#[gpui::test]
fn device_select_floats_over_the_page(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    fake_devices(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Calls);
    let trigger = vcx.debug_bounds("settings-calls-input").unwrap();
    let next = vcx.debug_bounds("settings-calls-output").unwrap();
    click(vcx, "settings-calls-input");
    assert_eq!(
        vcx.debug_bounds("settings-calls-output").unwrap(),
        next,
        "the next row does not move while the list is open"
    );
    assert_eq!(
        vcx.debug_bounds("settings-calls-input").unwrap(),
        trigger,
        "nor does the trigger"
    );
    let list = vcx
        .debug_bounds("settings-calls-input-list")
        .expect("the list is drawn");
    assert!(
        list.top() >= trigger.bottom(),
        "under the trigger: {list:?}"
    );
    assert!(
        list.bottom() > next.top(),
        "it covers the next row instead of pushing it: {list:?} {next:?}"
    );
}

#[gpui::test]
fn device_select_closes_on_outside_click_escape_and_trigger(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    fake_devices(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Calls);
    let trigger = vcx.debug_bounds("settings-calls-input").unwrap();
    click(vcx, "settings-calls-input");
    assert!(vcx.debug_bounds("settings-calls-input-1").is_some(), "open");
    // A press on the page well left of the select, over the section text.
    let outside = gpui::point(trigger.left() - gpui::px(240.), trigger.top());
    vcx.simulate_click(outside, Modifiers::none());
    vcx.run_until_parked();
    assert!(
        vcx.debug_bounds("settings-calls-input-1").is_none(),
        "an outside click closes it"
    );
    click(vcx, "settings-calls-input");
    assert!(vcx.debug_bounds("settings-calls-input-1").is_some());
    vcx.simulate_keystrokes("escape");
    vcx.run_until_parked();
    assert!(
        vcx.debug_bounds("settings-calls-input-1").is_none(),
        "Escape closes it"
    );
    click(vcx, "settings-calls-input");
    click(vcx, "settings-calls-input");
    assert!(
        vcx.debug_bounds("settings-calls-input-1").is_none(),
        "a second click on the trigger closes it"
    );
}

#[gpui::test]
fn device_select_options_follow_the_trigger_in_tab_order(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    fake_devices(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Calls);
    click(vcx, "settings-calls-input");
    // Default, then the first device.
    vcx.simulate_keystrokes("tab tab enter");
    vcx.run_until_parked();
    reload(vcx);
    assert_eq!(
        live(vcx).calls.input_device.as_deref(),
        Some("Built-in Microphone")
    );
    assert!(
        vcx.debug_bounds("settings-calls-input-1").is_none(),
        "a pick closes it"
    );
}

#[gpui::test]
fn device_select_list_takes_the_click_over_the_row_below(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    fake_devices(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Calls);
    let speaker = vcx.debug_bounds("settings-calls-output").unwrap().center();
    click(vcx, "settings-calls-input");
    // The option drawn over the Speaker trigger.
    let id = [
        "settings-calls-input-default",
        "settings-calls-input-0",
        "settings-calls-input-1",
    ]
    .into_iter()
    .find(|id| vcx.debug_bounds(id).is_some_and(|b| b.contains(&speaker)))
    .expect("an option covers the Speaker trigger");
    vcx.simulate_click(speaker, Modifiers::none());
    vcx.run_until_parked();
    assert!(
        vcx.debug_bounds("settings-calls-output-list").is_none(),
        "the Speaker select stays closed"
    );
    assert!(
        vcx.debug_bounds("settings-calls-input-list").is_none(),
        "the pick closes the list"
    );
    reload(vcx);
    let want = match id {
        "settings-calls-input-0" => Some("Built-in Microphone"),
        "settings-calls-input-1" => Some("USB Mic"),
        _ => None,
    };
    assert_eq!(
        live(vcx).calls.input_device.as_deref(),
        want,
        "{id} applied"
    );
}

fn input_open(vcx: &mut VisualTestContext) -> bool {
    vcx.debug_bounds("settings-calls-input-list").is_some()
}

/// The trigger has the focus: Enter on it opens the list again.
fn assert_trigger_focused(vcx: &mut VisualTestContext, after: &str) {
    assert!(!input_open(vcx), "closed after {after}");
    vcx.simulate_keystrokes("enter");
    vcx.run_until_parked();
    assert!(input_open(vcx), "the trigger has the focus after {after}");
}

#[gpui::test]
fn device_select_gives_the_focus_back_to_its_trigger(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    fake_devices(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Calls);
    click(vcx, "settings-calls-input");
    vcx.simulate_keystrokes("down escape");
    vcx.run_until_parked();
    assert_trigger_focused(vcx, "Escape");
    vcx.simulate_keystrokes("down down enter");
    vcx.run_until_parked();
    assert_trigger_focused(vcx, "an Enter pick");
    click(vcx, "settings-calls-input-1");
    assert_trigger_focused(vcx, "a mouse pick");
    reload(vcx);
    assert_eq!(live(vcx).calls.input_device.as_deref(), Some("USB Mic"));
}

#[gpui::test]
fn device_select_up_and_shift_tab_walk_back_to_the_trigger(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    fake_devices(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Calls);
    click(vcx, "settings-calls-input");
    // Down to the last option, Up back to the trigger: the list stays open.
    vcx.simulate_keystrokes("down down down up up up");
    vcx.run_until_parked();
    assert!(input_open(vcx), "Up walks the options without closing");
    // Shift-Tab on the trigger leaves the select and closes it; on an
    // option it would only move the focus.
    vcx.simulate_keystrokes("shift-tab");
    vcx.run_until_parked();
    assert!(!input_open(vcx), "Up reached the trigger");

    click(vcx, "settings-calls-input");
    vcx.simulate_keystrokes("down down shift-tab shift-tab");
    vcx.run_until_parked();
    assert!(
        input_open(vcx),
        "Shift-Tab walks the options without closing"
    );
    vcx.simulate_keystrokes("shift-tab");
    vcx.run_until_parked();
    assert!(!input_open(vcx), "Shift-Tab reached the trigger");
    reload(vcx);
    assert_eq!(live(vcx).calls.input_device, None, "nothing picked");
}

#[gpui::test]
fn device_select_tab_past_the_last_option_closes_it(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    fake_devices(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Calls);
    click(vcx, "settings-calls-input");
    // Default, Built-in, USB: three options, then past the last.
    vcx.simulate_keystrokes("tab tab tab");
    vcx.run_until_parked();
    assert!(input_open(vcx), "still on the last option");
    vcx.simulate_keystrokes("tab");
    vcx.run_until_parked();
    assert!(!input_open(vcx), "Tab past the last option closes it");
    reload(vcx);
    assert_eq!(live(vcx).calls.input_device, None, "nothing picked");
}

#[gpui::test]
fn keyboard_reaches_a_calls_switch(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Calls);
    // Header button, the nav (11 pages), then the ring switch.
    vcx.simulate_keystrokes(&"tab ".repeat(13));
    vcx.simulate_keystrokes("space");
    reload(vcx);
    assert!(!live(vcx).calls.ring);
}

#[gpui::test]
fn storage_steppers_write_their_keys(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Workspaces);
    click(vcx, "settings-cache-mb-inc");
    click(vcx, "settings-retention-dec");
    reload(vcx);
    let s = live(vcx);
    assert_eq!(s.storage.cache_mb, 16);
    assert_eq!(s.storage.retention_days, 30);
}

#[gpui::test]
fn clear_cache_asks_first(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Workspaces);
    click(vcx, "settings-clear-cache");
    assert!(vcx.has_pending_prompt(), "a confirmation is shown");
    vcx.simulate_prompt_answer("Cancel");
    vcx.run_until_parked();
    assert!(!vcx.has_pending_prompt());
}

#[gpui::test]
fn advanced_steppers_write_their_keys_and_apply_live(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Advanced);
    click(vcx, "settings-image-cache-inc");
    click(vcx, "settings-message-window-inc");
    click(vcx, "settings-lean-dec");
    reload(vcx);
    let s = live(vcx);
    assert_eq!(s.performance.image_cache_mb, 64);
    assert_eq!(s.performance.message_window, 500);
    assert_eq!(s.performance.lean_after_minutes, 5);
}

#[gpui::test]
fn the_log_section_sets_the_level_and_offers_logs_and_diagnostics(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Advanced);
    assert!(vcx.debug_bounds("settings-open-logs").is_some());
    assert!(vcx.debug_bounds("settings-export-diagnostics").is_some());
    assert_eq!(live(vcx).log.level, settings::LogLevel::Info);
    click(vcx, "settings-log-debug");
    reload(vcx);
    assert_eq!(live(vcx).log.level, settings::LogLevel::Debug);
    click(vcx, "settings-log-error");
    reload(vcx);
    assert_eq!(live(vcx).log.level, settings::LogLevel::Error);
}

#[gpui::test]
fn reset_settings_asks_then_restores_the_template(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    std::fs::write(&paths.settings, r#"{ "density": "cozy" }"#).unwrap();
    let (_view, vcx) = open_on(cx, &paths, Page::Advanced);
    reload(vcx);
    click(vcx, "settings-reset-settings");
    assert!(vcx.has_pending_prompt());
    vcx.simulate_prompt_answer("Cancel");
    vcx.run_until_parked();
    reload(vcx);
    assert_eq!(
        live(vcx).density,
        settings::Density::Cozy,
        "cancel keeps it"
    );

    click(vcx, "settings-reset-settings");
    vcx.simulate_prompt_answer("Reset settings");
    vcx.run_until_parked();
    reload(vcx);
    assert_eq!(live(vcx).density, settings::Density::Compact);
    let text = std::fs::read_to_string(&paths.settings).unwrap();
    assert!(text.contains("//"), "the commented template: {text}");
}

/// A sign-in that only records which workspaces were signed out.
struct RecordingSignIn {
    outs: std::rc::Rc<std::cell::RefCell<Vec<WorkspaceKey>>>,
}

impl components::sign_in::SignIn for RecordingSignIn {
    fn provider_name(&self) -> String {
        "Fake".into()
    }
    fn notice(&self) -> Vec<components::sign_in::SignInText> {
        vec![]
    }
    fn options(&self) -> Vec<components::sign_in::SignInOption> {
        vec![]
    }
    fn run(
        &self,
        _: &str,
        _: Vec<(String, String)>,
        _: futures::channel::mpsc::UnboundedSender<components::workspace_picker::RefreshStep>,
        _: &mut gpui::Window,
        _: &mut gpui::App,
    ) -> gpui::Task<Result<components::sign_in::SignInResult, components::sign_in::SignInError>>
    {
        gpui::Task::ready(Err(components::sign_in::SignInError::Failed(
            components::sign_in::SignInText::Plain("unused".into()),
        )))
    }
    fn sign_out(&self, ws: &WorkspaceKey, _: &mut gpui::App) -> gpui::Task<Result<(), String>> {
        self.outs.borrow_mut().push(ws.clone());
        gpui::Task::ready(Ok(()))
    }
}

type Outs = std::rc::Rc<std::cell::RefCell<Vec<WorkspaceKey>>>;

/// The Workspaces page over three running workspaces and the main window.
fn accounts_env(
    cx: &mut TestAppContext,
) -> (support::Env, gpui::Entity<parla::root_view::RootView>) {
    accounts_env_with(cx).0
}

fn accounts_env_with(
    cx: &mut TestAppContext,
) -> (
    (support::Env, gpui::Entity<parla::root_view::RootView>),
    Outs,
) {
    let mut e = support::env(cx, "{}");
    let outs: Outs = Default::default();
    e.registry
        .add_sign_in(std::rc::Rc::new(RecordingSignIn { outs: outs.clone() }));
    let dirs = parla::removal::AppDirs {
        data: e.data.clone(),
        cache: e.cache.clone(),
    };
    cx.update(|cx| {
        cx.set_global(dirs);
        i18n::gpui::set_pref(cx, LocalePref::Code("en".parse().unwrap()));
    });
    let r = e.registry.clone();
    let st: std::sync::Arc<dyn store::Store> = std::sync::Arc::new(store::MemoryStore::default());
    let (root, rcx) =
        cx.add_window_view(|window, cx| parla::root_view::RootView::new(&r, st, window, cx));
    // The accounts start on their own threads; every settings-page test needs
    // them all running before it acts.
    let w = rcx.update(|_, cx| cx.global::<parla::workspaces::WorkspacesHandle>().0.clone());
    support::settle_visual_until(rcx, move |rcx| {
        w.read_with(rcx, |w, cx| w.ready_to_lock(cx))
    });
    ((e, root), outs)
}

fn settle(vcx: &mut VisualTestContext) {
    for _ in 0..30 {
        vcx.run_until_parked();
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
}

fn open_accounts<'a>(cx: &'a mut TestAppContext, e: &support::Env) -> &'a mut VisualTestContext {
    let path = e.config.join("settings.json");
    let (_view, vcx) = cx.add_window_view(move |_, _| {
        parla::settings_window::view_on(path, "1.2.3".into(), Page::Workspaces)
    });
    settle(vcx);
    vcx
}

fn drawn(vcx: &mut VisualTestContext, id: &str) -> bool {
    vcx.debug_bounds(Box::leak(id.to_string().into_boxed_str()))
        .is_some()
}

#[gpui::test]
async fn sign_out_signs_that_workspace_out_after_a_confirm(cx: &mut TestAppContext) {
    let ((_e, _root), outs) = accounts_env_with(cx);
    let vcx = open_accounts(cx, &_e);
    click(vcx, "settings-account-1-signout");
    vcx.simulate_prompt_answer("Sign out");
    settle(vcx);
    assert_eq!(*outs.borrow(), vec![WorkspaceKey::new("fake", "w2")]);
}

#[gpui::test]
async fn clear_cache_rebuilds_every_session_after_a_confirm(cx: &mut TestAppContext) {
    let ((_e, _root), _) = accounts_env_with(cx);
    let vcx = open_accounts(cx, &_e);
    let sessions = vcx.update(|_, cx| {
        cx.global::<parla::workspaces::WorkspacesHandle>()
            .0
            .read(cx)
            .sessions()
            .cloned()
            .collect::<Vec<_>>()
    });
    assert_eq!(sessions.len(), 3);
    let rebuilt: Outs = Default::default();
    for s in &sessions {
        let key = s.read_with(vcx, |s, _| s.state().workspace.clone());
        let sink = rebuilt.clone();
        vcx.update(|_, cx| {
            cx.subscribe(s, move |_, ev: &parla::session::SessionEvent, _| {
                if *ev == parla::session::SessionEvent::Store {
                    sink.borrow_mut().push(key.clone());
                }
            })
            .detach()
        });
    }
    click(vcx, "settings-clear-cache");
    vcx.simulate_prompt_answer("Clear cache");
    // Each rebuild drops a database and recreates it on the store's threads.
    support::settle_visual_until(vcx, |_| {
        let mut got = rebuilt.borrow().clone();
        got.sort();
        got.dedup();
        got.len() == 3
    });
    let mut got = rebuilt.borrow().clone();
    got.sort();
    got.dedup();
    assert_eq!(got.len(), 3, "each session rebuilt its store: {got:?}");
}

#[gpui::test]
async fn a_workspace_status_follows_its_session(cx: &mut TestAppContext) {
    let ((e, _root), _) = accounts_env_with(cx);
    let path = e.config.join("settings.json");
    let (view, vcx) = cx.add_window_view(move |_, _| {
        parla::settings_window::view_on(path, "1.2.3".into(), Page::Workspaces)
    });
    settle(vcx);
    assert!(drawn(vcx, "settings-account-0-connected"));
    let notified = std::rc::Rc::new(std::cell::Cell::new(0));
    let n = notified.clone();
    vcx.update(|_, cx| cx.observe(&view, move |_, _| n.set(n.get() + 1)).detach());

    // A new message is not a reason to redraw the Settings window.
    e.providers[0].push_event(domain::ProviderEvent::MessagePosted(
        domain::test_support::msg("D1", "2.000000", "later"),
    ));
    settle(vcx);
    assert_eq!(notified.get(), 0, "no redraw for a message");

    e.providers[0].push_event(domain::ProviderEvent::SessionExpired);
    settle(vcx);
    assert!(notified.get() > 0, "the session change redraws the page");
    assert!(drawn(vcx, "settings-account-0-expired"));
    assert!(drawn(vcx, "settings-account-0-signin"));
}

#[gpui::test]
async fn workspaces_list_signs_out_and_deletes_only_after_a_confirm(cx: &mut TestAppContext) {
    let (e, _root) = accounts_env(cx);
    let vcx = open_accounts(cx, &e);
    for i in 0..3 {
        assert!(
            drawn(vcx, &format!("settings-account-{i}-signout")),
            "row {i}"
        );
    }
    click(vcx, "settings-account-1-signout");
    assert!(vcx.has_pending_prompt(), "sign out asks first");
    vcx.simulate_prompt_answer("Cancel");
    vcx.run_until_parked();
    assert_eq!(e.registry.accounts().len(), 3);

    click(vcx, "settings-delete-all");
    assert!(vcx.has_pending_prompt(), "delete asks first");
    vcx.simulate_prompt_answer("Cancel");
    vcx.run_until_parked();
    assert!(e.data.exists());

    click(vcx, "settings-delete-all");
    vcx.simulate_prompt_answer("Delete all data");
    // The removal runs on the background executor: wait for the data dir.
    let data = e.data.clone();
    support::settle_visual_until(vcx, move |_| !data.exists());
    assert!(!e.data.exists());
    assert!(e.config.join("settings.json").exists());
}

#[gpui::test]
fn a_stepper_at_the_end_of_its_range_does_nothing(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    std::fs::write(&paths.settings, r#"{ "storage": { "cache_mb": 128 } }"#).unwrap();
    let (_view, vcx) = open_on(cx, &paths, Page::Workspaces);
    reload(vcx);
    let before = std::fs::read_to_string(&paths.settings).unwrap();
    click(vcx, "settings-cache-mb-inc");
    assert_eq!(std::fs::read_to_string(&paths.settings).unwrap(), before);
    click(vcx, "settings-cache-mb-dec");
    reload(vcx);
    assert_eq!(live(vcx).storage.cache_mb, 64);
}

#[test]
fn privacy_changes_name_their_key_and_json_value() {
    let c = |path: Vec<String>, raw: String| (path.join("."), raw);
    let p = PrivacyChange::LockAfterMinutes(15);
    assert_eq!(
        c(p.key_path(), p.raw_json()),
        ("storage.lock_after_minutes".into(), "15".into())
    );
    let p = PrivacyChange::CrashReports(settings::CrashReports::Local);
    assert_eq!(
        c(p.key_path(), p.raw_json()),
        ("crash_reports".into(), "\"local\"".into())
    );
}

#[gpui::test]
fn privacy_rows_set_idle_lock_and_crash_reports(cx: &mut TestAppContext) {
    let (_dir, paths) = setup(cx);
    let (_view, vcx) = open_on(cx, &paths, Page::Workspaces);
    assert_eq!(live(vcx).storage.lock_after_minutes, 0);
    click(vcx, "settings-lock-15");
    click(vcx, "settings-crash-local");
    reload(vcx);
    let s = live(vcx);
    assert_eq!(s.storage.lock_after_minutes, 15);
    assert_eq!(s.crash_reports, settings::CrashReports::Local);
    click(vcx, "settings-lock-off");
    click(vcx, "settings-crash-off");
    reload(vcx);
    let s = live(vcx);
    assert_eq!(s.storage.lock_after_minutes, 0);
    assert_eq!(s.crash_reports, settings::CrashReports::Off);
}
