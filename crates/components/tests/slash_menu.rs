use std::cell::RefCell;
use std::rc::Rc;

use components::composer::AcceptCompletion;
use components::{Cancel, Composer, ComposerEvent, ComposerSource, Send};
use domain::test_support::{cid, uid};
use domain::{
    Capabilities, ConversationId, MessageWindow, RemindTarget, SlashCommand, SlashError, UserId,
};
use gpui::{AppContext as _, Entity, Focusable as _, TestAppContext, VisualTestContext};
use i18n::Translate as _;

#[derive(Default)]
struct Log {
    commands: Vec<SlashCommand>,
    opened: Vec<ConversationId>,
}

struct Src {
    me: UserId,
    caps: Capabilities,
    log: Rc<RefCell<Log>>,
}

impl ComposerSource for Src {
    fn window(&self, _: &ConversationId) -> Option<&MessageWindow> {
        None
    }
    fn self_user(&self) -> Option<&UserId> {
        Some(&self.me)
    }
    fn capabilities(&self) -> &Capabilities {
        &self.caps
    }
    fn provider_name(&self) -> &str {
        "Fake"
    }
    fn run_slash(&mut self, cmd: SlashCommand, _: &mut gpui::App) -> Result<(), SlashError> {
        self.log.borrow_mut().commands.push(cmd);
        Ok(())
    }
    fn open_in_provider(&mut self, c: &ConversationId, _: &mut gpui::App) {
        self.log.borrow_mut().opened.push(c.clone());
    }
}

type Events = Rc<RefCell<Vec<ComposerEvent>>>;

fn all_caps() -> Capabilities {
    Capabilities {
        reminders: true,
        dnd: true,
        status: true,
        presence: true,
        ..Default::default()
    }
}

fn setup(
    cx: &mut TestAppContext,
    caps: Capabilities,
) -> (
    Entity<Composer<Src>>,
    Events,
    Rc<RefCell<Log>>,
    &mut VisualTestContext,
) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    let log = Rc::<RefCell<Log>>::default();
    let l = log.clone();
    let src = cx.new(move |_| Src {
        me: uid("u0"),
        caps,
        log: l,
    });
    let (view, vcx) = cx.add_window_view(move |_w, cx| Composer::new(src, cid("C1"), cx));
    let events: Events = Rc::default();
    let sink = events.clone();
    vcx.update(|w, cx| {
        cx.subscribe(&view, move |_, e: &ComposerEvent, _| {
            sink.borrow_mut().push(e.clone())
        })
        .detach();
        let h = view.read(cx).focus_handle(cx);
        w.focus(&h, cx);
    });
    (view, events, log, vcx)
}

fn text(v: &Entity<Composer<Src>>, vcx: &mut VisualTestContext) -> String {
    v.read_with(vcx, |c, cx| c.text(cx).to_string())
}

fn names(v: &Entity<Composer<Src>>, vcx: &mut VisualTestContext) -> Option<Vec<&'static str>> {
    v.read_with(vcx, |c, _| {
        c.slash_items().map(|i| i.iter().map(|e| e.name).collect())
    })
}

#[gpui::test]
fn slash_menu_lists_capable_commands(cx: &mut TestAppContext) {
    let (v, _e, _l, vcx) = setup(cx, all_caps());
    assert_eq!(names(&v, vcx), None);
    vcx.simulate_input("/");
    assert_eq!(
        names(&v, vcx),
        Some(vec!["remind", "dnd", "status", "away", "active", "shrug"])
    );
    vcx.simulate_input("a");
    assert_eq!(names(&v, vcx), Some(vec!["away", "active"]));
    vcx.simulate_input("w");
    assert_eq!(names(&v, vcx), Some(vec!["away"]));
    // A space ends the command name: the menu closes.
    vcx.simulate_input(" ");
    assert_eq!(names(&v, vcx), None);

    let (v, _e, _l, vcx) = setup(
        cx,
        Capabilities {
            reminders: true,
            ..Default::default()
        },
    );
    vcx.simulate_input("/");
    assert_eq!(names(&v, vcx), Some(vec!["remind", "shrug"]));

    // /shrug needs no capability.
    let (v, _e, _l, vcx) = setup(cx, Capabilities::default());
    vcx.simulate_input("/");
    assert_eq!(names(&v, vcx), Some(vec!["shrug"]));
}

#[gpui::test]
fn menu_is_only_for_an_empty_composer_start(cx: &mut TestAppContext) {
    let (v, _e, _l, vcx) = setup(cx, all_caps());
    vcx.simulate_input("hi /");
    assert_eq!(names(&v, vcx), None);
}

#[gpui::test]
fn menu_shows_usage_text(cx: &mut TestAppContext) {
    let (_v, _e, _l, vcx) = setup(cx, all_caps());
    let u = vcx.update(|_, cx| components::slash_menu::usage(cx.localizer(), "remind"));
    assert!(u.starts_with("/remind "), "{u}");
}

#[gpui::test]
fn tab_completes_the_selected_command(cx: &mut TestAppContext) {
    let (v, _e, _l, vcx) = setup(cx, all_caps());
    vcx.simulate_input("/re");
    vcx.dispatch_action(AcceptCompletion);
    assert_eq!(text(&v, vcx), "/remind ");
    assert_eq!(names(&v, vcx), None);
}

#[gpui::test]
fn enter_completes_a_partial_name_instead_of_running(cx: &mut TestAppContext) {
    let (v, e, l, vcx) = setup(cx, all_caps());
    vcx.simulate_input("/st");
    vcx.dispatch_action(Send);
    assert_eq!(text(&v, vcx), "/status ");
    assert!(e.borrow().is_empty());
    assert!(l.borrow().commands.is_empty());
}

#[gpui::test]
fn escape_closes_the_menu(cx: &mut TestAppContext) {
    let (v, _e, _l, vcx) = setup(cx, all_caps());
    vcx.simulate_input("/");
    vcx.dispatch_action(Cancel);
    assert_eq!(names(&v, vcx), None);
    assert_eq!(text(&v, vcx), "/");
}

#[gpui::test]
fn remind_runs_instead_of_sending(cx: &mut TestAppContext) {
    let (v, e, l, vcx) = setup(cx, all_caps());
    vcx.simulate_input("/remind me stretch in 20 minutes");
    vcx.dispatch_action(Send);
    assert!(e.borrow().is_empty(), "not sent as a message");
    assert!(matches!(
        l.borrow().commands.as_slice(),
        [SlashCommand::Remind {
            target: RemindTarget::Me,
            ..
        }]
    ));
    assert_eq!(text(&v, vcx), "");
}

#[gpui::test]
fn exact_name_runs_a_no_argument_command(cx: &mut TestAppContext) {
    let (v, _e, l, vcx) = setup(cx, all_caps());
    vcx.simulate_input("/away");
    vcx.dispatch_action(Send);
    assert_eq!(l.borrow().commands, [SlashCommand::Away]);
    assert_eq!(text(&v, vcx), "");
}

#[gpui::test]
fn unknown_command_offers_open_in_provider(cx: &mut TestAppContext) {
    let (v, e, l, vcx) = setup(cx, all_caps());
    vcx.simulate_input("/giphy cats");
    vcx.dispatch_action(Send);
    assert!(e.borrow().is_empty() && l.borrow().commands.is_empty());
    assert_eq!(text(&v, vcx), "/giphy cats");
    let (msg, open) = vcx.update(|_, cx| {
        let c = v.read(cx);
        (c.slash_notice(cx.localizer()), c.slash_open_label(cx))
    });
    assert_eq!(msg.as_deref(), Some("/giphy is not supported in Parla"));
    assert_eq!(open.as_deref(), Some("Open in Fake"));
    vcx.update(|_, cx| v.update(cx, |c, cx| c.open_in_provider(cx)));
    assert_eq!(l.borrow().opened, [cid("C1")]);
}

#[gpui::test]
fn command_without_capability_is_unsupported(cx: &mut TestAppContext) {
    let (v, e, l, vcx) = setup(cx, Capabilities::default());
    vcx.simulate_input("/away");
    vcx.dispatch_action(Send);
    assert!(e.borrow().is_empty() && l.borrow().commands.is_empty());
    assert_eq!(text(&v, vcx), "/away");
    let msg = vcx.update(|_, cx| v.read(cx).slash_notice(cx.localizer()));
    assert_eq!(msg.as_deref(), Some("/away is not supported in Parla"));
}

#[gpui::test]
fn usage_error_keeps_text(cx: &mut TestAppContext) {
    let (v, e, l, vcx) = setup(cx, all_caps());
    vcx.simulate_input("/dnd banana");
    vcx.dispatch_action(Send);
    assert!(e.borrow().is_empty() && l.borrow().commands.is_empty());
    assert_eq!(text(&v, vcx), "/dnd banana");
    let (msg, open) = vcx.update(|_, cx| {
        let c = v.read(cx);
        (c.slash_notice(cx.localizer()), c.slash_open_label(cx))
    });
    assert!(msg.unwrap().starts_with("/dnd "));
    assert_eq!(open, None);
    // Typing again clears the notice.
    vcx.simulate_input("!");
    assert_eq!(
        vcx.update(|_, cx| v.read(cx).slash_notice(cx.localizer())),
        None
    );
}

#[gpui::test]
fn ordinary_text_still_sends(cx: &mut TestAppContext) {
    let (_v, e, l, vcx) = setup(cx, all_caps());
    vcx.simulate_input("hello /remind");
    vcx.dispatch_action(Send);
    assert_eq!(e.borrow().len(), 1);
    assert!(l.borrow().commands.is_empty());
}

const SHRUG: &str = "\u{af}\\_(\u{30c4})_/\u{af}";

#[gpui::test]
fn shrug_appends_the_shrug_and_sends_a_message(cx: &mut TestAppContext) {
    let (v, e, l, vcx) = setup(cx, Capabilities::default());
    vcx.simulate_input("/shrug no idea");
    vcx.dispatch_action(Send);
    assert!(l.borrow().commands.is_empty(), "not a provider command");
    assert_eq!(
        *e.borrow(),
        [ComposerEvent::Submit {
            body: domain::ComposedText::plain(&format!("no idea {SHRUG}"))
        }]
    );
    assert_eq!(text(&v, vcx), "");
}

#[gpui::test]
fn bare_shrug_sends_just_the_shrug(cx: &mut TestAppContext) {
    let (_v, e, _l, vcx) = setup(cx, Capabilities::default());
    vcx.simulate_input("/SHRUG");
    vcx.dispatch_action(Send);
    assert_eq!(
        *e.borrow(),
        [ComposerEvent::Submit {
            body: domain::ComposedText::plain(SHRUG)
        }]
    );
}

#[gpui::test]
fn every_row_has_a_one_line_description(cx: &mut TestAppContext) {
    let (_v, _e, _l, vcx) = setup(cx, all_caps());
    vcx.update(|_, cx| {
        for n in ["remind", "dnd", "status", "away", "active", "shrug"] {
            let d = components::slash_menu::description(cx.localizer(), n);
            assert!(!d.is_empty() && !d.contains('\n'), "{n}: {d}");
        }
    });
}

#[gpui::test]
fn menu_rows_draw_their_description(cx: &mut TestAppContext) {
    let (_v, _e, _l, vcx) = setup(cx, all_caps());
    vcx.simulate_input("/");
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("slash-desc-0").is_some());
}

#[gpui::test]
fn the_menu_has_a_header_aligned_columns_source_tags_and_a_footer(cx: &mut TestAppContext) {
    let (_v, _e, _l, vcx) = setup(cx, all_caps());
    vcx.simulate_input("/");
    vcx.run_until_parked();
    assert!(vcx.debug_bounds("slash-header").is_some());
    assert!(vcx.debug_bounds("slash-footer").is_some());
    let names = [
        "slash-name-0",
        "slash-name-1",
        "slash-name-2",
        "slash-name-3",
        "slash-name-4",
        "slash-name-5",
    ];
    let descs = [
        "slash-desc-0",
        "slash-desc-1",
        "slash-desc-2",
        "slash-desc-3",
        "slash-desc-4",
        "slash-desc-5",
    ];
    let srcs = [
        "slash-source-0",
        "slash-source-1",
        "slash-source-2",
        "slash-source-3",
        "slash-source-4",
        "slash-source-5",
    ];
    let mut get = |l: &[&'static str]| -> Vec<gpui::Bounds<gpui::Pixels>> {
        l.iter()
            .map(|s| vcx.debug_bounds(s).expect("drawn"))
            .collect()
    };
    let (n, d, s) = (get(&names), get(&descs), get(&srcs));
    for i in 1..6 {
        assert_eq!(d[i].left(), d[0].left(), "descriptions align");
        assert_eq!(s[i].right(), s[0].right(), "tags are right aligned");
        assert!(n[i].size.width >= themes::metrics::LAYOUT_COMMAND_NAME);
    }
}

#[gpui::test]
fn menu_rows_match_the_board(cx: &mut TestAppContext) {
    let (_v, _e, _l, vcx) = setup(cx, all_caps());
    vcx.simulate_input("/");
    vcx.run_until_parked();
    // A header row above the list and a footer of key hints.
    assert!(vcx.debug_bounds("slash-header").is_some());
    assert!(vcx.debug_bounds("slash-footer").is_some());
    for sel in [
        "slash-hint-move",
        "slash-hint-complete",
        "slash-hint-run",
        "slash-hint-close",
    ] {
        assert!(
            vcx.debug_bounds(sel).is_some(),
            "{sel} is drawn in the footer"
        );
    }
    // Every row draws its command name, its description and its source badge.
    let rows = [
        ("slash-name-0", "slash-desc-0", "slash-source-0"),
        ("slash-name-1", "slash-desc-1", "slash-source-1"),
        ("slash-name-2", "slash-desc-2", "slash-source-2"),
        ("slash-name-3", "slash-desc-3", "slash-source-3"),
        ("slash-name-4", "slash-desc-4", "slash-source-4"),
        ("slash-name-5", "slash-desc-5", "slash-source-5"),
    ];
    for (name, desc, src) in rows {
        for sel in [name, desc, src] {
            assert!(vcx.debug_bounds(sel).is_some(), "{sel} is drawn");
        }
    }
}

#[gpui::test]
fn the_source_badge_names_where_a_command_comes_from(cx: &mut TestAppContext) {
    use components::slash_menu::{SlashSource, entries};
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
    });
    assert!(
        cx.update(|cx| entries(&all_caps(), "", cx))
            .iter()
            .all(|e| e.source == SlashSource::Builtin)
    );
    cx.update(|cx| {
        use i18n::Translate as _;
        let l = cx.localizer();
        let labels = [
            SlashSource::Builtin.label(l),
            SlashSource::Workspace.label(l),
            SlashSource::Plugin.label(l),
        ];
        assert_eq!(labels[0], "Built in");
        assert_eq!(labels[1], "Workspace");
        assert_eq!(labels[2], "Plugin");
    });
}
