//! The huddle bar above the timeline: who is in the conversation's live
//! huddle, how long ago it started, and Join. The duration label refreshes
//! once a minute through one one-shot timer that exists only while the bar is
//! painted with a live huddle (no polling, nothing while idle).

use std::time::Duration;

use domain::{ConversationId, Huddle, Member, UserId};
use gpui::{
    App, Context, Entity, IntoElement, Render, SharedString, Subscription, Task, Window, div,
    prelude::*,
};
use themes::{ActiveTheme, metrics, parla_tokens};

use crate::badge::initials;
use crate::sidebar::best_name;

/// Most avatars the bar shows before "+N".
pub const MAX_AVATARS: usize = 5;
/// How often the "started N min ago" label refreshes while painted.
const TICK: Duration = Duration::from_secs(60);

/// What the bar needs from the session, so `components` does not depend on `parla`.
pub trait HuddleSource {
    /// The live huddle of `c`, if any.
    fn huddle(&self, c: &ConversationId) -> Option<&Huddle>;
    fn self_user(&self) -> Option<&UserId>;
    fn member(&self, u: &UserId) -> Option<&Member>;
    /// Opens the huddle of `c` per the call settings.
    fn join_huddle(&mut self, c: ConversationId, cx: &mut App);
    /// The provider has huddles, so one can be started where none runs.
    fn can_start_huddle(&self) -> bool {
        false
    }
    /// The running call is this conversation's huddle (the call bar shows it).
    fn in_call_here(&self, _c: &ConversationId, _cx: &App) -> bool {
        false
    }
}

/// What the bar's button does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HuddleAction {
    /// No huddle runs here.
    Start,
    /// A huddle runs and the user is not in it.
    Join,
}

impl HuddleAction {
    /// The button label.
    pub fn label(self, l: &i18n::Localizer) -> String {
        match self {
            Self::Start => t!(l, "huddle-bar-start").into_owned(),
            Self::Join => t!(l, "huddle-bar-join").into_owned(),
        }
    }
}

/// One participant avatar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Avatar {
    pub name: SharedString,
    pub initials: SharedString,
}

/// The bar as it is painted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BarView {
    pub avatars: Vec<Avatar>,
    /// Participants beyond [`MAX_AVATARS`].
    pub more: usize,
    /// "Started 12 min ago".
    pub started: String,
    /// Join is offered (the user is not in the huddle already).
    pub can_join: bool,
    /// Shown instead of Join when the user is in the huddle elsewhere.
    pub note: Option<String>,
}

pub struct HuddleBar<S: HuddleSource + 'static> {
    session: Entity<S>,
    conversation: ConversationId,
    tick: Option<Task<()>>,
    _sub: Subscription,
}

/// The button the huddle bar offers for `c`: Start where no huddle runs, Join
/// where one does; none while the conversation's call is open here or the user
/// is already in the huddle. The header's Huddle button uses the same check.
pub fn huddle_action<S: HuddleSource>(s: &S, c: &ConversationId, cx: &App) -> Option<HuddleAction> {
    if s.in_call_here(c, cx) {
        return None;
    }
    match s.huddle(c) {
        Some(h) => {
            let me_in = s.self_user().is_some_and(|me| h.participants.contains(me));
            (!me_in).then_some(HuddleAction::Join)
        }
        None => s.can_start_huddle().then_some(HuddleAction::Start),
    }
}

impl<S: HuddleSource + 'static> HuddleBar<S> {
    pub fn new(session: Entity<S>, conversation: ConversationId, cx: &mut Context<Self>) -> Self {
        let sub = cx.observe(&session, |_, _, cx| cx.notify());
        Self {
            session,
            conversation,
            tick: None,
            _sub: sub,
        }
    }

    /// Shows another conversation's huddle.
    pub fn set_conversation(&mut self, c: ConversationId, cx: &mut Context<Self>) {
        if self.conversation != c {
            self.conversation = c;
            self.tick = None;
            cx.notify();
        }
    }

    /// The bar's content; `None` when the conversation has no live huddle.
    pub fn view(&self, cx: &App) -> Option<BarView> {
        let s = self.session.read(cx);
        let h = s.huddle(&self.conversation)?;
        let l = i18n::Translate::localizer(cx);
        let me_in = s.self_user().is_some_and(|me| h.participants.contains(me));
        let avatars = h
            .participants
            .iter()
            .take(MAX_AVATARS)
            .map(|u| {
                let name = s
                    .member(u)
                    .and_then(best_name)
                    .map(|n| n.to_string())
                    .unwrap_or_default();
                Avatar {
                    initials: SharedString::from(initials(&name)),
                    name: SharedString::from(name),
                }
            })
            .collect();
        let minutes = (crate::now_secs(cx) - h.started).max(0) / 60;
        let started = match minutes {
            0 => t!(l, "huddle-bar-started-now").into_owned(),
            1..=59 => t!(l, "huddle-bar-started-minutes", minutes = minutes).into_owned(),
            _ => t!(l, "huddle-bar-started-hours", hours = minutes / 60).into_owned(),
        };
        Some(BarView {
            avatars,
            more: h.participants.len().saturating_sub(MAX_AVATARS),
            started,
            can_join: !me_in && !s.in_call_here(&self.conversation, cx),
            note: (me_in && !s.in_call_here(&self.conversation, cx))
                .then(|| t!(l, "huddle-bar-self-in").into_owned()),
        })
    }

    /// The button the bar offers: Start where no huddle runs, Join where one
    /// does; none while this conversation's call is open or the user is in it.
    pub fn action(&self, cx: &App) -> Option<HuddleAction> {
        huddle_action(self.session.read(cx), &self.conversation, cx)
    }

    /// Joins the huddle; does nothing when there is none or the user is in it.
    pub fn join(&mut self, cx: &mut Context<Self>) {
        if self.action(cx) != Some(HuddleAction::Join) {
            return;
        }
        self.open(cx);
    }

    /// Starts a huddle; does nothing when one runs or none can be started.
    pub fn start(&mut self, cx: &mut Context<Self>) {
        if self.action(cx) != Some(HuddleAction::Start) {
            return;
        }
        self.open(cx);
    }

    fn open(&mut self, cx: &mut Context<Self>) {
        let c = self.conversation.clone();
        self.session.update(cx, |s, cx| s.join_huddle(c, cx));
    }

    /// The minute timer is armed (tests).
    pub fn timer_armed(&self) -> bool {
        self.tick.is_some()
    }

    /// Arms the one-shot refresh while a live huddle is painted; drops it
    /// otherwise.
    fn sync_timer(&mut self, live: bool, cx: &mut Context<Self>) {
        if !live || crate::motion(cx) == crate::Motion::Frozen {
            self.tick = None;
        } else if self.tick.is_none() {
            self.tick = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(TICK).await;
                this.update(cx, |b, cx| {
                    // Cleared first: the repaint arms the next one if still live.
                    b.tick = None;
                    cx.notify();
                })
                .ok();
            }));
        }
    }
}

impl<S: HuddleSource + 'static> Render for HuddleBar<S> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = self.view(cx);
        self.sync_timer(view.is_some(), cx);
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let tokens = parla_tokens(&theme);
        let l = i18n::Translate::localizer(&**cx);
        let Some(v) = view else {
            if self.action(cx) != Some(HuddleAction::Start) {
                return div().into_any_element();
            }
            // No huddle runs: only the Start button, right aligned.
            return div()
                .debug_selector(|| "huddle-bar-idle".to_string())
                .flex()
                .flex_none()
                .items_center()
                .justify_end()
                .w_full()
                .px(metrics::SPACE_5)
                .py(metrics::SPACE_2)
                .text_size(metrics::FONT_SIZE_SM)
                .border_b_1()
                .border_color(tokens.border_subtle)
                .child(
                    crate::controls::button(
                        "huddle-start",
                        HuddleAction::Start.label(l),
                        crate::controls::ButtonKind::Secondary,
                        true,
                        colors,
                    )
                    .debug_selector(|| "huddle-start".to_string())
                    .hover(|s| s.bg(colors.element_hover))
                    .on_click(cx.listener(|this, _, _, cx| this.start(cx))),
                )
                .into_any_element();
        };
        // Overlapping avatar stack: circles ringed in the bar's ground.
        let mut avatars = div().flex().items_center();
        for (i, a) in v.avatars.iter().enumerate() {
            avatars = avatars.child(
                div()
                    .id(("huddle-avatar", i))
                    .debug_selector(|| format!("huddle-avatar-{i}"))
                    .size(metrics::LAYOUT_AVATAR_SM)
                    .when(i > 0, |d| d.ml(-metrics::SPACE_3))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .border_2()
                    .border_color(colors.elevated_surface_background)
                    .bg(colors.element_selected)
                    .text_size(metrics::FONT_SIZE_3XS)
                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                    .child(a.initials.clone()),
            );
        }
        let mut bar = div()
            .debug_selector(|| "huddle-bar".to_string())
            .flex()
            .items_center()
            .gap(metrics::SPACE_5)
            .px(metrics::SPACE_5)
            .py(metrics::SPACE_4)
            .text_size(metrics::FONT_SIZE_SM)
            .text_color(colors.text)
            .bg(colors.elevated_surface_background)
            .border_1()
            .border_color(colors.border)
            .rounded(metrics::RADIUS_LG)
            .child(
                div()
                    .debug_selector(|| "huddle-live-dot".to_string())
                    .flex_none()
                    .size(metrics::SPACE_4)
                    .rounded_full()
                    .bg(tokens.presence_active),
            )
            .child(
                div()
                    .font_weight(metrics::FONT_WEIGHT_BOLD)
                    .child(t!(l, "huddle-bar-title").into_owned()),
            )
            .child(avatars);
        if v.more > 0 {
            bar = bar.child(
                div()
                    .debug_selector(|| "huddle-more".to_string())
                    .text_size(metrics::FONT_SIZE_XS)
                    .text_color(colors.text_muted)
                    .child(t!(l, "huddle-bar-more", count = v.more).into_owned()),
            );
        }
        bar = bar.child(
            div()
                .flex_1()
                .text_size(metrics::FONT_SIZE_XS)
                .text_color(tokens.text_subtle)
                .child(v.started.clone()),
        );
        if let Some(n) = v.note.clone() {
            bar = bar.child(
                div()
                    .debug_selector(|| "huddle-self-in".to_string())
                    .text_size(metrics::FONT_SIZE_XS)
                    .text_color(colors.text_muted)
                    .child(n),
            );
        }
        if v.can_join {
            bar = bar.child(
                div()
                    .id("huddle-join")
                    .debug_selector(|| "huddle-join".to_string())
                    .px(metrics::SPACE_5)
                    .py(metrics::SPACE_2)
                    .rounded(metrics::RADIUS_MD)
                    .bg(theme.status().success)
                    .text_color(colors.text_on_accent)
                    .font_weight(metrics::FONT_WEIGHT_MEDIUM)
                    .cursor_pointer()
                    .hover(|s| s.opacity(0.9))
                    .child(t!(l, "huddle-bar-join").into_owned())
                    .on_click(cx.listener(|this, _, _, cx| this.join(cx))),
            );
        }
        div()
            .flex()
            .flex_none()
            .flex_col()
            .w_full()
            .px(metrics::SPACE_5)
            .py(metrics::SPACE_3)
            .child(bar)
            .into_any_element()
    }
}
