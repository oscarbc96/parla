//! one golden image per component state, light and dark.
#![cfg(not(target_os = "windows"))]

use gpui::{AnyElement, AppContext as _, IntoElement as _, ParentElement as _, Styled as _};
use themes::ActiveTheme as _;
use ui_testing::stage::{self, BannerFn, StageOptions};
use ui_testing::{Tolerance, UiHarness, Variant, assert_matches, scenario};

const SUITE: &str = "components";

fn run(name: &str, opts: impl Fn() -> StageOptions) {
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let s = scenario::by_name(name).unwrap_or_else(|| panic!("no scenario {name}"));
        let img = stage::capture(&mut h, s, opts()).unwrap();
        assert_matches(SUITE, name, &v, &img, Tolerance::default());
    }
}

fn plain(name: &str) {
    run(name, StageOptions::default);
}

fn session_expired() -> BannerFn {
    Box::new(|_, cx| -> AnyElement {
        parla::expiry_banner::expiry_banner(cx, "Example Workspace", |_, _| {}).into_any_element()
    })
}

fn offline() -> BannerFn {
    Box::new(|_, cx| -> AnyElement {
        parla::offline_banner::offline_banner(cx, |_, _| {}).into_any_element()
    })
}

fn config_error() -> BannerFn {
    Box::new(|_, cx| -> AnyElement {
        let colors = *cx.theme().colors();
        parla::root_view::error_strip(
            &colors,
            themes::parla_tokens(cx.theme()).danger_soft,
            "settings.toml:3:5: expected a boolean",
        )
        .into_any_element()
    })
}

fn incoming_huddle() -> BannerFn {
    Box::new(|_, cx| -> AnyElement {
        let data = components::RingBannerData {
            caller: "Ada Lovelace".into(),
        };
        components::ring_banner(&data, |_, _, _| {}, |_, _, _| {}, cx).into_any_element()
    })
}

/// The update notice alone in a strip-sized window: the whole-window images
/// would not fit the baseline folder budget (36 MB per platform).
struct UpdateStrip {
    phase: parla::updates::UpdatePhase,
    can_install: bool,
}

impl gpui::Render for UpdateStrip {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        let version = semver::Version::new(0, 6, 0);
        let url = url::Url::parse("https://github.com/oscarbc96/parla/releases/tag/v0.6.0")
            .expect("release url");
        let notice = parla::updates::notice_for(cx, &version, &url, self.phase, self.can_install);
        gpui::div()
            .size_full()
            .bg(cx.theme().colors().background)
            .child(parla::update_banner::update_banner(cx, &notice))
    }
}

fn update_strip(name: &str, phase: parla::updates::UpdatePhase, can_install: bool) {
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let w = h.open_sized(960.0, 52.0, move |_, app| {
            app.new(|_| UpdateStrip { phase, can_install })
        });
        let img = h.capture(w.into()).unwrap();
        assert_matches(SUITE, name, &v, &img, Tolerance::default());
    }
}

#[test]
fn update_available() {
    update_strip(
        "update_available",
        parla::updates::UpdatePhase::Offer,
        false,
    );
}

#[test]
fn update_available_install() {
    update_strip(
        "update_available_install",
        parla::updates::UpdatePhase::Offer,
        true,
    );
}

#[test]
fn update_downloading() {
    update_strip(
        "update_downloading",
        parla::updates::UpdatePhase::Downloading { percent: 42 },
        true,
    );
}

#[test]
fn update_installing() {
    update_strip(
        "update_installing",
        parla::updates::UpdatePhase::Installing,
        true,
    );
}

#[test]
fn update_ready() {
    update_strip("update_ready", parla::updates::UpdatePhase::Ready, true);
}

#[test]
fn update_failed() {
    update_strip(
        "update_failed",
        parla::updates::UpdatePhase::Failed(parla::updates::UpdateFailure::Signature),
        true,
    );
}

/// The board's toast stack: a success with detail and Undo, a link copy and an
/// error. The text comes from catalogs (the components one and the fake one) so
/// the pseudo-locale baseline exercises every string.
fn toast_samples() -> StageOptions {
    StageOptions {
        toasts: Some(Box::new(|cx| {
            // The headless clock may run on while the frame settles: stay.
            let stay = std::time::Duration::from_secs(24 * 3600);
            let l = i18n::gpui::localizer(cx);
            let text = |ns: &str, id: &str| l.format(ns, id, None).into_owned();
            vec![
                components::Toast::new(
                    components::ToastKind::Success,
                    text("fake", "toast-sample-title"),
                )
                .detail(text("fake", "toast-sample-detail"))
                .action(text("fake", "toast-sample-undo"), |_| {})
                .timeout(stay),
                components::Toast::new(
                    components::ToastKind::Info,
                    text("components", "toast-link-copied"),
                )
                .timeout(stay),
                components::Toast::new(
                    components::ToastKind::Error,
                    text("fake", "toast-sample-error"),
                )
                .detail(text("fake", "toast-sample-error-detail"))
                .timeout(stay),
            ]
        })),
        ..Default::default()
    }
}

#[test]
fn toast_stack() {
    for v in Variant::defaults() {
        let mut h =
            UiHarness::with_catalogs(v, &[parla::catalog(), domain::test_support::fake_catalog()])
                .unwrap();
        let s = scenario::by_name("toast_stack").unwrap();
        let img = stage::capture(&mut h, s, toast_samples()).unwrap();
        assert_matches(SUITE, "toast_stack", &v, &img, Tolerance::default());
    }
}

#[test]
fn toast_stack_en_xa() {
    for v in Variant::defaults() {
        let mut h =
            UiHarness::with_catalogs(v, &[parla::catalog(), domain::test_support::fake_catalog()])
                .unwrap();
        h.app().update(|cx| {
            let xa: i18n::LanguageIdentifier = i18n::PSEUDO.parse().unwrap();
            assert!(i18n::gpui::set_pref_with_os(
                cx,
                i18n::LocalePref::Code(xa),
                &[]
            ));
        });
        let s = scenario::by_name("toast_stack").unwrap();
        let img = stage::capture(&mut h, s, toast_samples()).unwrap();
        assert_matches(SUITE, "toast_stack_en_xa", &v, &img, Tolerance::default());
    }
}

/// The scenario in the `en-XA` pseudo-locale (text about 40% longer), to catch
/// clipped or overlapping text; its golden is named `<scenario>_en_xa`.
fn pseudo(name: &str) {
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        h.app().update(|cx| {
            let xa: i18n::LanguageIdentifier = i18n::PSEUDO.parse().unwrap();
            assert!(i18n::gpui::set_pref_with_os(
                cx,
                i18n::LocalePref::Code(xa),
                &[]
            ));
        });
        let s = scenario::by_name(name).unwrap_or_else(|| panic!("no scenario {name}"));
        let img = stage::capture(&mut h, s, StageOptions::default()).unwrap();
        assert_matches(
            SUITE,
            &format!("{name}_en_xa"),
            &v,
            &img,
            Tolerance::default(),
        );
    }
}

#[test]
fn sign_in_email_en_xa() {
    pseudo("sign_in_email");
}

#[test]
fn sign_in_code_wrong_en_xa() {
    pseudo("sign_in_code_wrong");
}

#[test]
fn sign_in_en_xa() {
    pseudo("sign_in_long");
}

#[test]
fn sign_in_form_en_xa() {
    pseudo("sign_in_form_long");
}

#[test]
fn workspace_picker_en_xa() {
    pseudo("workspace_picker");
}

#[test]
fn timeline_unread_divider_en_xa() {
    pseudo("timeline_unread_divider");
}

#[test]
fn rail_and_header_en_xa() {
    pseudo("rail_and_header");
}

#[test]
fn scheduled_long_en_xa() {
    pseudo("scheduled_long");
}

#[test]
fn channel_create_en_xa() {
    pseudo("channel_create");
}

#[test]
fn channel_browser_en_xa() {
    pseudo("channel_browser");
}

/// The scenario with the OS "Increase contrast" switch on: the brand themes
/// become Parla High Contrast Light and Dark. Golden `<scenario>_high_contrast`.
fn high_contrast(name: &str) {
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        h.app().update(|cx| {
            let mut s = settings::Settings::default();
            s.theme.mode = match v.appearance {
                ui_testing::Appearance::Light => settings::ThemeMode::Light,
                ui_testing::Appearance::Dark => settings::ThemeMode::Dark,
            };
            themes::set_increase_contrast(cx, true);
            themes::apply(cx, &s);
            assert_eq!(cx.theme().kind(), themes::ThemeKind::HighContrast);
        });
        let s = scenario::by_name(name).unwrap_or_else(|| panic!("no scenario {name}"));
        let img = stage::capture(&mut h, s, StageOptions::default()).unwrap();
        assert_matches(
            SUITE,
            &format!("{name}_high_contrast"),
            &v,
            &img,
            Tolerance::default(),
        );
    }
}

#[test]
fn timeline_rich_high_contrast() {
    high_contrast("timeline_rich");
}

macro_rules! plain_tests {
    ($($name:ident),* $(,)?) => {$(
        #[test]
        fn $name() {
            plain(stringify!($name));
        }
    )*};
}

plain_tests!(
    sign_in_email,
    sign_in_email_invalid,
    sign_in_email_sending,
    sign_in_email_needs_browser,
    sign_in_email_rate_limited,
    sign_in_email_offline,
    sign_in_code,
    sign_in_code_partial,
    sign_in_code_verifying,
    sign_in_code_wrong,
    sign_in_code_expired,
    sign_in_code_too_many,
    sign_in_code_rate_limited,
    sign_in_code_resend_wait,
    sign_in_code_resent,
    sign_in,
    sign_in_form,
    sign_in_error,
    sign_in_long,
    sign_in_form_long,
    workspace_picker,
    sidebar_busy,
    sidebar_starred,
    sidebar_timed_mute,
    long_names,
    unresolved_names,
    rail_and_header,
    rail_self,
    rail_scroll,
    rail_multi,
    rail_org,
    notify_controls,
    timeline_rich,
    timeline_unread_divider,
    timeline_markup,
    timeline_bot_alert,
    timeline_states,
    timeline_selected,
    empty_conversation,
    timeline_loading,
    timeline_load_error,
    thread_loading,
    thread_load_error,
    dm_section_collapsed,
    dm_section_expanded,
    reactions,
    thread_open,
    thread_followed,
    composer_empty,
    composer_typing,
    gif_picker_results,
    gif_picker_setup_card,
    gif_picker_empty,
    composer_multiline,
    composer_editing,
    composer_selection,
    composer_slash_menu,
    composer_link_prompt,
    composer_emoji_button_picker,
    composer_send_later_menu,
    composer_send_later_custom,
    scheduled_view,
    message_actions_menu,
    pins_popover,
    bookmarks_bar,
    later_view,
    threads_view,
    reminders_view,
    reminders_empty,
    reminder_edit,
    reminder_snooze_menu,
    drafts_view,
    drafts_empty,
    sidebar_sections_collapsed,
    sidebar_custom_sections,
    sidebar_organise,
    sidebar_section_menu,
    sidebar_settings_popover,
    scheduled_edit,
    profile_panel,
    people_directory,
    people_directory_empty_search,
    channel_details_about,
    channel_details_members,
    channel_details_recent,
    presence_dots,
    huddle_awareness,
    huddle_self_in,
    call_bar_live,
    call_bar_connecting,
    call_bar_people,
    call_bar_devices,
    huddle_start,
    status_dialog,
    status_dialog_custom,
    channel_create,
    channel_create_error,
    channel_browser,
    channel_edit,
    channel_invite,
    channel_leave_confirm,
    channel_archive_confirm,
    channel_browser_loading,
    channel_browser_empty,
    channel_browser_error,
    channel_details_about_admin,
    status_presets,
    dm_header,
    dm_intro,
    all_unreads,
    all_mentions,
    all_scheduled,
    all_unreads_confirm,
    all_unreads_previews,
    composer_mention_picker,
    composer_mention_groups,
    composer_channel_picker,
    composer_emoji_picker,
    composer_composing,
    composer_attachments,
    composer_snippet_offer,
    snippet_card,
    snippet_card_expanded,
    clip_controls,
    files,
    files_tab,
    files_tab_empty,
    files_upload_progress,
    files_upload_failed,
    search_results,
    search_offline,
    search_preview,
);

#[test]
fn timeline_offline() {
    run("timeline_offline", || StageOptions {
        banner: Some(offline()),
        ..Default::default()
    });
}

#[test]
fn composer_session_expired() {
    run("composer_session_expired", || StageOptions {
        banner: Some(session_expired()),
        ..Default::default()
    });
}

#[test]
fn banner_session_expired() {
    run("banner_session_expired", || StageOptions {
        banner: Some(session_expired()),
        ..Default::default()
    });
}

#[test]
fn banner_incoming_huddle() {
    run("banner_incoming_huddle", || StageOptions {
        banner: Some(incoming_huddle()),
        ..Default::default()
    });
}

#[test]
fn banner_config_error() {
    run("banner_config_error", || StageOptions {
        banner: Some(config_error()),
        ..Default::default()
    });
}

#[test]
fn emoji_picker_open() {
    // Same data as `reactions`, with the picker open on the newest message.
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let s = scenario::by_name("reactions").unwrap();
        let img = stage::capture(
            &mut h,
            s,
            StageOptions {
                picker: true,
                ..Default::default()
            },
        )
        .unwrap();
        assert_matches(SUITE, "emoji_picker_open", &v, &img, Tolerance::default());
    }
}

#[test]
fn quick_switcher_open() {
    use components::{SwitchItem, SwitchTarget};
    use domain::test_support::cid;
    let item = |w: &str, c: &str, label: &str, unread: bool, mentions: u32| SwitchItem {
        workspace: settings::WorkspaceKey::new("fake", w),
        target: SwitchTarget::Conversation(cid(c)),
        label: label.to_string().into(),
        workspace_name: if w == "w1" {
            "Example Workspace"
        } else {
            "Side Project"
        }
        .into(),
        unread,
        mentions,
        joined: true,
    };
    let items = || {
        vec![
            item("w1", "C1", "general", false, 0),
            item("w1", "C2", "design-reviews", true, 0),
            item("w1", "D1", "Ana Lopez", true, 2),
            item("w2", "C1", "general", false, 0),
            item("w2", "C3", "generators", false, 0),
            // A public channel the user is not in, marked "Not joined".
            SwitchItem {
                joined: false,
                ..item("w1", "C4", "gen-ai-lab", false, 0)
            },
        ]
    };
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let s = scenario::by_name("rail_multi").unwrap();
        let img = stage::capture(
            &mut h,
            s,
            StageOptions {
                switcher: Some((items(), "gen".into())),
                ..Default::default()
            },
        )
        .unwrap();
        assert_matches(SUITE, "quick_switcher_open", &v, &img, Tolerance::default());
    }
}

/// A notification menu over a scenario at a point.
fn menu_test(name: &str, scenario_name: &str, kind: components::MenuKind, at: (f32, f32)) {
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let s = scenario::by_name(scenario_name).unwrap();
        let img = stage::capture(
            &mut h,
            s,
            StageOptions {
                menu: Some((kind.clone(), at)),
                ..Default::default()
            },
        )
        .unwrap();
        assert_matches(SUITE, name, &v, &img, Tolerance::default());
    }
}

#[test]
fn pause_menu_open() {
    menu_test(
        "pause_menu_open",
        "rail_multi",
        components::MenuKind::Workspace,
        (60.0, 40.0),
    );
}

#[test]
fn pause_menu_paused() {
    menu_test(
        "pause_menu_paused",
        "notify_controls",
        components::MenuKind::Workspace,
        (60.0, 40.0),
    );
}

/// The notification levels of one conversation (the header bell's menu).
#[test]
fn notify_menu_open() {
    menu_test(
        "notify_menu_open",
        "notify_controls",
        components::MenuKind::Conversation(domain::test_support::cid("C1")),
        (700.0, 80.0),
    );
}

/// The same menu with thread replies off, sound off and a timed mute running.
#[test]
fn notify_menu_extras() {
    menu_test(
        "notify_menu_extras",
        "notify_menu_extras",
        components::MenuKind::Conversation(domain::test_support::cid("C1")),
        (700.0, 80.0),
    );
}

/// A sidebar row's right-click menu (the starred channel, so it offers Unstar).
#[test]
fn conversation_menu_open() {
    menu_test(
        "conversation_menu_open",
        "sidebar_starred",
        components::MenuKind::Row(domain::test_support::cid("C4")),
        (150.0, 120.0),
    );
}

#[test]
fn every_scenario_has_a_test() {
    let src = include_str!("visual_components.rs");
    for s in scenario::all() {
        assert!(
            src.contains(&format!("fn {}", s.name)) || src.contains(&format!("    {},", s.name)),
            "scenario {} has no visual test",
            s.name
        );
    }
}

/// The timeline and composer sit on the editor surface, not the sidebar's:
/// text is drawn in `text`, which is only readable on the editor background.
#[test]
fn timeline_and_composer_use_the_editor_background() {
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let s = scenario::by_name("long_names").unwrap();
        let img = stage::capture(&mut h, s, StageOptions::default()).unwrap();
        let editor = h.app().update(|cx| cx.theme().colors().editor_background);
        let c = themes::color::hsla_to_rgba(editor);
        let want = [c.r, c.g, c.b].map(|v| (v * 255.0).round() as u8);
        // Logical points: blank timeline area, and the composer's right padding.
        for (name, x, y) in [("timeline", 600.0, 75.0), ("composer", 954.0, 546.0)] {
            let p = img.get_pixel((x * v.scale) as u32, (y * v.scale) as u32).0;
            assert_eq!([p[0], p[1], p[2]], want, "{name} background in {v:?}");
        }
    }
}

/// An unbroken token wraps inside the column instead of running off the edge.
#[test]
fn long_unbroken_token_wraps_inside_the_column() {
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let s = scenario::by_name("long_names").unwrap();
        let img = stage::capture(&mut h, s, StageOptions::default()).unwrap();
        let editor = h.app().update(|cx| cx.theme().colors().editor_background);
        let c = themes::color::hsla_to_rgba(editor);
        let bg = [c.r, c.g, c.b].map(|v| (v * 255.0).round() as u8);
        // The message sits in the lower part of the timeline (logical 430..520).
        // Its right gutter (the last 12 points) must stay blank.
        let x0 = ((960.0 - 12.0) * v.scale) as u32;
        for y in (430.0 * v.scale) as u32..(520.0 * v.scale) as u32 {
            for x in x0..img.width() {
                let p = img.get_pixel(x, y).0;
                assert_eq!([p[0], p[1], p[2]], bg, "ink at ({x},{y}) in {v:?}");
            }
        }
    }
}

/// A clicked message is marked with the subtle `element.selected` surface, not
/// the saturated sidebar accent that drowned its text and metadata.
#[test]
fn selected_message_uses_the_subtle_selected_surface() {
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let s = scenario::by_name("timeline_selected").unwrap();
        let img = stage::capture(&mut h, s, StageOptions::default()).unwrap();
        let selected = h.app().update(|cx| cx.theme().colors().element_selected);
        let c = themes::color::hsla_to_rgba(selected);
        let want = [c.r, c.g, c.b].map(|v| (v * 255.0).round() as u8);
        // Logical point in the blank right part of the selected (middle) row.
        let p = img
            .get_pixel((940.0 * v.scale) as u32, (400.0 * v.scale) as u32)
            .0;
        assert_eq!([p[0], p[1], p[2]], want, "selected row background in {v:?}");
    }
}

/// The MCP write confirmation prompt, shown in a window-sized stage.
struct PromptStage(gpui::Entity<components::ConfirmWriteView>);

impl gpui::Render for PromptStage {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement as _, Styled as _, div};
        let bg = cx.theme().colors().background;
        div()
            .size_full()
            .bg(bg)
            .flex()
            .items_center()
            .justify_center()
            .child(
                div()
                    // The real prompt window (`write_confirm_window.rs`).
                    .w(
                        themes::metrics::LAYOUT_WINDOW_CONFIRM_MIN_WIDTH
                            + themes::metrics::SPACE_10,
                    )
                    .h(themes::metrics::LAYOUT_WINDOW_CONFIRM_HEIGHT)
                    .child(self.0.clone()),
            )
    }
}

#[test]
fn confirm_write() {
    use gpui::AppContext as _;
    use std::sync::Arc;
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let req = mcp::confirm::WriteRequest {
            client: mcp::audit::ClientInfo {
                id: mcp::limits::ClientId(1),
                name: Arc::from("claude-code"),
                peer_pid: Some(4242),
            },
            workspace: mcp::backend::WorkspaceInfo {
                key: settings::WorkspaceKey::new("fake", "W1"),
                name: Arc::from("Acme"),
                provider_display: Arc::from("Fakechat"),
                self_user: None,
                caps: domain::Capabilities::default(),
            },
            as_user: Some(Arc::from("Marta Vidal")),
            action: mcp::confirm::WriteAction::Send {
                conversation_name: Arc::from("design-system"),
                thread: None,
                broadcast: false,
                text: Arc::from(
                    "Contrast check done: *muted* sidebar text passes AA.\n\
                     <@U1> stays literal, hidden \u{202e}override shows.",
                ),
            },
        };
        let w = h.open(move |_, cx| {
            let view = cx.new(|cx| components::ConfirmWriteView::new(req, cx));
            cx.new(|_| PromptStage(view))
        });
        // The prompt as the user meets it: window focused, buttons armed.
        h.app()
            .update_window(w.into(), |_, window, _| window.activate_window())
            .unwrap();
        h.settle();
        h.app().advance_clock(
            components::confirm_write::ARM_DELAY + std::time::Duration::from_millis(100),
        );
        let img = h.capture(w.into()).unwrap();
        assert_matches(SUITE, "confirm_write", &v, &img, Tolerance::default());
    }
}

/// A plugin's write prompt, captured armed, as the user meets it, after
/// `keys` (each re-arms the buttons once more).
fn plugin_prompt(name: &str, action: write_confirm::WriteAction, keys: &[&str]) {
    use gpui::AppContext as _;
    use std::sync::Arc;
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let req = write_confirm::WriteRequest::new(
            write_confirm::Requester::Plugin {
                id: Arc::from("standup-bot"),
                name: Arc::from("Standup Bot"),
                source: write_confirm::PluginSource::Registry,
            },
            write_confirm::Workspace {
                key: settings::WorkspaceKey::new("fake", "W1"),
                name: Arc::from("Acme"),
            },
            Some(Arc::from("Marta Vidal")),
            action.clone(),
        );
        let w = h.open(move |_, cx| {
            let view = cx.new(|cx| components::ConfirmWriteView::new(req, cx));
            cx.new(|_| PromptStage(view))
        });
        h.app()
            .update_window(w.into(), |_, window, _| window.activate_window())
            .unwrap();
        h.settle();
        h.app().advance_clock(
            components::confirm_write::ARM_DELAY + std::time::Duration::from_millis(100),
        );
        for key in keys {
            h.app()
                .update_window(w.into(), |_, window, cx| {
                    window.dispatch_keystroke(
                        gpui::Keystroke::parse_with_os(key, cx.platform_info().os).unwrap(),
                        cx,
                    );
                })
                .unwrap();
            h.settle();
            h.app().advance_clock(
                components::confirm_write::ARM_DELAY + std::time::Duration::from_millis(100),
            );
        }
        let img = h.capture(w.into()).unwrap();
        assert_matches(SUITE, name, &v, &img, Tolerance::default());
    }
}

#[test]
fn confirm_write_plugin_send() {
    plugin_prompt(
        "confirm_write_plugin_send",
        write_confirm::WriteAction::Send {
            conversation_name: std::sync::Arc::from("design-system"),
            thread: None,
            broadcast: false,
            text: std::sync::Arc::from("Standup in 5 minutes: post your update in the thread."),
        },
        &[],
    );
}

#[test]
fn confirm_write_plugin_delete() {
    plugin_prompt(
        "confirm_write_plugin_delete",
        write_confirm::WriteAction::Delete {
            conversation_name: std::sync::Arc::from("design-system"),
            message_excerpt: std::sync::Arc::from("Standup moved to 10:30 today."),
        },
        &[],
    );
}

/// The check box ticked from the keyboard, with its focus ring.
#[test]
fn confirm_write_plugin_remember() {
    plugin_prompt(
        "confirm_write_plugin_remember",
        write_confirm::WriteAction::Send {
            conversation_name: std::sync::Arc::from("design-system"),
            thread: None,
            broadcast: false,
            text: std::sync::Arc::from("Standup in 5 minutes: post your update in the thread."),
        },
        &["shift-tab", "space"],
    );
}

/// A write with no text of its own, remembered for the plugin.
#[test]
fn confirm_write_plugin_mark_read() {
    plugin_prompt(
        "confirm_write_plugin_mark_read",
        write_confirm::WriteAction::MarkRead {
            conversation_name: std::sync::Arc::from("design-system"),
        },
        &[],
    );
}

/// Names for the audit log stage.
struct StageNames;

impl components::AuditNames for StageNames {
    fn workspaces(&self, _: &gpui::App) -> Vec<(settings::WorkspaceKey, gpui::SharedString)> {
        vec![(settings::WorkspaceKey::new("fake", "W1"), "Acme".into())]
    }
    fn tools(&self) -> Vec<gpui::SharedString> {
        vec!["get_unreads".into(), "send_message".into()]
    }
    fn conversation(
        &self,
        _: &settings::WorkspaceKey,
        id: &str,
        _: &gpui::App,
    ) -> Option<gpui::SharedString> {
        (id == "C1").then(|| "general".into())
    }
}

#[test]
fn audit_log() {
    use gpui::AppContext as _;
    use std::sync::Arc;
    use store::{AppStore as _, AuditEntry, AuditOutcome, MemoryAppStore, WriteDecision};
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let store = Arc::new(MemoryAppStore::default());
        let base = scenario::NOW_SECS as i64 * 1000;
        let rows = [
            (
                "get_unreads",
                Some("W1"),
                None,
                AuditOutcome::Ok,
                None,
                8,
                None,
            ),
            (
                "send_message",
                Some("W1"),
                Some("C1"),
                AuditOutcome::Ok,
                Some(WriteDecision::Confirmed),
                1450,
                Some("Deploy finished"),
            ),
            (
                "send_message",
                Some("W1"),
                Some("C1"),
                AuditOutcome::Declined,
                Some(WriteDecision::Declined),
                9200,
                None,
            ),
            (
                "search_messages",
                Some("W9"),
                None,
                AuditOutcome::Denied,
                None,
                1,
                None,
            ),
            (
                "get_unreads",
                None,
                None,
                AuditOutcome::RateLimited,
                None,
                0,
                None,
            ),
        ];
        for (i, (tool, ws, target, outcome, decision, ms, content)) in rows.into_iter().enumerate()
        {
            futures::executor::block_on(store.audit_append(AuditEntry {
                at_ms: base - (rows_len() - i as i64) * 61_000,
                client: Arc::from("claude-code"),
                peer_pid: Some(4242),
                tool: Arc::from(tool),
                workspace: ws.map(|w| settings::WorkspaceKey::new("fake", w)),
                target: target.map(Arc::from),
                outcome,
                detail: None,
                decision,
                duration_ms: ms,
                content: content.map(Arc::from),
            }))
            .unwrap();
        }
        let s: Arc<dyn store::AppStore> = store;
        let w = h.open(move |_, cx| {
            cx.new(|cx| components::AuditLogView::new(s, std::rc::Rc::new(StageNames), cx))
        });
        let img = h.capture(w.into()).unwrap();
        assert_matches(SUITE, "audit_log", &v, &img, Tolerance::default());
    }
}

fn rows_len() -> i64 {
    5
}

#[test]
fn command_palette_open() {
    use components::CommandItem;
    use gpui::SharedString;
    let items = || {
        vec![
            CommandItem {
                name: SharedString::from("workspace::QuickSwitch"),
                title: SharedString::from("Quick switch"),
                binding: Some(SharedString::from("cmd-k")),
            },
            CommandItem {
                name: SharedString::from("app::OpenSettings"),
                title: SharedString::from("Open settings"),
                binding: Some(SharedString::from("cmd-,")),
            },
            CommandItem {
                name: SharedString::from("theme::ToggleMode"),
                title: SharedString::from("Toggle light and dark mode"),
                binding: Some(SharedString::from("cmd-shift-l")),
            },
            CommandItem {
                name: SharedString::from("workspace::Search"),
                title: SharedString::from("Search"),
                binding: Some(SharedString::from("cmd-shift-f")),
            },
            CommandItem {
                name: SharedString::from("workspace::GoBack"),
                title: SharedString::from("Go back"),
                binding: Some(SharedString::from("cmd-[")),
            },
            CommandItem {
                name: SharedString::from("workspace::GoForward"),
                title: SharedString::from("Go forward"),
                binding: Some(SharedString::from("cmd-]")),
            },
            CommandItem {
                name: SharedString::from("store::RebuildCache"),
                title: SharedString::from("Rebuild cache"),
                binding: None,
            },
        ]
    };
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let s = scenario::by_name("rail_multi").unwrap();
        let img = stage::capture(
            &mut h,
            s,
            StageOptions {
                command_palette: Some((items(), String::new())),
                ..Default::default()
            },
        )
        .unwrap();
        assert_matches(
            SUITE,
            "command_palette_open",
            &v,
            &img,
            Tolerance::default(),
        );
    }
}

/// Every icon of the UI set in a grid, each at the default size in the theme's
/// muted text colour, on the app background.
struct IconsGallery;

impl gpui::Render for IconsGallery {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement as _, Styled as _, div};
        use themes::metrics::{ICON_SIZE_LG, SPACE_6, SPACE_8};
        let colors = *cx.theme().colors();
        div()
            .size_full()
            .flex()
            .flex_wrap()
            .content_start()
            .gap(SPACE_8)
            .p(SPACE_8)
            .bg(colors.background)
            .children(components::IconName::ALL.iter().map(|&name| {
                div()
                    .flex()
                    .items_center()
                    .justify_center()
                    .size(ICON_SIZE_LG * 2.0)
                    .rounded(themes::metrics::RADIUS_SM)
                    .border_1()
                    .border_color(colors.border)
                    .bg(colors.surface_background)
                    .p(SPACE_6)
                    .child(components::Icon::new(name).size(ICON_SIZE_LG))
            }))
    }
}

#[test]
fn icons_gallery() {
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let w = h.open(|_, app| app.new(|_| IconsGallery));
        let img = h.capture(w.into()).unwrap();
        assert_matches(SUITE, "icons_gallery", &v, &img, Tolerance::default());
    }
}

/// The About window at its own size, with a fixed version so a release does not
/// move the golden.
#[test]
fn about_window() {
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let w = h.open_sized(
            themes::metrics::LAYOUT_WINDOW_ABOUT_WIDTH.into(),
            themes::metrics::LAYOUT_WINDOW_ABOUT_HEIGHT.into(),
            |_, app| app.new(|_| parla::about::view(Vec::new(), "1.2.3")),
        );
        let img = h.capture(w.into()).unwrap();
        assert_matches(SUITE, "about_window", &v, &img, Tolerance::default());
    }
}

/// The Settings window on the General page, at its own size, with a fixed
/// version and settings path so a release or a machine does not move the golden.
fn settings_general_capture(v: &Variant, name: &str, pseudo: bool) {
    settings_capture(v, name, pseudo, parla::settings_window::Page::General, None);
}

/// A settings file the Settings window reads through the store, for a page that
/// shows saved values (the board's keywords and quiet hours).
const SETTINGS_SEED_NOTIFICATIONS: &str = r#"{
  "notifications": {
    "desktop": "mentions",
    "keywords": ["release", "tokens", "design review", "on-call"],
    "quiet_hours": { "from": "20:00", "to": "08:00" }
  }
}"#;

fn settings_capture(
    v: &Variant,
    name: &str,
    pseudo: bool,
    page: parla::settings_window::Page,
    seed: Option<&str>,
) {
    settings_capture_with(v, name, pseudo, page, seed, |_| {});
}

/// [`settings_capture`] after `setup` ran on the app (a page that lists
/// workspaces installs a manager).
fn settings_capture_with(
    v: &Variant,
    name: &str,
    pseudo: bool,
    page: parla::settings_window::Page,
    seed: Option<&str>,
    setup: impl FnOnce(&mut gpui::App),
) {
    // The pseudo-locale golden uses the window's minimum size: it is the case
    // that clips, and it keeps the PNG under the 200 KB baseline limit.
    let (width, height) = if pseudo {
        (
            themes::metrics::LAYOUT_WINDOW_SETTINGS_MIN_WIDTH,
            themes::metrics::LAYOUT_WINDOW_SETTINGS_MIN_HEIGHT,
        )
    } else {
        (
            themes::metrics::LAYOUT_WINDOW_SETTINGS_WIDTH,
            themes::metrics::LAYOUT_WINDOW_SETTINGS_HEIGHT,
        )
    };
    let mut h = UiHarness::with_catalogs(*v, &[parla::catalog()]).unwrap();
    if pseudo {
        h.app().update(|cx| {
            let xa: i18n::LanguageIdentifier = i18n::PSEUDO.parse().unwrap();
            assert!(i18n::gpui::set_pref_with_os(
                cx,
                i18n::LocalePref::Code(xa),
                &[]
            ));
        });
    }
    h.app().update(setup);
    let dir = tempfile::tempdir().unwrap();
    if let Some(text) = seed {
        let paths = settings::ConfigPaths::at(dir.path());
        std::fs::write(&paths.settings, text).unwrap();
        h.app()
            .update(|cx| cx.set_global(settings::SettingsStore::load(paths)));
    }
    let w = h.open_sized(width.into(), height.into(), move |_, app| {
        app.new(|_| {
            parla::settings_window::view_on(
                std::path::PathBuf::from("/example/parla/settings.json"),
                "0.4.0".into(),
                page,
            )
        })
    });
    let img = h.capture(w.into()).unwrap();
    assert_matches(SUITE, name, v, &img, Tolerance::default());
}

/// The Appearance page at the Settings window size (defaults: System mode,
/// Parla Light and Parla Dark cards, compact density).
#[test]
fn settings_appearance() {
    for v in Variant::defaults() {
        settings_capture(
            &v,
            "settings_appearance",
            false,
            parla::settings_window::Page::Appearance,
            None,
        );
    }
}

/// The Appearance page under "Increase contrast": the high contrast cards
/// are marked, the accent presets are disabled with their hint, and the
/// keyboard focus ring sits on the first control (one Tab).
#[test]
fn settings_appearance_high_contrast() {
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        h.app().update(|cx| {
            let mut s = settings::Settings::default();
            s.theme.mode = match v.appearance {
                ui_testing::Appearance::Light => settings::ThemeMode::Light,
                ui_testing::Appearance::Dark => settings::ThemeMode::Dark,
            };
            themes::set_increase_contrast(cx, true);
            themes::apply(cx, &s);
        });
        let w = h.open_sized(
            themes::metrics::LAYOUT_WINDOW_SETTINGS_WIDTH.into(),
            themes::metrics::LAYOUT_WINDOW_SETTINGS_HEIGHT.into(),
            move |_, app| {
                app.new(|_| {
                    parla::settings_window::view_on(
                        std::path::PathBuf::from("/example/parla/settings.json"),
                        "0.4.0".into(),
                        parla::settings_window::Page::Appearance,
                    )
                })
            },
        );
        h.app()
            .update_window(w.into(), |_, window, cx| {
                window.dispatch_keystroke(gpui::Keystroke::parse("tab").unwrap(), cx);
            })
            .unwrap();
        let img = h.capture(w.into()).unwrap();
        assert_matches(
            SUITE,
            "settings_appearance_high_contrast",
            &v,
            &img,
            Tolerance::default(),
        );
    }
}

/// The Notifications page at the Settings window size, with keywords and
/// quiet hours set.
#[test]
fn settings_notifications() {
    for v in Variant::defaults() {
        settings_capture(
            &v,
            "settings_notifications",
            false,
            parla::settings_window::Page::Notifications,
            Some(SETTINGS_SEED_NOTIFICATIONS),
        );
    }
}

/// The MCP page with the server on and one workspace shared with writes.
#[test]
fn settings_mcp() {
    for v in Variant::defaults() {
        settings_capture(
            &v,
            "settings_mcp",
            false,
            parla::settings_window::Page::Mcp,
            Some(
                r#"{ "mcp": { "enabled": true, "workspaces": {
                    "fake/W1": { "enabled": true, "allow_writes": true } } } }"#,
            ),
        );
    }
}

/// The Calls page with its defaults (ring on, Parla chime, Parla audio, muted).
#[test]
fn settings_calls() {
    for v in Variant::defaults() {
        settings_capture(
            &v,
            "settings_calls",
            false,
            parla::settings_window::Page::Calls,
            None,
        );
    }
}

/// The Calls page with the microphone select open: the list floats over the
/// Speaker row below it instead of pushing it down.
#[test]
fn settings_calls_select_open() {
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        h.app().update(|cx| {
            cx.set_global(parla::call_manager::DeviceLister(std::sync::Arc::new(
                || {
                    (
                        vec!["Built-in Microphone".into(), "USB Microphone".into()],
                        vec!["Built-in Speakers".into()],
                    )
                },
            )))
        });
        let w = h.open_sized(
            themes::metrics::LAYOUT_WINDOW_SETTINGS_WIDTH.into(),
            themes::metrics::LAYOUT_WINDOW_SETTINGS_HEIGHT.into(),
            |_, app| {
                app.new(|_| {
                    parla::settings_window::view_on(
                        std::path::PathBuf::from("/example/parla/settings.json"),
                        "0.4.0".into(),
                        parla::settings_window::Page::Calls,
                    )
                })
            },
        );
        h.app()
            .update(|cx| w.update(cx, |v, _, cx| v.open_input_select(cx)))
            .unwrap();
        let img = h.capture(w.into()).unwrap();
        assert_matches(
            SUITE,
            "settings_calls_select_open",
            &v,
            &img,
            Tolerance::default(),
        );
    }
}

/// The Workspaces and privacy page over two signed-in workspaces whose
/// sessions have not started (Sign in again shows).
#[test]
fn settings_accounts() {
    for v in Variant::defaults() {
        settings_capture_with(
            &v,
            "settings_accounts",
            false,
            parla::settings_window::Page::Workspaces,
            None,
            |cx| {
                use gpui::AppContext as _;
                let factory = domain::test_support::FakeFactory::default();
                factory.set_accounts(
                    [("W1", "Northwind Studio"), ("W2", "Fieldwork")]
                        .into_iter()
                        .map(|(id, name)| domain::AccountInfo {
                            key: settings::WorkspaceKey::new("fake", id),
                            name: std::sync::Arc::from(name),
                            org: None,
                            icon: None,
                        })
                        .collect(),
                );
                let mut registry = parla::providers::ProviderRegistry::new();
                registry.add(factory);
                let store: std::sync::Arc<dyn store::Store> =
                    std::sync::Arc::new(store::MemoryStore::default());
                let w = cx.new(|cx| parla::workspaces::Workspaces::new(&registry, store, cx));
                cx.set_global(parla::workspaces::WorkspacesHandle(w));
            },
        );
    }
}

/// The Advanced page with its defaults and the danger zone.
#[test]
fn settings_advanced() {
    for v in Variant::defaults() {
        settings_capture(
            &v,
            "settings_advanced",
            false,
            parla::settings_window::Page::Advanced,
            None,
        );
    }
}

/// The Keymap page at the Settings window size: the defaults of this platform.
#[test]
fn settings_keymap() {
    for v in Variant::defaults() {
        settings_capture(
            &v,
            "settings_keymap",
            false,
            parla::settings_window::Page::Keymap,
            None,
        );
    }
}

/// The Keymap page with one binding of the user's (Reset) and a row recording
/// keys that another action already uses (the conflict banner).
#[test]
fn settings_keymap_recording() {
    let mac = cfg!(target_os = "macos");
    let (user_keys, taken) = if mac {
        ("cmd-alt-d", "cmd-k")
    } else {
        ("ctrl-alt-d", "ctrl-k")
    };
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        h.app().update(|cx| {
            cx.set_global(parla::shortcuts::LiveKeymap {
                user: format!(
                    r#"[{{ "bindings": {{ "{user_keys}": "workspace::PauseNotifications" }} }}]"#
                ),
            })
        });
        let w = h.open_sized(
            themes::metrics::LAYOUT_WINDOW_SETTINGS_WIDTH.into(),
            themes::metrics::LAYOUT_WINDOW_SETTINGS_MIN_HEIGHT.into(),
            move |window, app| {
                let view = app.new(|_| {
                    parla::settings_window::view_on(
                        std::path::PathBuf::from("/example/parla/settings.json"),
                        "0.4.0".into(),
                        parla::settings_window::Page::Keymap,
                    )
                });
                view.update(app, |v, cx| {
                    v.begin_recording("workspace::SetStatus", window, cx);
                    v.recorded(taken.to_string(), cx);
                });
                view
            },
        );
        let img = h.capture(w.into()).unwrap();
        assert_matches(
            SUITE,
            "settings_keymap_recording",
            &v,
            &img,
            Tolerance::default(),
        );
    }
}

/// The keyboard shortcuts sheet over the app ground, from this platform's
/// default keymap.
struct ShortcutsSheetStage(gpui::Entity<components::shortcuts_sheet::ShortcutsSheet>);

impl gpui::Render for ShortcutsSheetStage {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement as _, Styled as _, div};
        div()
            .flex()
            .justify_center()
            .size_full()
            .p(themes::metrics::SPACE_6)
            .bg(cx.theme().colors().background)
            .child(self.0.clone())
    }
}

#[test]
fn shortcuts_sheet() {
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let w = h.open_sized(
            themes::metrics::LAYOUT_WINDOW_WIDTH.into(),
            themes::metrics::LAYOUT_WINDOW_MIN_HEIGHT.into(),
            |_, app| {
                let mac = cfg!(target_os = "macos");
                let rows = parla::shortcuts::shortcuts(parla::shortcuts::DEFAULT_KEYMAP, "");
                let l = i18n::gpui::localizer(app);
                let groups = parla::shortcuts::sheet_groups(&l, &rows, mac);
                let open = if mac { "cmd-/" } else { "ctrl-/" };
                let sheet = app.new(|cx| {
                    components::shortcuts_sheet::ShortcutsSheet::new(groups, open, mac, cx)
                });
                app.new(|_| ShortcutsSheetStage(sheet))
            },
        );
        let img = h.capture(w.into()).unwrap();
        assert_matches(SUITE, "shortcuts_sheet", &v, &img, Tolerance::default());
    }
}

#[test]
fn settings_general() {
    for v in Variant::defaults() {
        settings_general_capture(&v, "settings_general", false);
    }
}

#[test]
fn settings_general_en_xa() {
    for v in Variant::defaults() {
        settings_general_capture(&v, "settings_general_en_xa", true);
    }
}

/// The thread pop-out window at its own size: the window header over the
/// thread panel, without the panel's own header.
#[test]
fn popout_thread() {
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let s = scenario::by_name("popout_thread").unwrap();
        let opts = StageOptions {
            popout: true,
            banner: Some(Box::new(|_, cx| -> AnyElement {
                parla::popout::thread_header("general \u{b7} Example Workspace", cx)
                    .into_any_element()
            })),
            ..Default::default()
        };
        let w = h.open_sized(
            themes::metrics::LAYOUT_WINDOW_POPOUT_WIDTH.into(),
            themes::metrics::LAYOUT_WINDOW_POPOUT_HEIGHT.into(),
            move |window, app| app.new(|cx| stage::Stage::new(&s, opts, window, cx)),
        );
        let img = h.capture(w.into()).unwrap();
        assert_matches(SUITE, "popout_thread", &v, &img, Tolerance::default());
    }
}

/// Serves the media viewer's images: the original of each file is a mock
/// screenshot (dark sidebar beside a light page), a rendition is a flat swatch.
struct ViewerFetcher;

/// The originals never arrive (the loading state).
struct StalledFetcher;

impl components::media::MediaFetcher for StalledFetcher {
    fn fetch(
        &self,
        src: &domain::ImageRef,
        max: u64,
    ) -> futures::future::BoxFuture<'static, Result<Vec<u8>, components::media::MediaError>> {
        if matches!(src, domain::ImageRef::Provider(u) if u.0.ends_with("/dl")) {
            return Box::pin(futures::future::pending());
        }
        components::media::MediaFetcher::fetch(&ViewerFetcher, src, max)
    }
}

/// The originals fail (the failed card).
struct FailingFetcher;

impl components::media::MediaFetcher for FailingFetcher {
    fn fetch(
        &self,
        src: &domain::ImageRef,
        max: u64,
    ) -> futures::future::BoxFuture<'static, Result<Vec<u8>, components::media::MediaError>> {
        if matches!(src, domain::ImageRef::Provider(u) if u.0.ends_with("/dl")) {
            return Box::pin(futures::future::ready(Err(
                components::media::MediaError::Refused,
            )));
        }
        components::media::MediaFetcher::fetch(&ViewerFetcher, src, max)
    }
}

#[derive(Clone, Copy, PartialEq)]
enum ViewerShot {
    Image,
    Zoomed,
    Loading,
    Failed,
    Pdf,
}

const SWATCHES: [[u8; 4]; 8] = [
    [46, 42, 102, 255],
    [255, 166, 17, 255],
    [23, 22, 47, 255],
    [254, 97, 41, 255],
    [31, 122, 85, 255],
    [241, 239, 236, 255],
    [253, 52, 89, 255],
    [58, 58, 120, 255],
];

fn viewer_png(img: image::RgbaImage) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).unwrap();
    out.into_inner()
}

fn mock_screenshot() -> image::RgbaImage {
    image::RgbaImage::from_fn(1440, 900, |x, y| {
        let px = |c: [u8; 3]| image::Rgba([c[0], c[1], c[2], 255]);
        if x < 380 {
            let row = (y / 60, y % 60);
            if (1..8).contains(&row.0) && row.1 < 18 && x > 28 && x < 28 + 40 * (9 - row.0) {
                px([143, 140, 196])
            } else {
                px([23, 22, 47])
            }
        } else if (440..900).contains(&x) && (80..300).contains(&y) {
            px([23, 22, 47])
        } else if (940..1380).contains(&x) && (80..300).contains(&y) {
            px([33, 30, 76])
        } else if (440..1380).contains(&x) && (360..380).contains(&y) {
            px([211, 207, 200])
        } else {
            px([248, 247, 245])
        }
    })
}

impl components::media::MediaFetcher for ViewerFetcher {
    fn fetch(
        &self,
        src: &domain::ImageRef,
        _: u64,
    ) -> futures::future::BoxFuture<'static, Result<Vec<u8>, components::media::MediaError>> {
        let domain::ImageRef::Provider(u) = src else {
            return Box::pin(futures::future::ready(Err(
                components::media::MediaError::Refused,
            )));
        };
        let u = u.0.to_string();
        let bytes = if u.ends_with("/dl") {
            viewer_png(mock_screenshot())
        } else {
            let n: usize = u
                .chars()
                .filter(|c| c.is_ascii_digit())
                .collect::<String>()
                .parse()
                .unwrap_or(0);
            viewer_png(image::RgbaImage::from_pixel(
                128,
                88,
                image::Rgba(SWATCHES[n % SWATCHES.len()]),
            ))
        };
        Box::pin(futures::future::ready(Ok(bytes)))
    }
}

fn viewer_gallery() -> components::media_viewer::Gallery {
    use std::sync::Arc;
    let items = (0..8)
        .map(|i| {
            let url = |s: String| domain::RemoteUrl(Arc::from(s));
            components::media_viewer::ViewerItem {
                file: domain::FileRef {
                    id: domain::FileId(Arc::from(format!("F{i}"))),
                    name: Arc::from(if i == 2 {
                        "sidebar-contrast-study.png".to_string()
                    } else {
                        format!("shot-{i}.png")
                    }),
                    title: None,
                    mime: Arc::from("image/png"),
                    size: 1_258_291,
                    kind: domain::FileKind::Image,
                    thumbs: Arc::from([domain::Rendition {
                        url: url(format!("https://files.example/t{i}")),
                        width: 360,
                        height: 225,
                    }]),
                    download: Some(url(format!("https://files.example/{i}/dl"))),
                    preview: None,
                    video: None,
                    tombstone: false,
                    lines: None,
                    ext: domain::ProviderExt::none(),
                },
                message: domain::test_support::msg("C1", &format!("1763030640.00000{i}"), "").id,
                sender: "Aiko Tanaka".into(),
            }
        })
        .collect();
    components::media_viewer::Gallery::new(items, 2).with_place(Some("design-system".into()))
}

fn pdf_gallery() -> components::media_viewer::Gallery {
    let mut g = viewer_gallery();
    let mut item = g.current().unwrap().clone();
    item.file.name = std::sync::Arc::from("brand-guide.pdf");
    item.file.mime = std::sync::Arc::from("application/pdf");
    item.file.kind = domain::FileKind::Pdf;
    item.file.thumbs = std::sync::Arc::from([]);
    g = components::media_viewer::Gallery::new(vec![item], 0)
        .with_place(Some("design-system".into()));
    g
}

/// The media viewer window at its own size on the third of eight images:
/// fitted, at actual size and panned, loading, failed, or a PDF's card.
fn media_viewer_capture(name: &str, shot: ViewerShot) {
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let disk = dir.path().to_path_buf();
        // No workspace image cache: the strip loads through the viewer's own fetcher.
        struct NoHost;
        impl parla::media_viewer::ViewerHost for NoHost {
            fn download(&self, _: &domain::FileRef, _: &mut gpui::App) {}
            fn open_external(&self, _: &domain::FileRef, _: &mut gpui::App) {}
        }
        let w = h.open_sized(
            themes::metrics::LAYOUT_WINDOW_MEDIA_WIDTH.into(),
            themes::metrics::LAYOUT_WINDOW_MEDIA_HEIGHT.into(),
            move |_, app| {
                app.new(|cx| {
                    let fetcher: std::sync::Arc<dyn components::media::MediaFetcher> = match shot {
                        ViewerShot::Loading => std::sync::Arc::new(StalledFetcher),
                        ViewerShot::Failed => std::sync::Arc::new(FailingFetcher),
                        _ => std::sync::Arc::new(ViewerFetcher),
                    };
                    let gallery = if shot == ViewerShot::Pdf {
                        pdf_gallery()
                    } else {
                        viewer_gallery()
                    };
                    parla::media_viewer::MediaViewer::new(
                        gallery,
                        components::media::ViewerCache::new(fetcher, disk.join("viewer")),
                        std::rc::Rc::new(NoHost),
                        cx,
                    )
                })
            },
        );
        h.settle();
        if shot == ViewerShot::Zoomed {
            w.update(h.app(), |viewer, window, cx| {
                viewer.actual_size(window, cx);
                viewer.pan_for_tests((240.0, 120.0), window, cx);
            })
            .unwrap();
        }
        let img = h.capture(w.into()).unwrap();
        assert_matches(SUITE, name, &v, &img, Tolerance::default());
    }
}

#[test]
fn media_viewer_image() {
    media_viewer_capture("media_viewer_image", ViewerShot::Image);
}

#[test]
fn media_viewer_zoomed() {
    media_viewer_capture("media_viewer_zoomed", ViewerShot::Zoomed);
}

#[test]
fn media_viewer_loading() {
    media_viewer_capture("media_viewer_loading", ViewerShot::Loading);
}

#[test]
fn media_viewer_failed() {
    media_viewer_capture("media_viewer_failed", ViewerShot::Failed);
}

#[test]
fn media_viewer_pdf() {
    media_viewer_capture("media_viewer_pdf", ViewerShot::Pdf);
}

/// A neutral sign-in for the first-run screens (they never run it).
struct ExampleProviderSignIn;

impl components::sign_in::SignIn for ExampleProviderSignIn {
    fn provider_name(&self) -> String {
        "Example Chat".into()
    }
    fn notice(&self) -> Vec<components::sign_in::SignInText> {
        Vec::new()
    }
    fn options(&self) -> Vec<components::sign_in::SignInOption> {
        Vec::new()
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
        gpui::Task::ready(Ok(
            components::sign_in::SignInResult::Workspaces(Vec::new()),
        ))
    }
    fn sign_out(
        &self,
        _: &settings::WorkspaceKey,
        _: &mut gpui::App,
    ) -> gpui::Task<Result<(), String>> {
        gpui::Task::ready(Ok(()))
    }
}

/// A first-run screen at the main window's size, `setup` moving it to its step.
fn onboarding_capture(
    name: &str,
    setup: impl Fn(
        &mut components::onboarding::Onboarding,
        &mut gpui::Window,
        &mut gpui::Context<components::onboarding::Onboarding>,
    ) + Copy
    + 'static,
) {
    onboarding_capture_sized(name, themes::metrics::LAYOUT_WINDOW_HEIGHT, setup);
}

/// [`onboarding_capture`] in a window of this height (a state that needs less
/// of the screen keeps its golden small).
fn onboarding_capture_sized(
    name: &str,
    height: gpui::Pixels,
    setup: impl Fn(
        &mut components::onboarding::Onboarding,
        &mut gpui::Window,
        &mut gpui::Context<components::onboarding::Onboarding>,
    ) + Copy
    + 'static,
) {
    use components::onboarding::{Onboarding, ProviderCard};
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let w = h.open_sized(
            themes::metrics::LAYOUT_WINDOW_WIDTH.into(),
            height.into(),
            move |window, app| {
                app.new(|cx| {
                    let screen = cx.new(|cx| {
                        components::sign_in::SignInScreen::new(
                            std::rc::Rc::new(ExampleProviderSignIn),
                            cx,
                        )
                    });
                    let providers = vec![ProviderCard {
                        name: "Example Chat".into(),
                    }];
                    let mut ob = Onboarding::new(providers, screen, cx);
                    setup(&mut ob, window, cx);
                    ob
                })
            },
        );
        let img = h.capture(w.into()).unwrap();
        assert_matches(SUITE, name, &v, &img, Tolerance::default());
    }
}

#[test]
fn onboarding_welcome() {
    onboarding_capture("onboarding_welcome", |_, _, _| {});
}

#[test]
fn onboarding_provider() {
    onboarding_capture("onboarding_provider", |ob, _, cx| ob.next(cx));
}

/// Welcome with the language select open (the floating list).
#[test]
fn onboarding_language_open() {
    onboarding_capture_sized(
        "onboarding_language_open",
        themes::metrics::LAYOUT_WINDOW_MIN_HEIGHT,
        |ob, _, cx| ob.open_language(cx),
    );
}

/// Part way: the first workspace has its directory and is catching up, the
/// second waits.
#[test]
fn onboarding_first_sync() {
    use components::onboarding::{SyncProgress, WorkspaceSync};
    onboarding_capture("onboarding_first_sync", |ob, _, cx| {
        ob.start_first_sync(vec!["Example Workspace".into(), "Fieldwork".into()], cx);
        ob.set_sync(
            vec![
                WorkspaceSync {
                    name: "Example Workspace".into(),
                    progress: SyncProgress {
                        store_ready: true,
                        directory: Some((38, 214)),
                        catch_up: Some((23, 61)),
                        failed: false,
                    },
                },
                WorkspaceSync {
                    name: "Fieldwork".into(),
                    progress: SyncProgress {
                        store_ready: true,
                        ..Default::default()
                    },
                },
            ],
            cx,
        );
    });
}

/// Welcome sliding out and the provider picker sliding in, part way through
/// the stack's base duration.
#[test]
fn onboarding_transition() {
    use components::onboarding::{Onboarding, ProviderCard};
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        h.app()
            .update(|app| components::set_motion(app, components::Motion::Live));
        let built: std::rc::Rc<std::cell::RefCell<Option<gpui::Entity<Onboarding>>>> =
            std::rc::Rc::default();
        let built_by_window = built.clone();
        let w = h.open_sized(
            themes::metrics::LAYOUT_WINDOW_WIDTH.into(),
            themes::metrics::LAYOUT_WINDOW_HEIGHT.into(),
            move |_, app| {
                let ob = app.new(|cx| {
                    let screen = cx.new(|cx| {
                        components::sign_in::SignInScreen::new(
                            std::rc::Rc::new(ExampleProviderSignIn),
                            cx,
                        )
                    });
                    let providers = vec![ProviderCard {
                        name: "Example Chat".into(),
                    }];
                    Onboarding::new(providers, screen, cx)
                });
                *built_by_window.borrow_mut() = Some(ob.clone());
                ob
            },
        );
        // Welcome renders present first, so pushing the provider has an exit
        // to run. The draw after the push starts the transition clock.
        let _ = h.capture_frame(w.into()).unwrap();
        let ob = built.borrow().clone().expect("the onboarding view");
        h.app().update(|app| ob.update(app, |ob, cx| ob.next(cx)));
        let _ = h.capture_frame(w.into()).unwrap();
        h.advance_clock(themes::metrics::DURATION_BASE / 4);
        let img = h.capture_frame(w.into()).unwrap();
        assert_matches(
            SUITE,
            "onboarding_transition",
            &v,
            &img,
            Tolerance::default(),
        );
    }
}

/// The tour's second tip (the workspace rail) over a workspace.
struct TourStage {
    stage: gpui::Entity<stage::Stage>,
    tour: gpui::Entity<components::tour::Tour>,
}

impl gpui::Render for TourStage {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement as _, Styled as _, div};
        div()
            .relative()
            .size_full()
            .child(self.stage.clone())
            .child(self.tour.clone())
    }
}

/// The tour's first tip (the sidebar) with a config banner under the columns:
/// the spotlight ends where the sidebar does.
struct TourBannerStage {
    stage: gpui::Entity<stage::Stage>,
    tour: gpui::Entity<components::tour::Tour>,
}

impl gpui::Render for TourBannerStage {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        cx: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement as _, Styled as _, div};
        let colors = *cx.theme().colors();
        div()
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .child(div().flex_1().min_h_0().child(self.stage.clone()))
            .child(parla::root_view::error_strip(
                &colors,
                themes::parla_tokens(cx.theme()).danger_soft,
                "settings.toml:3:5: expected a boolean",
            ))
            .child(self.tour.clone())
    }
}

#[test]
fn onboarding_tour_with_banner() {
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let w = h.open(|window, app| {
            let targets = components::tour::TourTargets::default();
            app.set_global(targets.clone());
            let s = scenario::by_name("rail_and_header").expect("scenario");
            let st = app.new(|cx| stage::Stage::new(&s, StageOptions::default(), window, cx));
            let tour = app.new(|cx| {
                components::tour::Tour::new(components::tour::TourLayout::default(), cx)
                    .with_targets(targets, cx)
            });
            app.new(|_| TourBannerStage { stage: st, tour })
        });
        let img = h.capture(w.into()).unwrap();
        assert_matches(
            SUITE,
            "onboarding_tour_banner",
            &v,
            &img,
            Tolerance::default(),
        );
    }
}

#[test]
fn onboarding_tour() {
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let w = h.open(|window, app| {
            let s = scenario::by_name("rail_and_header").expect("scenario");
            let st = app.new(|cx| stage::Stage::new(&s, StageOptions::default(), window, cx));
            let keys = components::tour::TourKeys {
                workspace_first: "⌘1".into(),
                workspace_last: "⌘9".into(),
                switcher: "⌘K".into(),
                shortcuts: "⌘/".into(),
            };
            let tour = app.new(|cx| {
                let mut t =
                    components::tour::Tour::new(components::tour::TourLayout::default(), cx)
                        .with_keys(keys);
                t.next(cx);
                t
            });
            app.new(|_| TourStage { stage: st, tour })
        });
        let img = h.capture(w.into()).unwrap();
        assert_matches(SUITE, "onboarding_tour", &v, &img, Tolerance::default());
    }
}

/// The lock screen the idle lock (`storage.lock_after_minutes`) puts over a
/// window: title, what happened and Unlock.
#[test]
fn lock_screen() {
    struct Locked(Option<gpui::FocusHandle>);
    impl gpui::Render for Locked {
        fn render(
            &mut self,
            window: &mut gpui::Window,
            cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            let focus = self.0.get_or_insert_with(|| cx.focus_handle()).clone();
            parla::idle_lock::cover(&focus, window, cx)
        }
    }
    for pseudo in [false, true] {
        let name = if pseudo {
            "lock_screen_en_xa"
        } else {
            "lock_screen"
        };
        for v in Variant::defaults() {
            let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
            if pseudo {
                h.app().update(|cx| {
                    let xa: i18n::LanguageIdentifier = i18n::PSEUDO.parse().unwrap();
                    assert!(i18n::gpui::set_pref_with_os(
                        cx,
                        i18n::LocalePref::Code(xa),
                        &[]
                    ));
                });
            }
            // The pseudo-locale golden uses the smallest Settings window: the
            // case that wraps.
            let (width, height) = if pseudo {
                (
                    themes::metrics::LAYOUT_WINDOW_SETTINGS_MIN_WIDTH,
                    themes::metrics::LAYOUT_WINDOW_SETTINGS_MIN_HEIGHT,
                )
            } else {
                (
                    themes::metrics::LAYOUT_WINDOW_SETTINGS_WIDTH,
                    themes::metrics::LAYOUT_WINDOW_SETTINGS_HEIGHT,
                )
            };
            let w = h.open_sized(width.into(), height.into(), |_, app| {
                app.new(|_| Locked(None))
            });
            let img = h.capture(w.into()).unwrap();
            assert_matches(SUITE, name, &v, &img, Tolerance::default());
        }
    }
}

/// A synthetic Rust file for the code viewer baseline.
const CODE_VIEWER_TEXT: &str = r#"//! Loads theme families from JSON in the documented shape.

use serde::Deserialize;
use crate::model::{Appearance, Theme, ThemeFamily};

/// Parse a family; unknown keys are ignored, bad colours dropped.
pub fn load(json: &str) -> Result<ThemeFamily, Error> {
    let raw: RawFamily = serde_json::from_str(json)?;
    Ok(raw.into_family())
}

impl RawFamily {
    fn into_family(self) -> ThemeFamily {
        let themes = self.themes.into_iter()
            .filter_map(|t| t.resolve().ok())
            .collect();
        ThemeFamily { name: self.name, themes }
    }
}

// Fallbacks follow the active theme's appearance.
fn fallback(a: Appearance) -> &'static str {
    match a { Appearance::Light => "Parla Light", Appearance::Dark => "Parla Dark" }
}

const MAX_BYTES: usize = 256 * 1024;
"#;

/// The code viewer window at its own size on a Rust file, with find open on
/// three matches (the second current).
#[test]
fn code_viewer() {
    code_viewer_capture("code_viewer", false);
}

/// The code viewer at its minimum width in the `en-XA` pseudo-locale, so the
/// header, toolbar, find bar and status bar fit 40% longer text.
#[test]
fn code_viewer_en_xa() {
    code_viewer_capture("code_viewer_en_xa", true);
}

fn code_viewer_capture(name: &str, narrow_pseudo: bool) {
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        if narrow_pseudo {
            h.app().update(|cx| {
                let xa: i18n::LanguageIdentifier = i18n::PSEUDO.parse().unwrap();
                assert!(i18n::gpui::set_pref_with_os(
                    cx,
                    i18n::LocalePref::Code(xa),
                    &[]
                ));
            });
        }
        let (width, height) = if narrow_pseudo {
            (
                themes::metrics::LAYOUT_WINDOW_MEDIA_MIN_WIDTH,
                themes::metrics::LAYOUT_WINDOW_MEDIA_MIN_HEIGHT,
            )
        } else {
            (
                themes::metrics::LAYOUT_WINDOW_MEDIA_WIDTH,
                themes::metrics::LAYOUT_WINDOW_MEDIA_HEIGHT,
            )
        };
        struct NoHost;
        impl parla::code_viewer::CodeHost for NoHost {
            fn download(&self, _: &domain::FileRef, _: &mut gpui::App) {}
        }
        let w = h.open_sized(width.into(), height.into(), move |window, app| {
            app.new(|cx| {
                let mut g = viewer_gallery();
                let mut item = g.current().unwrap().clone();
                item.file.name = std::sync::Arc::from("theme_loader.rs");
                item.file.mime = std::sync::Arc::from("text/plain");
                item.file.kind = domain::FileKind::Snippet;
                item.file.size = CODE_VIEWER_TEXT.len() as u64;
                item.file.preview = Some(std::sync::Arc::from(CODE_VIEWER_TEXT));
                g = components::media_viewer::Gallery::new(vec![item], 0)
                    .with_place(Some("eng-desktop".into()));
                parla::code_viewer::CodeViewer::new(g, None, std::rc::Rc::new(NoHost), window, cx)
            })
        });
        h.settle();
        w.update(h.app(), |viewer, window, cx| {
            viewer.find_text("theme", window, cx)
        })
        .unwrap();
        // The scan runs off the UI thread; step once it has landed.
        h.settle();
        w.update(h.app(), |viewer, _, cx| viewer.find_next(cx))
            .unwrap();
        h.settle();
        let img = h.capture(w.into()).unwrap();
        assert_matches(SUITE, name, &v, &img, Tolerance::default());
    }
}

/// A picture avatar is clipped by its own corner radius. GPUI's
/// `overflow_hidden` clips to a rectangle, so the image needs the frame's
/// radius or its square corners escape the rounded border. This checks the
/// painted pixels: a golden image would hide a change this small under its
/// 0.1% tolerance.
#[test]
fn picture_avatar_corners_are_rounded() {
    use std::io::Cursor;
    use std::sync::Arc;

    use components::media::{ImageCache, MediaError, MediaFetcher};
    use domain::ImageRef;
    use futures::future::{BoxFuture, ready};
    use gpui::{Context, Render, Window};
    use image::{ImageFormat, Rgba, RgbaImage};

    struct Solid;

    impl MediaFetcher for Solid {
        fn fetch(&self, _: &ImageRef, _: u64) -> BoxFuture<'static, Result<Vec<u8>, MediaError>> {
            let mut png = Cursor::new(Vec::new());
            RgbaImage::from_pixel(72, 72, Rgba([200, 90, 60, 255]))
                .write_to(&mut png, ImageFormat::Png)
                .expect("png");
            Box::pin(ready(Ok(png.into_inner())))
        }
    }

    struct Host(ImageRef);

    impl Render for Host {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
            components::message_row::avatar_sized(
                "Ada Lovelace",
                cx.theme(),
                themes::metrics::LAYOUT_AVATAR_LG,
                Some(&self.0),
            )
        }
    }

    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let pic = ImageRef::Web(Arc::from("https://avatars.example.test/ada.png"));
        h.app().update(|app| {
            app.set_global(ImageCache::new(
                Arc::new(Solid),
                dir.path().to_path_buf(),
                8 << 20,
            ));
        });
        let size: f32 = themes::metrics::LAYOUT_AVATAR_LG.into();
        let w = h.open_sized(size, size, move |_, app| app.new(|_| Host(pic.clone())));
        h.settle();
        h.settle();
        let img = h.capture(w.into()).unwrap();
        let centre = *img.get_pixel(img.width() / 2, img.height() / 2);
        // Inside the corner arc, where a square-clipped picture still reaches.
        let corner = *img.get_pixel(8, 8);
        assert_eq!(centre, Rgba([200, 90, 60, 255]), "picture at the centre");
        assert_ne!(corner, centre, "the picture must not reach the corner");
    }
}

/// Counts the blue corner patches of the test pictures (`edge-*` in the
/// stage's fetcher) that stay square: a patch whose bounding box has all four
/// corners blue was not clipped by the avatar's rounded edge. Returns
/// `(patches, square_patches)`.
fn square_picture_corners(img: &image::RgbaImage) -> (usize, usize) {
    let blue = |x: u32, y: u32| {
        let p = img.get_pixel(x, y).0;
        p[2] > 90 && p[2] < 150 && p[0] < 30 && p[1] < 30
    };
    let (w, h) = img.dimensions();
    let mut seen = vec![false; (w * h) as usize];
    let (mut patches, mut square) = (0, 0);
    for y0 in 0..h {
        for x0 in 0..w {
            if seen[(y0 * w + x0) as usize] || !blue(x0, y0) {
                continue;
            }
            let (mut lo, mut hi) = ((x0, y0), (x0, y0));
            let mut stack = vec![(x0, y0)];
            seen[(y0 * w + x0) as usize] = true;
            let mut n = 0;
            while let Some((x, y)) = stack.pop() {
                n += 1;
                lo = (lo.0.min(x), lo.1.min(y));
                hi = (hi.0.max(x), hi.1.max(y));
                for (nx, ny) in [
                    (x.wrapping_sub(1), y),
                    (x + 1, y),
                    (x, y.wrapping_sub(1)),
                    (x, y + 1),
                ] {
                    if nx < w && ny < h && !seen[(ny * w + nx) as usize] && blue(nx, ny) {
                        seen[(ny * w + nx) as usize] = true;
                        stack.push((nx, ny));
                    }
                }
            }
            if n < 40 {
                continue;
            }
            patches += 1;
            if blue(lo.0, lo.1) && blue(hi.0, lo.1) && blue(lo.0, hi.1) && blue(hi.0, hi.1) {
                square += 1;
            }
        }
    }
    (patches, square)
}

fn pictures_stay_inside_their_corners(name: &str, at_least: usize) {
    for v in Variant::defaults() {
        let mut h = UiHarness::with_catalogs(v, &[parla::catalog()]).unwrap();
        let s = scenario::by_name(name).unwrap_or_else(|| panic!("no scenario {name}"));
        let img = stage::capture(&mut h, s, StageOptions::default()).unwrap();
        let (patches, square) = square_picture_corners(&img);
        assert!(
            patches >= at_least,
            "{name}: only {patches} picture patches drawn"
        );
        assert_eq!(square, 0, "{name}: {square} picture corners stay square");
    }
}

/// Members with pictures in the channel details list and in the message list.
#[test]
fn profile_pictures() {
    pictures_stay_inside_their_corners("profile_pictures", 12);
}

/// The People cards with pictures, one of them wider than tall.
#[test]
fn people_directory_pictures() {
    pictures_stay_inside_their_corners("people_directory_pictures", 8);
}
