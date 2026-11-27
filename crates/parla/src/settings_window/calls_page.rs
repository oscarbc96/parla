//! The Calls page of the Settings window (the SettingsCalls board): ringing
//! and its ringtone, how Parla joins a huddle and mute on join, and the
//! microphone and speaker of native calls. Every key is read when a ring or
//! a join happens, so a change applies to the next one; a device change also
//! applies to the running call at once. There is no camera control until
//! Parla carries video.

use components::settings_controls::{
    segment, segmented, select, select_option, select_trigger, settings_row, switch,
};
use gpui::{Context, Window, div, prelude::*};
use settings::JoinWith;
use themes::{ActiveTheme as _, metrics};

use super::{SettingChange, SettingsView, json_text, owned};

/// The ringtones Parla ships: its chime, or none (the banner only).
pub const RINGTONE_CHIME: &str = "parla-chime";
pub const RINGTONE_NONE: &str = "none";

/// One edit a Calls page control makes.
#[derive(Clone, Debug, PartialEq)]
pub enum CallsChange {
    Ring(bool),
    Ringtone(String),
    JoinWith(JoinWith),
    MuteOnJoin(bool),
    /// `None` follows the system default.
    InputDevice(Option<String>),
    OutputDevice(Option<String>),
}

/// Which device select is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceSide {
    Input,
    Output,
}

impl CallsChange {
    pub fn key_path(&self) -> Vec<String> {
        owned(match self {
            Self::Ring(_) => &["calls", "ring"],
            Self::Ringtone(_) => &["calls", "ringtone"],
            Self::JoinWith(_) => &["calls", "join_with"],
            Self::MuteOnJoin(_) => &["calls", "mute_on_join"],
            Self::InputDevice(_) => &["calls", "input_device"],
            Self::OutputDevice(_) => &["calls", "output_device"],
        })
    }

    pub fn raw_json(&self) -> String {
        match self {
            Self::Ring(v) | Self::MuteOnJoin(v) => json_text(v),
            Self::Ringtone(v) => json_text(v),
            Self::JoinWith(v) => json_text(v),
            Self::InputDevice(v) | Self::OutputDevice(v) => json_text(v),
        }
    }
}

impl SettingChange for CallsChange {
    fn path(&self) -> Vec<String> {
        self.key_path()
    }
    fn raw_json(&self) -> String {
        CallsChange::raw_json(self)
    }
}

impl SettingsView {
    pub(super) fn calls_page(
        &mut self,
        window: &mut Window,
        l: &i18n::Localizer,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let colors = *cx.theme().colors();
        let subtle = themes::parla_tokens(cx.theme()).text_subtle;
        let calls = self.settings(cx).calls;

        let ring = self.wire(
            switch("settings-calls-ring", calls.ring, &colors),
            cx,
            |v, _, cx| {
                let on = !v.settings(cx).calls.ring;
                v.apply(CallsChange::Ring(on), cx)
            },
        );
        let silent = calls.ringtone == RINGTONE_NONE;
        let tone = |this: &Self,
                    cx: &mut Context<Self>,
                    id: &'static str,
                    label: String,
                    value: &'static str,
                    selected: bool| {
            let el = segment(id, label, selected, &colors);
            this.wire(el, cx, move |v, _, cx| {
                v.apply(CallsChange::Ringtone(value.into()), cx)
            })
        };
        let tones = segmented(
            [
                tone(
                    self,
                    cx,
                    "settings-calls-ringtone-chime",
                    t!(l, "settings-calls-ringtone-chime").into_owned(),
                    RINGTONE_CHIME,
                    !silent,
                ),
                tone(
                    self,
                    cx,
                    "settings-calls-ringtone-none",
                    t!(l, "settings-calls-ringtone-none").into_owned(),
                    RINGTONE_NONE,
                    silent,
                ),
            ],
            &colors,
        );
        let ringing_body = div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_3)
            .child(settings_row(
                t!(l, "settings-calls-ring").into_owned(),
                None,
                ring,
                &colors,
                subtle,
            ))
            .child(settings_row(
                t!(l, "settings-calls-ringtone").into_owned(),
                None,
                tones,
                &colors,
                subtle,
            ));

        let join = |this: &Self,
                    cx: &mut Context<Self>,
                    id: &'static str,
                    label: String,
                    value: JoinWith| {
            let el = segment(id, label, calls.join_with == value, &colors);
            this.wire(el, cx, move |v, _, cx| {
                v.apply(CallsChange::JoinWith(value), cx)
            })
        };
        let join_with = segmented(
            [
                join(
                    self,
                    cx,
                    "settings-calls-join-native",
                    t!(l, "settings-calls-join-native").into_owned(),
                    JoinWith::Native,
                ),
                join(
                    self,
                    cx,
                    "settings-calls-join-window",
                    t!(l, "settings-calls-join-window").into_owned(),
                    JoinWith::Window,
                ),
                join(
                    self,
                    cx,
                    "settings-calls-join-browser",
                    t!(l, "settings-calls-join-browser").into_owned(),
                    JoinWith::Browser,
                ),
                join(
                    self,
                    cx,
                    "settings-calls-join-provider",
                    t!(l, "settings-calls-join-provider").into_owned(),
                    JoinWith::ProviderApp,
                ),
            ],
            &colors,
        )
        .self_start();
        let mute = self.wire(
            switch("settings-calls-mute", calls.mute_on_join, &colors),
            cx,
            |v, _, cx| {
                let on = !v.settings(cx).calls.mute_on_join;
                v.apply(CallsChange::MuteOnJoin(on), cx)
            },
        );
        let join_body = div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_4)
            .child(join_with)
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_XS)
                    .text_color(subtle)
                    .child(t!(l, "settings-calls-join-help").into_owned()),
            )
            .child(settings_row(
                t!(l, "settings-calls-mute").into_owned(),
                None,
                mute,
                &colors,
                subtle,
            ));

        let input =
            self.device_select(window, l, cx, DeviceSide::Input, calls.input_device.clone());
        let output = self.device_select(
            window,
            l,
            cx,
            DeviceSide::Output,
            calls.output_device.clone(),
        );
        let devices_body = div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_3)
            .child(settings_row(
                t!(l, "settings-calls-input").into_owned(),
                None,
                input,
                &colors,
                subtle,
            ))
            .child(settings_row(
                t!(l, "settings-calls-output").into_owned(),
                None,
                output,
                &colors,
                subtle,
            ));

        div()
            .flex()
            .flex_col()
            .child(Self::page_header(
                t!(l, "settings-calls-title").into_owned(),
                Some(t!(l, "settings-calls-subtitle").into_owned()),
                &colors,
            ))
            .child(div().h(metrics::SPACE_4))
            .child(Self::section(
                l,
                "settings-calls-ringing-title",
                "settings-calls-ringing-desc",
                ringing_body,
                true,
                cx,
            ))
            .child(Self::section(
                l,
                "settings-calls-join-title",
                "settings-calls-join-desc",
                join_body,
                false,
                cx,
            ))
            .child(Self::section(
                l,
                "settings-calls-devices-title",
                "settings-calls-devices-desc",
                devices_body,
                false,
                cx,
            ))
            .pb(metrics::SPACE_8)
    }

    /// A device select: closed it shows the choice (or the system default)
    /// without listing anything; opening it lists the connected devices off
    /// the main thread, once per open.
    fn device_select(
        &mut self,
        window: &mut Window,
        l: &i18n::Localizer,
        cx: &mut Context<Self>,
        side: DeviceSide,
        chosen: Option<String>,
    ) -> gpui::Div {
        let colors = *cx.theme().colors();
        let (id, change): (&'static str, fn(Option<String>) -> CallsChange) = match side {
            DeviceSide::Input => ("settings-calls-input", CallsChange::InputDevice),
            DeviceSide::Output => ("settings-calls-output", CallsChange::OutputDevice),
        };
        let default = t!(l, "settings-calls-device-default").into_owned();
        let open = self.device_open == Some(side);
        let shown = chosen.clone().unwrap_or_else(|| default.clone());
        let trigger = self.wire(
            select_trigger(id, shown, open, &colors),
            cx,
            move |v, _, cx| v.toggle_device_select(side, cx),
        );
        let options = open.then(|| {
            let names = match side {
                DeviceSide::Input => self.device_names.0.clone(),
                DeviceSide::Output => self.device_names.1.clone(),
            };
            let mut options = vec![self.wire(
                select_option(format!("{id}-default"), default, chosen.is_none(), &colors),
                cx,
                move |v, _, cx| v.apply(change(None), cx),
            )];
            for (i, name) in names.into_iter().enumerate() {
                let selected = chosen.as_deref() == Some(name.as_str());
                let el = select_option(format!("{id}-{i}"), name.clone(), selected, &colors);
                options.push(self.wire(el, cx, move |v, _, cx| {
                    v.apply(change(Some(name.clone())), cx)
                }));
            }
            options
        });
        let this = cx.entity();
        select(
            id,
            trigger,
            options,
            &colors,
            move |_, cx| {
                this.update(cx, |v, cx| {
                    v.device_open = None;
                    cx.notify();
                })
            },
            window,
            cx,
        )
    }

    /// Opens the microphone select as a click on it does (the visual
    /// scenario of an open select).
    pub fn open_input_select(&mut self, cx: &mut Context<Self>) {
        if self.device_open != Some(DeviceSide::Input) {
            self.toggle_device_select(DeviceSide::Input, cx);
        }
    }

    fn toggle_device_select(&mut self, side: DeviceSide, cx: &mut Context<Self>) {
        if self.device_open == Some(side) {
            self.device_open = None;
            cx.notify();
            return;
        }
        self.device_open = Some(side);
        let list = crate::call_manager::device_lister(cx);
        cx.spawn(async move |this, cx| {
            let names = cx.background_spawn(async move { list() }).await;
            this.update(cx, |v, cx| {
                v.device_names = names;
                cx.notify();
            })
            .ok();
        })
        .detach();
        cx.notify();
    }
}
