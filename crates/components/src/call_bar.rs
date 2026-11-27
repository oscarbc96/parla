//! The call bar: the running huddle's title, whether it is connecting or
//! live, who is in it (avatars, the speaker ringed and named), Mute, the
//! audio device picker (native calls), Show call window, Leave and (when
//! Parla cannot show what is happening) Open in browser. It is a plain view
//! over a [`CallBarModel`]; the host maps its call state into the model and
//! acts on [`CallBarEvent`]. No timers: it repaints only when the model
//! changes, so a speaking change repaints once, never per audio frame.

use gpui::{
    App, Context, EventEmitter, FocusHandle, IntoElement, KeyDownEvent, Render, SharedString,
    Stateful, Window, div, prelude::*,
};
use themes::{ActiveTheme, metrics, parla_tokens};

use crate::badge::initials;
use crate::controls::{focus_style, on_activate};
use crate::icon::{Icon, IconName};
use crate::reactions::tooltip_shared;

/// `on_activate` in method position, for the bar's button chains.
trait Activate: Sized {
    fn pipe_activate(
        self,
        cx: &Context<CallBar>,
        f: impl Fn(&mut CallBar, &mut Window, &mut Context<CallBar>) + 'static,
    ) -> Self;
}

impl Activate for Stateful<gpui::Div> {
    fn pipe_activate(
        self,
        cx: &Context<CallBar>,
        f: impl Fn(&mut CallBar, &mut Window, &mut Context<CallBar>) + 'static,
    ) -> Self {
        on_activate(self, cx, f)
    }
}

/// Most avatars the bar shows before "+N".
pub const MAX_CALL_AVATARS: usize = 5;

/// Where the call is, as the bar needs it (the host maps its own phases).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallBarPhase {
    /// Fetching, starting or loading: "Connecting...".
    Loading,
    Live,
    Leaving,
}

/// A capture device (microphone or camera) of the call window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaState {
    /// The call window cannot report or control it.
    Unsupported,
    Inactive,
    Active,
    Muted,
}

/// What the bar shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallBarModel {
    pub title: SharedString,
    pub phase: CallBarPhase,
    pub mic: MediaState,
    pub camera: MediaState,
    pub participants: u32,
    /// The call window took long to load.
    pub load_slow: bool,
    /// Someone shares their screen and Parla cannot show it (a native call
    /// receives no video): the bar offers Open in browser.
    pub screen_share_unavailable: bool,
    /// There is a call window to show (the native path has none).
    pub show_window: bool,
    /// Who is in the call, in the order the call lists them; empty when the
    /// host only knows the count.
    pub people: Vec<CallPerson>,
    /// The audio device picker; `None` where Parla does not own the devices
    /// (the call window).
    pub devices: Option<DeviceMenu>,
}

/// One person in the call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallPerson {
    pub name: SharedString,
    pub speaking: bool,
    pub muted: bool,
}

/// The device picker's state: the connected devices (listed when it opens)
/// and the choices; `None` is the system default.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeviceMenu {
    pub open: bool,
    pub inputs: Vec<SharedString>,
    pub outputs: Vec<SharedString>,
    pub input: Option<SharedString>,
    pub output: Option<SharedString>,
}

/// One avatar as painted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallAvatar {
    pub name: SharedString,
    pub initials: SharedString,
    pub speaking: bool,
    pub muted: bool,
    /// The name and, when muted, that it is: the tooltip and the label a
    /// screen reader would use.
    pub label: String,
}

/// One row of the device picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceRow {
    pub label: String,
    /// `None` is the system default.
    pub device: Option<SharedString>,
    pub selected: bool,
}

/// The device picker as painted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceMenuView {
    pub open: bool,
    pub inputs: Vec<DeviceRow>,
    pub outputs: Vec<DeviceRow>,
}

/// What a click on the bar asks the host to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallBarEvent {
    ToggleMute,
    Leave,
    ShowWindow,
    OpenInBrowser,
    /// Opens or closes the device picker.
    ToggleDevices,
    /// A microphone was picked; `None` is the system default.
    PickInput(Option<SharedString>),
    /// A speaker was picked; `None` is the system default.
    PickOutput(Option<SharedString>),
    /// Closes the device picker (Escape, or a click outside it).
    CloseDevices,
}

/// The mute control as it is painted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MuteControl {
    /// A button; `muted` is the current state.
    Button { label: String, muted: bool },
    /// No control: the call window cannot report the microphone.
    InCallWindow(String),
}

/// The bar's content, for painting and tests.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallBarLayout {
    /// "Connecting..." until live, then the participant count.
    pub status: String,
    pub mute: MuteControl,
    /// The camera indicator; `None` when unsupported.
    pub camera_on: Option<bool>,
    pub open_in_browser: bool,
    /// Mute, Show call window and Leave are offered (not while leaving).
    pub controls: bool,
    /// Show call window is offered (a call window exists).
    pub show_window: bool,
    /// Up to [`MAX_CALL_AVATARS`] people.
    pub avatars: Vec<CallAvatar>,
    /// People beyond [`MAX_CALL_AVATARS`].
    pub more: usize,
    pub devices: Option<DeviceMenuView>,
}

pub struct CallBar {
    model: CallBarModel,
    /// The picker was open when the mouse went down on Devices: that press
    /// already closed it (a click outside), so its click must not reopen it.
    open_at_down: bool,
    /// The picker's first row, focused when the picker opens.
    first_row: FocusHandle,
    /// Move the focus into the picker on the next paint.
    focus_picker: bool,
}

impl EventEmitter<CallBarEvent> for CallBar {}

impl CallBar {
    pub fn new(model: CallBarModel, cx: &mut Context<Self>) -> Self {
        Self {
            model,
            open_at_down: false,
            first_row: cx.focus_handle().tab_stop(true),
            focus_picker: false,
        }
    }

    pub fn set_model(&mut self, model: CallBarModel, cx: &mut Context<Self>) {
        if self.model != model {
            let was_open = self.picker_open();
            self.model = model;
            self.focus_picker |= !was_open && self.picker_open();
            cx.notify();
        }
    }

    fn picker_open(&self) -> bool {
        self.model.devices.as_ref().is_some_and(|d| d.open)
    }

    pub fn layout(&self, cx: &App) -> CallBarLayout {
        let l = i18n::Translate::localizer(cx);
        let m = &self.model;
        let status = match m.phase {
            CallBarPhase::Loading => t!(l, "call-bar-connecting").into_owned(),
            CallBarPhase::Live => {
                let mut speakers = m.people.iter().filter(|p| p.speaking);
                match (speakers.next(), speakers.count()) {
                    (Some(p), 0) => {
                        t!(l, "call-bar-speaking", name = p.name.to_string()).into_owned()
                    }
                    (Some(p), more) => t!(
                        l,
                        "call-bar-speaking-more",
                        name = p.name.to_string(),
                        count = more
                    )
                    .into_owned(),
                    (None, _) => {
                        t!(l, "call-bar-participants", count = m.participants).into_owned()
                    }
                }
            }
            CallBarPhase::Leaving => t!(l, "call-bar-leaving").into_owned(),
        };
        let mute = match m.mic {
            MediaState::Unsupported => {
                MuteControl::InCallWindow(t!(l, "call-bar-mute-in-window").into_owned())
            }
            MediaState::Muted => MuteControl::Button {
                label: t!(l, "call-bar-unmute").into_owned(),
                muted: true,
            },
            MediaState::Inactive | MediaState::Active => MuteControl::Button {
                label: t!(l, "call-bar-mute").into_owned(),
                muted: false,
            },
        };
        CallBarLayout {
            status,
            mute,
            camera_on: match m.camera {
                MediaState::Unsupported => None,
                s => Some(s == MediaState::Active),
            },
            open_in_browser: m.load_slow || m.screen_share_unavailable,
            controls: m.phase != CallBarPhase::Leaving,
            show_window: m.show_window,
            avatars: m
                .people
                .iter()
                .take(MAX_CALL_AVATARS)
                .map(|p| CallAvatar {
                    label: if p.muted {
                        t!(l, "call-bar-person-muted", name = p.name.to_string()).into_owned()
                    } else {
                        p.name.to_string()
                    },
                    name: p.name.clone(),
                    initials: initials(&p.name).into(),
                    speaking: p.speaking,
                    muted: p.muted,
                })
                .collect(),
            more: m.people.len().saturating_sub(MAX_CALL_AVATARS),
            devices: m.devices.as_ref().map(|d| {
                let default = t!(l, "call-bar-device-default").into_owned();
                let rows = |names: &[SharedString], chosen: &Option<SharedString>| {
                    std::iter::once(DeviceRow {
                        label: default.clone(),
                        device: None,
                        selected: chosen.is_none(),
                    })
                    .chain(names.iter().map(|n| DeviceRow {
                        label: n.to_string(),
                        device: Some(n.clone()),
                        selected: chosen.as_ref() == Some(n),
                    }))
                    .collect()
                };
                DeviceMenuView {
                    open: d.open,
                    inputs: rows(&d.inputs, &d.input),
                    outputs: rows(&d.outputs, &d.output),
                }
            }),
        }
    }

    pub fn toggle_devices(&mut self, cx: &mut Context<Self>) {
        cx.emit(CallBarEvent::ToggleDevices);
    }

    pub fn close_devices(&mut self, cx: &mut Context<Self>) {
        cx.emit(CallBarEvent::CloseDevices);
    }

    pub fn pick_input(&mut self, device: Option<SharedString>, cx: &mut Context<Self>) {
        cx.emit(CallBarEvent::PickInput(device));
    }

    pub fn pick_output(&mut self, device: Option<SharedString>, cx: &mut Context<Self>) {
        cx.emit(CallBarEvent::PickOutput(device));
    }

    pub fn toggle_mute(&mut self, cx: &mut Context<Self>) {
        cx.emit(CallBarEvent::ToggleMute);
    }

    pub fn leave(&mut self, cx: &mut Context<Self>) {
        cx.emit(CallBarEvent::Leave);
    }

    pub fn show_window(&mut self, cx: &mut Context<Self>) {
        cx.emit(CallBarEvent::ShowWindow);
    }

    pub fn open_in_browser(&mut self, cx: &mut Context<Self>) {
        cx.emit(CallBarEvent::OpenInBrowser);
    }
}

impl CallBar {
    /// The open device picker, under its button: microphones, then speakers.
    /// Every row is a tab stop with the focus ring and picks on click, Enter
    /// or Space; Escape or a press outside closes it.
    fn device_menu(&self, menu: &DeviceMenuView, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = *cx.theme().colors();
        let l = i18n::Translate::localizer(&**cx);
        let heading = |text: String| {
            div()
                .px(metrics::SPACE_4)
                .pt(metrics::SPACE_3)
                .pb(metrics::SPACE_2)
                .text_size(metrics::FONT_SIZE_XS)
                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                .text_color(colors.text_muted)
                .child(text)
        };
        let first = self.first_row.clone();
        let row = |id: (&'static str, usize), r: &DeviceRow| {
            let el = div()
                .id(id)
                .debug_selector(move || format!("{}-{}", id.0, id.1))
                .flex()
                .items_center()
                .justify_between()
                .gap(metrics::SPACE_4)
                .h(metrics::LAYOUT_MENU_ROW)
                .px(metrics::SPACE_4)
                .rounded(metrics::RADIUS_SM)
                .text_size(metrics::FONT_SIZE_SM)
                .text_color(colors.text)
                .cursor_pointer()
                .hover(|h| h.bg(colors.element_hover))
                .focus_visible(focus_style(&colors))
                .when(r.selected, |d| d.bg(colors.element_selected))
                .child(div().min_w_0().truncate().child(r.label.clone()))
                // A fixed slot, so labels line up whether checked or not.
                .child(
                    div()
                        .flex_none()
                        .size(metrics::ICON_SIZE_MD)
                        .when(r.selected, |d| {
                            d.child(
                                Icon::new(IconName::Check)
                                    .size(metrics::ICON_SIZE_MD)
                                    .color(colors.text),
                            )
                        }),
                );
            if id == ("call-bar-input", 0) {
                el.track_focus(&first)
            } else {
                el.tab_stop(true).focusable()
            }
        };
        let mut list = div()
            .id("call-bar-device-menu")
            .debug_selector(|| "call-bar-device-menu".to_string())
            .flex()
            .flex_col()
            .w(metrics::LAYOUT_POPOVER_SM)
            .mt(metrics::SPACE_2)
            .p(metrics::SPACE_2)
            .rounded(metrics::RADIUS_MD)
            .border_1()
            .border_color(colors.border)
            .bg(colors.elevated_surface_background)
            .on_mouse_down_out(cx.listener(|this, _, _, cx| this.close_devices(cx)))
            .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                if e.keystroke.key == "escape" {
                    this.close_devices(cx);
                    cx.stop_propagation();
                }
            }))
            .child(heading(t!(l, "call-bar-device-input").into_owned()));
        for (i, r) in menu.inputs.iter().enumerate() {
            let device = r.device.clone();
            list = list.child(on_activate(
                row(("call-bar-input", i), r),
                cx,
                move |this, _, cx| this.pick_input(device.clone(), cx),
            ));
        }
        list = list.child(heading(t!(l, "call-bar-device-output").into_owned()));
        for (i, r) in menu.outputs.iter().enumerate() {
            let device = r.device.clone();
            list = list.child(on_activate(
                row(("call-bar-output", i), r),
                cx,
                move |this, _, cx| this.pick_output(device.clone(), cx),
            ));
        }
        // Under the Devices button, right-aligned with it, painted above the
        // views below the bar.
        gpui::deferred(div().absolute().top_full().right_0().child(list))
    }
}

/// How a call bar button is filled.
#[derive(Clone, Copy)]
enum Look {
    Neutral,
    /// Leave: solid error fill.
    Danger,
    /// Muted state: error tint with error text.
    Soft,
}

impl Render for CallBar {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let v = self.layout(cx);
        if std::mem::take(&mut self.focus_picker) && self.picker_open() {
            window.focus(&self.first_row, cx);
        }
        let mut device_menu = v
            .devices
            .as_ref()
            .filter(|m| m.open)
            .map(|m| self.device_menu(m, cx).into_any_element());
        let theme = cx.theme().clone();
        let tokens = parla_tokens(&theme);
        let error = theme.status().error;
        let theme_colors = *theme.colors();
        let l = i18n::Translate::localizer(&**cx);
        let dot = if self.model.phase == CallBarPhase::Live {
            tokens.presence_active
        } else {
            tokens.presence_away
        };
        // Neutral controls sit on the sidebar ground; Leave takes the solid
        // error fill with its readable text, a muted Mute the soft error tint.
        let button = |id: &'static str, label: String, look: Look| {
            let d = div()
                .id(id)
                .debug_selector(|| id.to_string())
                .flex_none()
                .px(metrics::SPACE_5)
                .py(metrics::SPACE_2)
                .rounded(metrics::RADIUS_MD)
                .font_weight(metrics::FONT_WEIGHT_MEDIUM)
                .cursor_pointer()
                .tab_stop(true)
                .focusable()
                .focus_visible(focus_style(&theme_colors));
            match look {
                Look::Danger => d
                    .bg(error)
                    .text_color(tokens.danger_text)
                    .hover(|s| s.opacity(0.9)),
                Look::Soft => d
                    .bg(tokens.danger_soft)
                    .text_color(error)
                    .hover(move |s| s.bg(error.opacity(0.24))),
                Look::Neutral => d
                    .bg(tokens.sidebar_hover)
                    .text_color(tokens.sidebar_text_strong)
                    .hover(|s| s.bg(tokens.sidebar_active)),
            }
            .child(label)
        };
        let mut bar = div()
            .debug_selector(|| "call-bar".to_string())
            .flex()
            .items_center()
            .gap(metrics::SPACE_5)
            .px(metrics::SPACE_5)
            .py(metrics::SPACE_4)
            .text_size(metrics::FONT_SIZE_SM)
            .text_color(tokens.sidebar_text)
            .bg(tokens.sidebar_background)
            .rounded(metrics::RADIUS_LG)
            .child(
                div()
                    .debug_selector(|| "call-bar-dot".to_string())
                    .flex_none()
                    .size(metrics::SPACE_4)
                    .rounded_full()
                    .bg(dot),
            )
            .child(
                div()
                    .id("call-bar-title")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .overflow_hidden()
                    // The status may be cut short; the tooltip has all of it.
                    .tooltip(tooltip_shared(
                        format!("{} \u{b7} {}", self.model.title, v.status).into(),
                    ))
                    .child(
                        div()
                            .truncate()
                            .font_weight(metrics::FONT_WEIGHT_BOLD)
                            .text_color(tokens.sidebar_text_strong)
                            .child(self.model.title.clone()),
                    )
                    .child(
                        div()
                            .debug_selector(|| "call-bar-status".to_string())
                            .truncate()
                            .text_size(metrics::FONT_SIZE_XS)
                            .text_color(tokens.sidebar_muted)
                            .child(v.status.clone()),
                    ),
            );
        if !v.avatars.is_empty() {
            // Avatars side by side (no overlap, so a speaker's ring is never
            // covered by a neighbour); the speaker is ringed in signal, the
            // others in the bar's ground. The status line names the speaker
            // and each avatar's tooltip says who and whether muted, so the
            // ring is never the only cue.
            let mut stack = div()
                .debug_selector(|| "call-bar-people".to_string())
                .flex()
                .flex_none()
                .items_center()
                .gap(metrics::SPACE_1);
            for (i, a) in v.avatars.iter().enumerate() {
                let muted = a.muted.then(|| {
                    div()
                        .debug_selector(move || format!("call-bar-avatar-{i}-muted"))
                        .absolute()
                        .right(-metrics::SPACE_2)
                        .bottom(-metrics::SPACE_2)
                        .size(metrics::ICON_SIZE_MD)
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .bg(tokens.sidebar_background)
                        .child(
                            Icon::new(IconName::MicOff)
                                .size(metrics::ICON_SIZE_SM)
                                .color(error),
                        )
                });
                stack = stack.child(
                    div()
                        .id(("call-bar-avatar", i))
                        .debug_selector(move || format!("call-bar-avatar-{i}"))
                        .tooltip(tooltip_shared(a.label.clone().into()))
                        .relative()
                        .size(metrics::LAYOUT_AVATAR_SM)
                        .flex()
                        .items_center()
                        .justify_center()
                        .rounded_full()
                        .border_2()
                        .border_color(if a.speaking {
                            tokens.signal
                        } else {
                            tokens.sidebar_background
                        })
                        .bg(tokens.sidebar_active)
                        .text_color(tokens.sidebar_text_strong)
                        .text_size(metrics::FONT_SIZE_3XS)
                        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                        .child(a.initials.clone())
                        .children(muted),
                );
            }
            if v.more > 0 {
                stack = stack.child(
                    div()
                        .ml(metrics::SPACE_2)
                        .text_size(metrics::FONT_SIZE_XS)
                        .text_color(tokens.sidebar_muted)
                        .child(t!(l, "call-bar-more", count = v.more).into_owned()),
                );
            }
            bar = bar.child(stack);
        }
        if let Some(on) = v.camera_on {
            bar = bar.child(
                div()
                    .debug_selector(|| "call-bar-camera".to_string())
                    .text_size(metrics::FONT_SIZE_XS)
                    .text_color(tokens.sidebar_muted)
                    .child(if on {
                        t!(l, "call-bar-camera-on").into_owned()
                    } else {
                        t!(l, "call-bar-camera-off").into_owned()
                    }),
            );
        }
        if v.controls {
            bar = match v.mute.clone() {
                MuteControl::Button { label, muted } => bar.child(
                    button(
                        "call-bar-mute",
                        label,
                        if muted { Look::Soft } else { Look::Neutral },
                    )
                    .pipe_activate(cx, |this, _, cx| this.toggle_mute(cx)),
                ),
                MuteControl::InCallWindow(text) => bar.child(
                    div()
                        .debug_selector(|| "call-bar-mute-hint".to_string())
                        // The hint shrinks before the title and status do.
                        .min_w_0()
                        .truncate()
                        .text_size(metrics::FONT_SIZE_XS)
                        .text_color(tokens.sidebar_muted)
                        .child(text),
                ),
            };
            if v.devices.is_some() {
                // A press while the picker is open already closed it (a press
                // outside the picker): its click must not open it again.
                let toggle = button(
                    "call-bar-devices",
                    t!(l, "call-bar-devices").into_owned(),
                    Look::Neutral,
                )
                .on_mouse_down(
                    gpui::MouseButton::Left,
                    cx.listener(|this, _, _, _| this.open_at_down = this.picker_open()),
                )
                .on_click(cx.listener(|this, _, _, cx| {
                    if !std::mem::take(&mut this.open_at_down) {
                        this.toggle_devices(cx);
                    }
                }))
                .on_key_down(cx.listener(|this, e: &KeyDownEvent, _, cx| {
                    let k = &e.keystroke;
                    if k.modifiers.platform || k.modifiers.control || k.modifiers.alt {
                        return;
                    }
                    match k.key.as_str() {
                        "enter" | "space" => {
                            this.toggle_devices(cx);
                            cx.stop_propagation();
                        }
                        "escape" if this.picker_open() => {
                            this.close_devices(cx);
                            cx.stop_propagation();
                        }
                        _ => {}
                    }
                }));
                let mut wrap = div().relative().flex_none().child(toggle);
                if let Some(menu) = device_menu.take() {
                    wrap = wrap.child(menu);
                }
                bar = bar.child(wrap);
            }
            if v.show_window {
                bar = bar.child(
                    button(
                        "call-bar-show",
                        t!(l, "call-bar-show-window").into_owned(),
                        Look::Neutral,
                    )
                    .pipe_activate(cx, |this, _, cx| this.show_window(cx)),
                );
            }
            if v.open_in_browser {
                bar = bar.child(
                    button(
                        "call-bar-browser",
                        t!(l, "call-bar-open-in-browser").into_owned(),
                        Look::Neutral,
                    )
                    .pipe_activate(cx, |this, _, cx| this.open_in_browser(cx)),
                );
            }
            bar = bar.child(
                button(
                    "call-bar-leave",
                    t!(l, "call-bar-leave").into_owned(),
                    Look::Danger,
                )
                .pipe_activate(cx, |this, _, cx| this.leave(cx)),
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
    }
}
