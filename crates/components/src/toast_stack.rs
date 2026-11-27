//! The toast stack (board States, "Toast stack"): short messages at the bottom
//! left of the main window, each with an optional action (Undo) and a close
//! button. Bounded: at most [`MAX_VISIBLE_TOASTS`] show, up to
//! [`MAX_QUEUED_TOASTS`] more wait. Each shown toast owns one timer that ends
//! with it and is dropped while the pointer or the keyboard focus is on the
//! toast; nothing runs while the stack is empty. Every window that hosts a
//! stack draws its own, and a toast shows in the window that pushed it.

use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Duration;

use gpui::{
    App, Context, Entity, FocusHandle, Global, IntoElement, Render, SharedString, Subscription,
    Task, Window, WindowId, div, prelude::*,
};
use themes::{ActiveTheme, metrics};

use crate::controls;
use crate::icon::{Icon, IconName};

/// Toasts on screen at once.
pub const MAX_VISIBLE_TOASTS: usize = 4;
/// Toasts waiting for a free slot; past this the oldest waiting one is dropped.
pub const MAX_QUEUED_TOASTS: usize = 8;
/// How long a success or info toast stays.
pub const TOAST_TIMEOUT: Duration = Duration::from_secs(5);
/// How long an error toast stays (long enough to read and act on).
pub const TOAST_ERROR_TIMEOUT: Duration = Duration::from_secs(10);
/// The least a toast with an action stays, whatever its timeout: time to reach it.
pub const TOAST_ACTION_MIN: Duration = Duration::from_secs(10);

/// What a toast reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Success,
    Info,
    Error,
}

/// Identifies a toast, for [`ToastStack::dismiss`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ToastId(u64);

type ActionFn = Rc<dyn Fn(&mut App)>;

/// A toast to show. Text is already localised by the caller.
#[derive(Clone)]
pub struct Toast {
    kind: ToastKind,
    title: SharedString,
    detail: Option<SharedString>,
    action: Option<(SharedString, ActionFn)>,
    timeout: Option<Duration>,
}

impl Toast {
    pub fn new(kind: ToastKind, title: impl Into<SharedString>) -> Self {
        Self {
            kind,
            title: title.into(),
            detail: None,
            action: None,
            timeout: None,
        }
    }

    /// A second, muted line under the title.
    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// A link-style button (such as Undo). Running it dismisses the toast.
    pub fn action(
        mut self,
        label: impl Into<SharedString>,
        run: impl Fn(&mut App) + 'static,
    ) -> Self {
        self.action = Some((label.into(), Rc::new(run)));
        self
    }

    /// Overrides the default stay ([`TOAST_TIMEOUT`], longer for errors); a toast
    /// with an action stays at least [`TOAST_ACTION_MIN`].
    pub fn timeout(mut self, after: Duration) -> Self {
        self.timeout = Some(after);
        self
    }

    fn stay(&self) -> Duration {
        let base = self.timeout.unwrap_or(match self.kind {
            ToastKind::Error => TOAST_ERROR_TIMEOUT,
            _ => TOAST_TIMEOUT,
        });
        if self.action.is_some() {
            base.max(TOAST_ACTION_MIN)
        } else {
            base
        }
    }

    /// The same words and no action: pushing it again only restarts the timer.
    fn same_as(&self, other: &Toast) -> bool {
        self.kind == other.kind
            && self.title == other.title
            && self.detail == other.detail
            && self.action.is_none()
            && other.action.is_none()
    }
}

/// A toast on screen.
pub struct Shown {
    pub id: ToastId,
    pub kind: ToastKind,
    pub title: SharedString,
    pub detail: Option<SharedString>,
    /// Title and detail as one accessible label, built once.
    label: SharedString,
    toast: Toast,
    /// Ends the toast when it fires; dropped (cancelled) with the toast and
    /// while it is held.
    timer: Option<Task<()>>,
    focus: FocusHandle,
    hovered: bool,
    focused: bool,
    subs: Vec<Subscription>,
}

#[derive(Clone, Copy)]
enum Hold {
    Hover,
    Focus,
}

/// The stack: what shows, what waits.
pub struct ToastStack {
    shown: Vec<Shown>,
    waiting: VecDeque<(ToastId, Toast)>,
    next: u64,
    /// The close button's label; rebuilt after a language change.
    dismiss_label: Option<SharedString>,
    _locale: Subscription,
}

impl ToastStack {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let _locale = cx.observe_global::<i18n::gpui::I18n>(|this, _| this.dismiss_label = None);
        Self {
            shown: Vec::new(),
            waiting: VecDeque::new(),
            next: 0,
            dismiss_label: None,
            _locale,
        }
    }

    /// Adds `toast`: shown now when there is room, else queued. Returns its id.
    pub fn push(&mut self, toast: Toast, cx: &mut Context<Self>) -> ToastId {
        if let Some(i) = self.shown.iter().position(|s| s.toast.same_as(&toast)) {
            let id = self.shown[i].id;
            if !(self.shown[i].hovered || self.shown[i].focused) {
                self.arm(i, cx);
            }
            return id;
        }
        if let Some((id, _)) = self.waiting.iter().find(|(_, t)| t.same_as(&toast)) {
            return *id;
        }
        let id = ToastId(self.next);
        self.next += 1;
        if self.shown.len() < MAX_VISIBLE_TOASTS {
            self.show(id, toast, cx);
        } else {
            if self.waiting.len() >= MAX_QUEUED_TOASTS {
                // Keep a toast the user can act on over a plain notice.
                let drop = self
                    .waiting
                    .iter()
                    .position(|(_, t)| t.action.is_none())
                    .unwrap_or(0);
                self.waiting.remove(drop);
            }
            self.waiting.push_back((id, toast));
        }
        cx.notify();
        id
    }

    /// Removes a toast (shown or waiting); the next waiting one takes the slot.
    pub fn dismiss(&mut self, id: ToastId, cx: &mut Context<Self>) {
        self.waiting.retain(|(w, _)| *w != id);
        if let Some(i) = self.shown.iter().position(|s| s.id == id) {
            self.shown.remove(i);
            if let Some((next, toast)) = self.waiting.pop_front() {
                self.show(next, toast, cx);
            }
            cx.notify();
        }
    }

    /// The toasts on screen, oldest first (the newest is lowest).
    pub fn visible(&self) -> &[Shown] {
        &self.shown
    }

    /// How many toasts wait for a slot.
    pub fn queued(&self) -> usize {
        self.waiting.len()
    }

    /// How many timers are armed (one per shown toast that is not held).
    pub fn timers(&self) -> usize {
        self.shown.iter().filter(|s| s.timer.is_some()).count()
    }

    fn show(&mut self, id: ToastId, toast: Toast, cx: &mut Context<Self>) {
        let label: SharedString = match &toast.detail {
            Some(d) => format!("{}. {}", toast.title, d).into(),
            None => toast.title.clone(),
        };
        self.shown.push(Shown {
            id,
            kind: toast.kind,
            title: toast.title.clone(),
            detail: toast.detail.clone(),
            label,
            toast,
            timer: None,
            focus: cx.focus_handle(),
            hovered: false,
            focused: false,
            subs: Vec::new(),
        });
        let last = self.shown.len() - 1;
        self.arm(last, cx);
    }

    /// (Re)starts the timer of the shown toast at `index`; replacing it cancels the old one.
    fn arm(&mut self, index: usize, cx: &mut Context<Self>) {
        let id = self.shown[index].id;
        let stay = self.shown[index].toast.stay();
        self.shown[index].timer = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(stay).await;
            this.update(cx, |s, cx| s.dismiss(id, cx)).ok();
        }));
    }

    /// The pointer or the focus came to (or left) a toast: while either is on
    /// it the timer is gone, and when both have left it starts again in full.
    fn set_hold(&mut self, id: ToastId, hold: Hold, on: bool, cx: &mut Context<Self>) {
        let Some(i) = self.shown.iter().position(|s| s.id == id) else {
            return;
        };
        let s = &mut self.shown[i];
        let flag = match hold {
            Hold::Hover => &mut s.hovered,
            Hold::Focus => &mut s.focused,
        };
        if *flag == on {
            return;
        }
        *flag = on;
        if s.hovered || s.focused {
            s.timer = None;
        } else {
            self.arm(i, cx);
        }
    }

    fn run_action(&mut self, id: ToastId, cx: &mut Context<Self>) {
        let Some(run) = self
            .shown
            .iter()
            .find(|s| s.id == id)
            .and_then(|s| s.toast.action.as_ref().map(|(_, f)| f.clone()))
        else {
            return;
        };
        self.dismiss(id, cx);
        // Later, outside this update: the action may push or dismiss a toast.
        cx.defer(move |cx| run(cx));
    }
}

impl Render for ToastStack {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut col = div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_4)
            .w(metrics::LAYOUT_TOAST_WIDTH);
        if self.shown.is_empty() {
            return col;
        }
        for i in 0..self.shown.len() {
            if self.shown[i].subs.is_empty() {
                let (id, handle) = (self.shown[i].id, self.shown[i].focus.clone());
                let a = cx.on_focus_in(&handle, window, move |this, _, cx| {
                    this.set_hold(id, Hold::Focus, true, cx)
                });
                let b = cx.on_focus_out(&handle, window, move |this, _, _, cx| {
                    this.set_hold(id, Hold::Focus, false, cx)
                });
                self.shown[i].subs = vec![a, b];
            }
        }
        let theme = cx.theme().clone();
        let colors = theme.colors();
        let tokens = themes::parla_tokens(&theme);
        let dismiss_label = match &self.dismiss_label {
            Some(l) => l.clone(),
            None => {
                let l = i18n::gpui::localizer(cx);
                let label: SharedString = t!(l, "toast-dismiss").into_owned().into();
                self.dismiss_label = Some(label.clone());
                label
            }
        };
        for s in &self.shown {
            let id = s.id;
            let n = id.0;
            let (icon, tint, role) = match s.kind {
                ToastKind::Success => (
                    IconName::CheckCircle,
                    theme.status().success,
                    gpui::Role::Status,
                ),
                ToastKind::Info => (IconName::Info, colors.text_accent, gpui::Role::Status),
                ToastKind::Error => (IconName::Alert, theme.status().error, gpui::Role::Alert),
            };
            let mut body = div().flex().flex_col().flex_1().min_w_0().child(
                div()
                    .font_weight(metrics::FONT_WEIGHT_BOLD)
                    .child(s.title.clone()),
            );
            if let Some(d) = &s.detail {
                body = body.child(div().text_color(colors.text_muted).child(d.clone()));
            }
            if let Some((label, _)) = &s.toast.action {
                let hover = colors.element_hover;
                body = body.child(
                    div().flex().mt(metrics::SPACE_3).child(
                        div()
                            .id(("toast-action", n as usize))
                            .debug_selector(move || format!("toast-action-{n}"))
                            .role(gpui::Role::Button)
                            .aria_label(label.clone())
                            .tab_stop(true)
                            .focusable()
                            .focus_visible(controls::focus_style(colors))
                            .flex()
                            .items_center()
                            .min_h(metrics::LAYOUT_TITLEBAR_BUTTON)
                            .px(metrics::SPACE_3)
                            // The label lines up with the title; the padding is hit area.
                            .ml(-metrics::SPACE_3)
                            .rounded(metrics::RADIUS_SM)
                            .cursor_pointer()
                            .hover(move |d| d.bg(hover))
                            .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                            .text_color(colors.text_accent)
                            .child(label.clone())
                            .on_click(cx.listener(move |this, _, _, cx| this.run_action(id, cx))),
                    ),
                );
            }
            let close = controls::icon_button_sized(
                ("toast-close", n as usize),
                IconName::X,
                dismiss_label.clone(),
                controls::IconButtonSize::Small,
                colors,
                {
                    let me = cx.entity().downgrade();
                    move |_, cx| {
                        me.update(cx, |this, cx| this.dismiss(id, cx)).ok();
                    }
                },
            )
            .debug_selector(move || format!("toast-dismiss-{n}"));
            col = col.child(
                div()
                    .id(("toast", n as usize))
                    .debug_selector(move || format!("toast-{n}"))
                    .track_focus(&s.focus)
                    .role(role)
                    .aria_label(s.label.clone())
                    .occlude()
                    .on_hover(cx.listener(move |this, hovered: &bool, _, cx| {
                        this.set_hold(id, Hold::Hover, *hovered, cx)
                    }))
                    .flex()
                    .items_start()
                    .gap(metrics::SPACE_5)
                    .py(metrics::SPACE_5)
                    .pl(metrics::SPACE_5)
                    .pr(metrics::SPACE_4)
                    .rounded(metrics::RADIUS_LG)
                    .bg(colors.elevated_surface_background)
                    .border_1()
                    .border_color(tokens.border_subtle)
                    .shadow_lg()
                    .text_size(metrics::FONT_SIZE_SM)
                    .text_color(colors.text)
                    .child(
                        div()
                            .flex_none()
                            .mt(metrics::SPACE_1)
                            .child(Icon::new(icon).size(metrics::ICON_SIZE_LG).color(tint)),
                    )
                    .child(body)
                    .child(close),
            );
        }
        col
    }
}

/// The stacks of the app, one per window that hosts one (plus a loose one
/// that the first host adopts), in a global.
#[derive(Default)]
struct Registry {
    stacks: Vec<(Option<WindowId>, Entity<ToastStack>)>,
}

impl Global for Registry {}

fn registry(cx: &mut App) -> &mut Registry {
    if !cx.has_global::<Registry>() {
        cx.set_global(Registry::default());
    }
    cx.global_mut::<Registry>()
}

/// Entry points for showing a toast from anywhere.
pub struct Toasts;

impl Toasts {
    /// Puts `url` on the clipboard and says so ("Link copied"): every Copy
    /// link (a message's, a conversation's) goes through here.
    pub fn copy_link(url: String, cx: &mut App) {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(url));
        let l = i18n::gpui::localizer(cx);
        Self::push(
            cx,
            Toast::new(ToastKind::Info, t!(l, "toast-link-copied").into_owned()),
        );
    }

    /// The stack that `window` draws (created on its first call, or adopted
    /// from a toast pushed before any window drew). Windows that closed lose
    /// theirs.
    pub fn host(window: &Window, cx: &mut App) -> Entity<ToastStack> {
        let wid = window.window_handle().window_id();
        let live: Vec<WindowId> = cx.windows().iter().map(|w| w.window_id()).collect();
        let reg = registry(cx);
        reg.stacks
            .retain(|(k, _)| k.is_none_or(|k| k == wid || live.contains(&k)));
        if let Some((_, e)) = reg.stacks.iter().find(|(k, _)| *k == Some(wid)) {
            return e.clone();
        }
        if let Some(slot) = reg.stacks.iter_mut().find(|(k, _)| k.is_none()) {
            slot.0 = Some(wid);
            return slot.1.clone();
        }
        let e = cx.new(ToastStack::new);
        registry(cx).stacks.push((Some(wid), e.clone()));
        e
    }

    /// The bottom-left overlay of `window`: draw it last in the window's root
    /// (which must be `relative`) so it paints over the content.
    pub fn overlay(window: &Window, cx: &mut App) -> gpui::Div {
        div()
            .absolute()
            .bottom(metrics::SPACE_6)
            .left(metrics::SPACE_6)
            .child(Self::host(window, cx))
    }

    /// The stack a push would use now: the active window's, else the first
    /// window's, else the loose one. `None` until something created one.
    pub fn target(cx: &App) -> Option<Entity<ToastStack>> {
        let reg = cx.try_global::<Registry>()?;
        let active = cx.active_window().map(|w| w.window_id());
        reg.stacks
            .iter()
            .find(|(k, _)| k.is_some() && *k == active)
            .or_else(|| reg.stacks.iter().find(|(k, _)| k.is_some()))
            .or_else(|| reg.stacks.first())
            .map(|(_, e)| e.clone())
    }

    /// Shows `toast` in the window that is active (the one the user acted in).
    pub fn push(cx: &mut App, toast: Toast) -> ToastId {
        let stack = match Self::target(cx) {
            Some(s) => s,
            None => {
                let e = cx.new(ToastStack::new);
                registry(cx).stacks.push((None, e.clone()));
                e
            }
        };
        stack.update(cx, |s, cx| s.push(toast, cx))
    }
}
