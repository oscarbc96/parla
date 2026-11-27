//! the profile panel, the status dialog, own presence.
mod common;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use components::{
    ExpiryChoice, ProfileModel, ProfilePanel, ProfilePanelEvent, ProfileSource, StatusDialog,
    StatusDialogEvent, expiry_at, expiry_rows, next_presence, profile_model,
};
use domain::test_support::uid;
use domain::{DndStatus, EmojiRef, FixedNow, PresenceSetting, Profile, Status, UserId};
use gpui::{AppContext as _, Entity, TestAppContext, VisualTestContext};
use jiff::Timestamp;
use jiff::tz::TimeZone;
use settings::Clock;

/// 2026-11-15 10:00:00 UTC (a Sunday); 11:00 in Madrid.
const NOW: i64 = 1_794_736_800;

fn now() -> Timestamp {
    Timestamp::from_second(NOW).unwrap()
}

fn zone(name: &str) -> TimeZone {
    TimeZone::get(name).expect("tz")
}

fn profile(user: &str, name: &str) -> Profile {
    Profile {
        user: uid(user),
        display_name: Arc::from(name),
        real_name: Arc::from(name),
        title: None,
        pronouns: None,
        tz: None,
        status: None,
        email: None,
        phone: None,
        fields: Vec::new(),
        image: None,
        is_bot: false,
        deleted: false,
    }
}

fn palm(text: &str, expires: Option<i64>) -> Status {
    Status {
        emoji: Some(EmojiRef {
            shortcode: Arc::from("palm_tree"),
            unicode: Some(Arc::from("\u{1f334}")),
            skin_tone: None,
        }),
        text: Arc::from(text),
        expires,
    }
}

#[derive(Default)]
struct Src {
    profiles: HashMap<UserId, Profile>,
    presence: HashMap<UserId, bool>,
    dnd: HashMap<UserId, DndStatus>,
    huddle: HashSet<UserId>,
    shown: Option<UserId>,
    me: Option<UserId>,
    setting: Option<PresenceSetting>,
    dms: HashSet<UserId>,
    can_open: bool,
    can_status: bool,
    can_presence: bool,
    opened_dms: Vec<UserId>,
    closed: usize,
    presences: Vec<PresenceSetting>,
}

impl ProfileSource for Src {
    fn profile(&self, u: &UserId) -> Option<&Profile> {
        self.profiles.get(u)
    }
    fn presence(&self, u: &UserId) -> Option<bool> {
        self.presence.get(u).copied()
    }
    fn dnd(&self, u: &UserId) -> Option<&DndStatus> {
        self.dnd.get(u)
    }
    fn in_huddle(&self, u: &UserId) -> bool {
        self.huddle.contains(u)
    }
    fn shown_profile(&self) -> Option<&UserId> {
        self.shown.as_ref()
    }
    fn is_own(&self, u: &UserId) -> bool {
        self.me.as_ref() == Some(u)
    }
    fn own_presence(&self) -> Option<PresenceSetting> {
        self.setting
    }
    fn has_dm(&self, u: &UserId) -> bool {
        self.dms.contains(u)
    }
    fn can_open_dm(&self) -> bool {
        self.can_open
    }
    fn open_dm(&mut self, u: &UserId, _: &mut gpui::App) {
        self.opened_dms.push(u.clone());
    }
    fn close_profile(&mut self, _: &mut gpui::App) {
        self.closed += 1;
    }
    fn set_presence(&mut self, p: PresenceSetting, _: &mut gpui::App) {
        self.presences.push(p);
        self.setting = Some(p);
    }
    fn can_set_status(&self) -> bool {
        self.can_status
    }
    fn can_set_presence(&self) -> bool {
        self.can_presence
    }
}

fn model(src: &Src, user: &str, mine: &str) -> ProfileModel {
    profile_model(
        src,
        &uid(user),
        now(),
        &zone(mine),
        Clock::H24,
        &common::en(),
    )
    .expect("profile is loaded")
}

fn with(p: Profile) -> Src {
    let mut s = Src::default();
    s.profiles.insert(p.user.clone(), p);
    s
}

#[test]
fn panel_shows_fields_and_local_time() {
    let mut p = profile("u2", "Bob");
    p.real_name = Arc::from("Roberto Gil");
    p.title = Some(Arc::from("Staff engineer"));
    p.pronouns = Some(Arc::from("he/him"));
    p.email = Some(Arc::from("bob@example.com"));
    p.phone = Some(Arc::from("+34 600 000 000"));
    p.fields = vec![(Arc::from("Team"), Arc::from("Platform"))];
    p.status = Some(palm("On holiday", None));
    p.tz = Some(Arc::from("Europe/Madrid"));
    let src = with(p);
    let m = model(&src, "u2", "Europe/London");
    assert_eq!(m.name, "Bob");
    assert_eq!(m.real_name.as_deref(), Some("Roberto Gil"));
    assert_eq!(m.title.as_deref(), Some("Staff engineer"));
    assert_eq!(m.pronouns.as_deref(), Some("he/him"));
    assert_eq!(m.email.as_deref(), Some("bob@example.com"));
    assert_eq!(m.phone.as_deref(), Some("+34 600 000 000"));
    assert_eq!(m.fields, [("Team".to_string(), "Platform".to_string())]);
    assert_eq!(m.status.as_deref(), Some("\u{1f334} On holiday"));
    assert_eq!(m.local_time.as_deref(), Some("11:00, 1 h ahead of you"));
}

#[test]
fn local_time_names_the_offset_from_the_viewer() {
    let at = |tz: &str, mine: &str| {
        let mut p = profile("u2", "Bob");
        p.tz = Some(Arc::from(tz));
        model(&with(p), "u2", mine).local_time
    };
    assert_eq!(
        at("America/New_York", "Europe/Madrid").as_deref(),
        Some("05:00, 6 h behind you")
    );
    assert_eq!(
        at("Asia/Kolkata", "Europe/London").as_deref(),
        Some("15:30, 5 h 30 min ahead of you")
    );
    assert_eq!(
        at("Europe/Madrid", "Europe/Paris").as_deref(),
        Some("11:00, same time as you")
    );
    assert_eq!(at("Not/AZone", "Europe/Paris"), None);
    // No zone at all.
    let src = with(profile("u2", "Bob"));
    assert_eq!(model(&src, "u2", "Europe/Paris").local_time, None);
}

#[test]
fn unloaded_profile_has_no_model() {
    let src = Src::default();
    assert!(
        profile_model(
            &src,
            &uid("u2"),
            now(),
            &TimeZone::UTC,
            Clock::H24,
            &common::en()
        )
        .is_none()
    );
}

#[test]
fn expired_status_not_shown() {
    let shown = |expires| {
        let mut p = profile("u2", "Bob");
        p.status = Some(palm("Out", expires));
        model(&with(p), "u2", "UTC").status
    };
    assert_eq!(shown(None).as_deref(), Some("\u{1f334} Out"));
    assert_eq!(shown(Some(NOW + 60)).as_deref(), Some("\u{1f334} Out"));
    assert_eq!(shown(Some(NOW - 1)), None);
    assert_eq!(shown(Some(NOW)), None);
}

#[test]
fn dnd_and_huddle_badges() {
    let mut src = with(profile("u2", "Bob"));
    assert_eq!(model(&src, "u2", "Europe/Madrid").dnd, None);
    assert_eq!(model(&src, "u2", "Europe/Madrid").huddle, None);
    src.dnd.insert(
        uid("u2"),
        DndStatus {
            snooze_until: Some(NOW + 1800),
            ..Default::default()
        },
    );
    src.huddle.insert(uid("u2"));
    let m = model(&src, "u2", "Europe/Madrid");
    assert_eq!(m.dnd.as_deref(), Some("Notifications paused until 11:30"));
    assert_eq!(m.huddle.as_deref(), Some("In a huddle"));
    // A pause that already ended is not shown.
    src.dnd.insert(
        uid("u2"),
        DndStatus {
            snooze_until: Some(NOW - 5),
            ..Default::default()
        },
    );
    assert_eq!(model(&src, "u2", "Europe/Madrid").dnd, None);
}

#[test]
fn presence_of_others_and_of_self() {
    let mut src = with(profile("u2", "Bob"));
    assert_eq!(model(&src, "u2", "UTC").presence, None);
    src.presence.insert(uid("u2"), true);
    assert_eq!(model(&src, "u2", "UTC").presence.as_deref(), Some("Active"));
    src.presence.insert(uid("u2"), false);
    assert_eq!(model(&src, "u2", "UTC").presence.as_deref(), Some("Away"));
    // Own presence comes from the setting.
    let mut me = with(profile("u1", "Me"));
    me.me = Some(uid("u1"));
    assert!(model(&me, "u1", "UTC").own);
    assert_eq!(model(&me, "u1", "UTC").presence, None);
    me.setting = Some(PresenceSetting::Away);
    assert_eq!(model(&me, "u1", "UTC").presence.as_deref(), Some("Away"));
    me.setting = Some(PresenceSetting::Auto);
    assert_eq!(model(&me, "u1", "UTC").presence.as_deref(), Some("Active"));
}

#[test]
fn bots_show_what_they_have() {
    let mut p = profile("b1", "Deploy bot");
    p.is_bot = true;
    p.title = Some(Arc::from("Ships things"));
    let m = model(&with(p), "b1", "UTC");
    assert!(m.bot);
    assert_eq!(m.title.as_deref(), Some("Ships things"));
    assert_eq!(m.email, None);
    assert_eq!(m.local_time, None);
}

#[test]
fn presence_flips_between_away_and_auto() {
    assert_eq!(next_presence(None), PresenceSetting::Away);
    assert_eq!(
        next_presence(Some(PresenceSetting::Auto)),
        PresenceSetting::Away
    );
    assert_eq!(
        next_presence(Some(PresenceSetting::Away)),
        PresenceSetting::Auto
    );
}

fn init(cx: &mut TestAppContext) {
    cx.update(|cx| {
        themes::init_default(cx);
        components::init(cx);
        components::set_now(cx, Arc::new(FixedNow(now())));
        components::set_time_zone(cx, zone("Europe/Madrid"));
    });
}

fn setup(
    cx: &mut TestAppContext,
    src: Src,
) -> (
    Entity<Src>,
    Entity<ProfilePanel<Src>>,
    &mut VisualTestContext,
) {
    init(cx);
    let src = cx.new(|_| src);
    let s = src.clone();
    let (panel, vcx) = cx.add_window_view(move |_, cx| ProfilePanel::new(s, cx));
    (src, panel, vcx)
}

/// Counts the panel events equal to `want`.
fn count_events(
    vcx: &mut VisualTestContext,
    panel: &Entity<ProfilePanel<Src>>,
    want: ProfilePanelEvent,
) -> Arc<AtomicUsize> {
    let n = Arc::new(AtomicUsize::new(0));
    let c = n.clone();
    let sub = vcx.update(|_, cx| {
        cx.subscribe(panel, move |_, ev: &ProfilePanelEvent, _| {
            if *ev == want {
                c.fetch_add(1, Ordering::SeqCst);
            }
        })
    });
    std::mem::forget(sub);
    n
}

#[gpui::test]
fn presence_toggle(cx: &mut TestAppContext) {
    let mut s = with(profile("u1", "Me"));
    s.me = Some(uid("u1"));
    s.shown = Some(uid("u1"));
    s.can_presence = true;
    let (src, panel, vcx) = setup(cx, s);
    panel.update(vcx, |p, cx| p.toggle_presence(cx));
    src.read_with(vcx, |s, _| assert_eq!(s.presences, [PresenceSetting::Away]));
    panel.update(vcx, |p, cx| p.toggle_presence(cx));
    src.read_with(vcx, |s, _| {
        assert_eq!(s.presences, [PresenceSetting::Away, PresenceSetting::Auto])
    });
    // Another person's presence is not ours to set.
    src.update(vcx, |s, _| s.shown = Some(uid("u2")));
    panel.update(vcx, |p, cx| p.toggle_presence(cx));
    src.read_with(vcx, |s, _| assert_eq!(s.presences.len(), 2));
}

#[gpui::test]
fn message_opens_the_dm_and_closes(cx: &mut TestAppContext) {
    let mut s = with(profile("u2", "Bob"));
    s.shown = Some(uid("u2"));
    s.dms.insert(uid("u2"));
    let (src, panel, vcx) = setup(cx, s);
    let m = panel.read_with(vcx, |p, cx| p.model(cx)).unwrap();
    assert!(m.can_message);
    let closed = count_events(vcx, &panel, ProfilePanelEvent::Close);
    panel.update(vcx, |p, cx| p.message(cx));
    src.read_with(vcx, |s, _| {
        assert_eq!(s.opened_dms, [uid("u2")]);
        assert_eq!(s.closed, 1);
    });
    assert_eq!(closed.load(Ordering::SeqCst), 1);
}

#[gpui::test]
fn no_message_button_without_a_dm(cx: &mut TestAppContext) {
    let mut s = with(profile("u2", "Bob"));
    s.shown = Some(uid("u2"));
    let (_src, panel, vcx) = setup(cx, s);
    let m = panel.read_with(vcx, |p, cx| p.model(cx)).unwrap();
    assert!(!m.can_message);
    assert!(vcx.debug_bounds("profile-message").is_none());
}

#[gpui::test]
fn message_starts_a_dm_when_the_provider_can_open_one(cx: &mut TestAppContext) {
    let mut s = with(profile("u2", "Bob"));
    s.shown = Some(uid("u2"));
    s.can_open = true;
    let (src, panel, vcx) = setup(cx, s);
    let m = panel.read_with(vcx, |p, cx| p.model(cx)).unwrap();
    assert!(m.can_message);
    panel.update(vcx, |p, cx| p.message(cx));
    src.read_with(vcx, |s, _| assert_eq!(s.opened_dms, [uid("u2")]));
}

#[test]
fn model_carries_the_presence_flag() {
    let mut s = with(profile("u2", "Bob"));
    assert_eq!(model(&s, "u2", "UTC").presence_active, None);
    s.presence.insert(uid("u2"), true);
    let m = model(&s, "u2", "UTC");
    assert_eq!(m.presence_active, Some(true));
    assert!(m.presence.is_some());
    s.presence.insert(uid("u2"), false);
    assert_eq!(model(&s, "u2", "UTC").presence_active, Some(false));
}

#[gpui::test]
fn panel_paints_its_fields(cx: &mut TestAppContext) {
    let mut p = profile("u2", "Bob");
    p.title = Some(Arc::from("Engineer"));
    p.email = Some(Arc::from("bob@example.com"));
    let mut s = with(p);
    s.shown = Some(uid("u2"));
    s.dms.insert(uid("u2"));
    let (_src, _panel, vcx) = setup(cx, s);
    for sel in [
        "profile-name",
        "profile-title",
        "profile-email",
        "profile-message",
    ] {
        assert!(vcx.debug_bounds(sel).is_some(), "{sel}");
    }
}

#[gpui::test]
fn edit_status_is_for_own_profile_only(cx: &mut TestAppContext) {
    let mut s = with(profile("u1", "Me"));
    s.me = Some(uid("u1"));
    s.shown = Some(uid("u1"));
    s.can_status = true;
    let (src, panel, vcx) = setup(cx, s);
    let seen = count_events(vcx, &panel, ProfilePanelEvent::EditStatus);
    panel.update(vcx, |p, cx| p.edit_status(cx));
    assert_eq!(seen.load(Ordering::SeqCst), 1);
    src.update(vcx, |s, _| s.shown = Some(uid("u2")));
    panel.update(vcx, |p, cx| p.edit_status(cx));
    assert_eq!(seen.load(Ordering::SeqCst), 1);
}

// The status dialog.

fn zoned(tz: &str) -> jiff::ZonedDateTime {
    now().to_zoned_datetime(zone(tz))
}

#[test]
fn expiry_choices_in_order() {
    let rows = expiry_rows(&common::en());
    let labels: Vec<_> = rows.iter().map(|r| r.label.as_str()).collect();
    assert_eq!(
        labels,
        [
            "Don't clear",
            "30 minutes",
            "1 hour",
            "4 hours",
            "Today",
            "This week",
            "Custom..."
        ]
    );
    let choices: Vec<_> = rows.iter().map(|r| r.choice).collect();
    assert_eq!(
        choices,
        [
            ExpiryChoice::Never,
            ExpiryChoice::Minutes(30),
            ExpiryChoice::Minutes(60),
            ExpiryChoice::Minutes(240),
            ExpiryChoice::Today,
            ExpiryChoice::ThisWeek,
            ExpiryChoice::Custom,
        ]
    );
}

#[test]
fn expiry_times() {
    let z = zoned("Europe/Madrid");
    assert_eq!(expiry_at(ExpiryChoice::Never, &z, None), Some(None));
    assert_eq!(
        expiry_at(ExpiryChoice::Minutes(30), &z, None),
        Some(Some(NOW + 1800))
    );
    assert_eq!(
        expiry_at(ExpiryChoice::Minutes(240), &z, None),
        Some(Some(NOW + 4 * 3600))
    );
    // Madrid is 11:00: the day ends at midnight, 13 hours later.
    assert_eq!(
        expiry_at(ExpiryChoice::Today, &z, None),
        Some(Some(NOW + 13 * 3600))
    );
    // Sunday 11:00: the week ends at the next midnight too.
    assert_eq!(
        expiry_at(ExpiryChoice::ThisWeek, &z, None),
        Some(Some(NOW + 13 * 3600))
    );
    // Saturday 2026-11-14 11:00 Madrid: the week ends at Sunday's end, 37 hours later.
    let sat = Timestamp::from_second(NOW - 86_400)
        .unwrap()
        .to_zoned_datetime(zone("Europe/Madrid"));
    assert_eq!(
        expiry_at(ExpiryChoice::ThisWeek, &sat, None),
        Some(Some(NOW - 86_400 + 37 * 3600))
    );
    assert_eq!(
        expiry_at(ExpiryChoice::Custom, &z, Some(45)),
        Some(Some(NOW + 2700))
    );
    assert_eq!(expiry_at(ExpiryChoice::Custom, &z, None), None);
}

type Saved = Arc<std::sync::Mutex<Vec<StatusDialogEvent>>>;

fn dialog(
    cx: &mut TestAppContext,
    current: Option<Status>,
) -> (Entity<StatusDialog>, Saved, &mut VisualTestContext) {
    init(cx);
    let (d, vcx) = cx.add_window_view(move |_, cx| StatusDialog::new(current.as_ref(), cx));
    let saved = Saved::default();
    let s = saved.clone();
    let sub = vcx.update(|_, cx| {
        cx.subscribe(&d, move |_, ev: &StatusDialogEvent, _| {
            s.lock().unwrap().push(ev.clone())
        })
    });
    std::mem::forget(sub);
    (d, saved, vcx)
}

#[gpui::test]
fn status_dialog_sets_expiry(cx: &mut TestAppContext) {
    let (d, saved, vcx) = dialog(cx, None);
    let emoji = palm("", None).emoji;
    d.update(vcx, |d, cx| {
        d.set_text("Out for lunch", cx);
        d.set_emoji(emoji.clone(), cx);
        d.choose(ExpiryChoice::Minutes(60), cx);
        d.save(cx);
    });
    d.update(vcx, |d, cx| {
        d.choose(ExpiryChoice::Never, cx);
        d.save(cx);
    });
    d.update(vcx, |d, cx| {
        d.choose(ExpiryChoice::Custom, cx);
        d.set_custom("45", cx);
        d.save(cx);
    });
    // An invalid custom duration saves nothing.
    d.update(vcx, |d, cx| {
        d.set_custom("soon", cx);
        d.save(cx);
    });
    let got = saved.lock().unwrap().clone();
    let st = |expires| {
        StatusDialogEvent::Save(Some(Status {
            emoji: emoji.clone(),
            text: Arc::from("Out for lunch"),
            expires,
        }))
    };
    assert_eq!(got, [st(Some(NOW + 3600)), st(None), st(Some(NOW + 2700))]);
}

#[gpui::test]
fn status_dialog_keeps_existing_expiry(cx: &mut TestAppContext) {
    let (d, _saved, vcx) = dialog(cx, Some(palm("Away", Some(NOW + 2700))));
    let s = d.read_with(vcx, |d, cx| d.status(cx)).unwrap();
    assert_eq!(s.expires, Some(NOW + 2700));
    // An expiry already in the past is not kept.
    let (d, _saved, vcx) = dialog(cx, Some(palm("Away", Some(NOW - 60))));
    let s = d.read_with(vcx, |d, cx| d.status(cx)).unwrap();
    assert_eq!(s.expires, None);
}

#[gpui::test]
fn status_dialog_clear_and_blank(cx: &mut TestAppContext) {
    let (d, saved, vcx) = dialog(cx, Some(palm("Away", None)));
    // Prefilled from the current status.
    let s = d.read_with(vcx, |d, cx| d.status(cx));
    assert_eq!(s.map(|s| s.text.to_string()).as_deref(), Some("Away"));
    d.update(vcx, |d, cx| d.clear(cx));
    // Blank text and no emoji is a clear.
    d.update(vcx, |d, cx| {
        d.set_text("  ", cx);
        d.set_emoji(None, cx);
        d.save(cx);
    });
    d.update(vcx, |d, cx| d.cancel(cx));
    assert_eq!(
        *saved.lock().unwrap(),
        [
            StatusDialogEvent::Save(None),
            StatusDialogEvent::Save(None),
            StatusDialogEvent::Cancel
        ]
    );
}

#[gpui::test]
fn status_dialog_enter_on_focused_button_activates_only_it(cx: &mut TestAppContext) {
    // Tab order after the dialog: emoji-less fields first, then Clear, Cancel, Save.
    let mut seen = Vec::new();
    // The presets are tab stops ahead of the buttons.
    for tabs in 1..=12 {
        let (d, saved, vcx) = dialog(cx, Some(palm("Away", None)));
        d.update(vcx, |d, cx| d.choose(ExpiryChoice::Never, cx));
        vcx.update(|w, cx| {
            let f = gpui::Focusable::focus_handle(d.read(cx), cx);
            w.focus(&f, cx)
        });
        for _ in 0..tabs {
            vcx.update(|w, cx| w.focus_next(cx));
        }
        vcx.simulate_keystrokes("enter");
        seen.push(saved.lock().unwrap().clone());
    }
    eprintln!("{seen:?}");
    // Cancel and Clear never fall through to the dialog's Enter, which saves.
    let saves = |e: &Vec<StatusDialogEvent>| {
        e.iter()
            .filter(|e| matches!(e, StatusDialogEvent::Save(Some(_))))
            .count()
    };
    assert!(
        seen.iter().any(|e| e == &[StatusDialogEvent::Cancel]),
        "{seen:?}"
    );
    for e in &seen {
        if e.contains(&StatusDialogEvent::Cancel) {
            assert_eq!(saves(e), 0, "{e:?}");
        }
    }
}

#[gpui::test]
fn status_presets_fill_the_dialog(cx: &mut TestAppContext) {
    let (d, saved, vcx) = dialog(cx, None);
    let presets = vcx.update(|_, cx| components::status_presets(&i18n::gpui::localizer(cx)));
    let got: Vec<(String, String, ExpiryChoice)> = presets
        .iter()
        .map(|p| {
            (
                p.emoji.unicode.as_deref().unwrap_or_default().to_string(),
                p.text.clone(),
                p.choice,
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            (
                "\u{1f3a7}".into(),
                "Focusing".into(),
                ExpiryChoice::Minutes(60)
            ),
            (
                "\u{1f4c5}".into(),
                "In a meeting".into(),
                ExpiryChoice::Minutes(60)
            ),
            (
                "\u{1f68c}".into(),
                "Commuting".into(),
                ExpiryChoice::Minutes(30)
            ),
            ("\u{1f912}".into(), "Out sick".into(), ExpiryChoice::Today),
            ("\u{1f334}".into(), "On holiday".into(), ExpiryChoice::Never),
        ]
    );
    // A click on a preset fills the emoji, the text and the expiry.
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    let row = vcx.debug_bounds("status-preset-0").expect("preset row");
    vcx.simulate_click(row.center(), gpui::Modifiers::none());
    d.update(vcx, |d, cx| d.save(cx));
    d.update(vcx, |d, cx| {
        d.apply_preset(3, cx);
        d.save(cx);
    });
    d.update(vcx, |d, cx| {
        d.apply_preset(4, cx);
        d.save(cx);
    });
    let st = |p: &components::StatusPreset, expires| {
        StatusDialogEvent::Save(Some(Status {
            emoji: Some(p.emoji.clone()),
            text: Arc::from(p.text.as_str()),
            expires,
        }))
    };
    // Sunday 11:00 in Madrid: "Today" ends 13 hours later.
    assert_eq!(
        *saved.lock().unwrap(),
        [
            st(&presets[0], Some(NOW + 3600)),
            st(&presets[3], Some(NOW + 13 * 3600)),
            st(&presets[4], None),
        ]
    );
}

/// The dialog as the window shows it: under the top inset, in a 600pt window.
struct DialogHost {
    dialog: Entity<StatusDialog>,
}

impl gpui::Render for DialogHost {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        use gpui::{ParentElement as _, Styled as _};
        gpui::div()
            .size_full()
            .flex()
            .justify_center()
            .items_start()
            .pt(themes::metrics::LAYOUT_DIALOG_TOP)
            .child(
                gpui::div()
                    .w(themes::metrics::LAYOUT_DIALOG_SM)
                    .child(self.dialog.clone()),
            )
    }
}

/// In a short window the body scrolls and the buttons stay on screen; preset
/// rows are 40pt.
#[gpui::test]
fn status_dialog_buttons_stay_visible_in_a_short_window(cx: &mut TestAppContext) {
    init(cx);
    let (_v, vcx) = cx.add_window_view(|_, cx| DialogHost {
        dialog: cx.new(|cx| StatusDialog::new(None, cx)),
    });
    vcx.simulate_resize(gpui::size(gpui::px(800.0), gpui::px(600.0)));
    vcx.run_until_parked();
    vcx.update(|w, cx| w.draw(cx).clear(cx));
    for b in ["status-save", "status-cancel", "status-clear"] {
        let r = vcx.debug_bounds(b).unwrap_or_else(|| panic!("{b}"));
        assert!(r.bottom() <= gpui::px(600.0), "{b} at {:?}", r.bottom());
    }
    let row = vcx.debug_bounds("status-preset-0").expect("preset");
    assert_eq!(row.size.height, themes::metrics::LAYOUT_PRESET_ROW);
}

/// A preset row is a tab stop: Enter on it fills the dialog and saves nothing.
#[gpui::test]
fn status_preset_activates_from_the_keyboard(cx: &mut TestAppContext) {
    let mut filled = false;
    for tabs in 1..=8 {
        let (d, saved, vcx) = dialog(cx, None);
        vcx.update(|w, cx| {
            let f = gpui::Focusable::focus_handle(d.read(cx), cx);
            w.focus(&f, cx)
        });
        for _ in 0..tabs {
            vcx.update(|w, cx| w.focus_next(cx));
        }
        vcx.simulate_keystrokes("enter");
        let text = d.read_with(vcx, |d, cx| d.status(cx).map(|s| s.text.to_string()));
        if text.as_deref() == Some("Focusing") {
            assert!(saved.lock().unwrap().is_empty(), "Enter on a preset saved");
            filled = true;
            break;
        }
    }
    assert!(filled, "no preset row is reachable with Tab");
}
