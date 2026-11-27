//! The "Set a status" dialog: an emoji, a text and when to clear it.

use domain::{EmojiRef, Status};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render, Window, div,
    prelude::*,
};
use i18n::Localizer;
use jiff::ZonedDateTime;
use themes::ActiveTheme as _;

use crate::controls::ButtonKind;
use crate::dnd_menu::parse_custom_minutes;
use crate::emoji_picker::{EmojiPicked, EmojiPicker, PickerDismissed};
use crate::text_input::TextInput;

/// Key context of the dialog.
pub const KEY_CONTEXT: &str = "StatusDialog";

/// When the status clears.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExpiryChoice {
    Never,
    Minutes(u32),
    /// The end of the day, in the viewer's zone.
    Today,
    /// The end of the week (Sunday), in the viewer's zone.
    ThisWeek,
    /// Minutes typed in the dialog's field.
    Custom,
}

/// One row of the "clear after" list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExpiryRow {
    pub choice: ExpiryChoice,
    pub label: String,
}

/// The choices in menu order.
pub fn expiry_rows(l: &Localizer) -> Vec<ExpiryRow> {
    let row = |choice, label: std::borrow::Cow<'_, str>| ExpiryRow {
        choice,
        label: label.into_owned(),
    };
    vec![
        row(ExpiryChoice::Never, t!(l, "status-expiry-never")),
        row(ExpiryChoice::Minutes(30), t!(l, "status-expiry-30m")),
        row(ExpiryChoice::Minutes(60), t!(l, "status-expiry-1h")),
        row(ExpiryChoice::Minutes(240), t!(l, "status-expiry-4h")),
        row(ExpiryChoice::Today, t!(l, "status-expiry-today")),
        row(ExpiryChoice::ThisWeek, t!(l, "status-expiry-week")),
        row(ExpiryChoice::Custom, t!(l, "status-expiry-custom")),
    ]
}

/// A ready-made status: an emoji, a text and when it clears.
#[derive(Clone, Debug, PartialEq)]
pub struct StatusPreset {
    pub emoji: EmojiRef,
    pub text: String,
    pub choice: ExpiryChoice,
}

/// The presets in dialog order. "In a meeting" clears after an hour: Parla
/// reads no calendar.
pub fn status_presets(l: &Localizer) -> Vec<StatusPreset> {
    let p =
        |shortcode: &str, unicode: &str, text: std::borrow::Cow<'_, str>, choice| StatusPreset {
            emoji: EmojiRef {
                shortcode: shortcode.into(),
                unicode: Some(unicode.into()),
                skin_tone: None,
            },
            text: text.into_owned(),
            choice,
        };
    vec![
        p(
            "headphones",
            "\u{1f3a7}",
            t!(l, "status-preset-focusing"),
            ExpiryChoice::Minutes(60),
        ),
        p(
            "calendar",
            "\u{1f4c5}",
            t!(l, "status-preset-meeting"),
            ExpiryChoice::Minutes(60),
        ),
        p(
            "bus",
            "\u{1f68c}",
            t!(l, "status-preset-commuting"),
            ExpiryChoice::Minutes(30),
        ),
        p(
            "face_with_thermometer",
            "\u{1f912}",
            t!(l, "status-preset-sick"),
            ExpiryChoice::Today,
        ),
        p(
            "palm_tree",
            "\u{1f334}",
            t!(l, "status-preset-holiday"),
            ExpiryChoice::Never,
        ),
    ]
}

/// Midnight at the end of the civil day `days` after today, in the zone of `now`.
fn midnight_after(now: &ZonedDateTime, days: i64) -> Option<i64> {
    let date = now.date().checked_add(jiff::Span::new().days(days)).ok()?;
    let at = date
        .to_datetime(jiff::civil::Time::midnight())
        .to_zoned_datetime(now.time_zone().clone())
        .ok()?;
    Some(at.timestamp().as_second())
}

/// When `choice` clears, from `now` (unix seconds). The outer `None` is an
/// invalid choice (a custom duration that does not parse).
pub fn expiry_at(
    choice: ExpiryChoice,
    now: &ZonedDateTime,
    custom: Option<u32>,
) -> Option<Option<i64>> {
    let secs = now.timestamp().as_second();
    match choice {
        ExpiryChoice::Never => Some(None),
        ExpiryChoice::Minutes(m) => Some(Some(secs + i64::from(m) * 60)),
        ExpiryChoice::Today => midnight_after(now, 1).map(Some),
        ExpiryChoice::ThisWeek => {
            // Monday is 1 and Sunday 7: the week ends at the end of Sunday.
            let left = 7 - i64::from(now.weekday().to_monday_one_offset());
            midnight_after(now, left + 1).map(Some)
        }
        ExpiryChoice::Custom => custom
            .filter(|m| *m > 0)
            .map(|m| Some(secs + i64::from(m) * 60)),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum StatusDialogEvent {
    /// Save this status (`None` clears it).
    Save(Option<Status>),
    Cancel,
}

pub struct StatusDialog {
    text: Entity<TextInput>,
    custom: Entity<TextInput>,
    emoji: Option<EmojiRef>,
    choice: ExpiryChoice,
    picker: Option<Entity<EmojiPicker>>,
    _picker_subs: Vec<gpui::Subscription>,
    focus: FocusHandle,
}

impl EventEmitter<StatusDialogEvent> for StatusDialog {}

impl Focusable for StatusDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl StatusDialog {
    /// `current` prefills the emoji, text and any running expiry.
    pub fn new(current: Option<&Status>, cx: &mut Context<Self>) -> Self {
        let text = cx.new(TextInput::new);
        let custom = cx.new(TextInput::new);
        let l = i18n::gpui::localizer(cx);
        let ph = t!(l, "status-custom-placeholder").into_owned();
        custom.update(cx, |t, _| t.set_placeholder(ph));
        let ph = t!(l, "status-text-placeholder").into_owned();
        text.update(cx, |t, _| t.set_placeholder(ph));
        let mut choice = ExpiryChoice::Never;
        if let Some(s) = current {
            text.update(cx, |t, cx| t.set_text(&s.text, cx));
            // Keep a running expiry as a custom number of minutes (rounded up).
            if let Some(left) = s
                .expires
                .map(|e| e - crate::now_secs(cx))
                .filter(|l| *l > 0)
            {
                choice = ExpiryChoice::Custom;
                let minutes = (left + 59) / 60;
                custom.update(cx, |t, cx| t.set_text(&minutes.to_string(), cx));
            }
        }
        Self {
            text,
            custom,
            emoji: current.and_then(|s| s.emoji.clone()),
            choice,
            picker: None,
            _picker_subs: Vec::new(),
            focus: cx.focus_handle(),
        }
    }

    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.text.update(cx, |t, cx| t.set_text(text, cx));
    }

    pub fn set_emoji(&mut self, e: Option<EmojiRef>, cx: &mut Context<Self>) {
        self.emoji = e;
        cx.notify();
    }

    pub fn choose(&mut self, c: ExpiryChoice, cx: &mut Context<Self>) {
        self.choice = c;
        cx.notify();
    }

    /// Fills the emoji, the text and the expiry from the `ix`th preset.
    pub fn apply_preset(&mut self, ix: usize, cx: &mut Context<Self>) {
        let l = i18n::gpui::localizer(cx);
        let Some(p) = status_presets(&l).into_iter().nth(ix) else {
            return;
        };
        self.text.update(cx, |t, cx| t.set_text(&p.text, cx));
        self.emoji = Some(p.emoji);
        self.choice = p.choice;
        cx.notify();
    }

    pub fn set_custom(&mut self, text: &str, cx: &mut Context<Self>) {
        self.custom.update(cx, |t, cx| t.set_text(text, cx));
    }

    /// The status the dialog would save; `None` when it has no text and no emoji.
    pub fn status(&self, cx: &App) -> Option<Status> {
        let text = self.text.read(cx).text().trim().to_string();
        if text.is_empty() && self.emoji.is_none() {
            return None;
        }
        Some(Status {
            emoji: self.emoji.clone(),
            text: text.into(),
            expires: self.expires(cx).flatten(),
        })
    }

    /// When the chosen duration ends; the outer `None` while it is invalid.
    fn expires(&self, cx: &App) -> Option<Option<i64>> {
        let now = crate::now(cx).to_zoned_datetime(crate::time_zone(cx));
        let custom = parse_custom_minutes(self.custom.read(cx).text());
        expiry_at(self.choice, &now, custom)
    }

    /// Whether the chosen duration can be saved.
    pub fn valid(&self, cx: &App) -> bool {
        self.expires(cx).is_some()
    }

    /// Saves the status; does nothing while the chosen duration is invalid.
    /// No text and no emoji clears the status.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        if !self.valid(cx) {
            return;
        }
        let s = self.status(cx);
        cx.emit(StatusDialogEvent::Save(s));
    }

    /// Clears the status.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        cx.emit(StatusDialogEvent::Save(None));
    }

    /// Opens or closes the emoji picker.
    pub fn toggle_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.picker.take().is_some() {
            window.focus(&self.focus, cx);
            cx.notify();
            return;
        }
        let picker = cx.new(|cx| EmojiPicker::new(Vec::new(), false, cx));
        let focus = picker.read(cx).focus_handle(cx);
        self._picker_subs = vec![
            cx.subscribe(&picker, |this, _, ev: &EmojiPicked, cx| {
                this.emoji = Some(ev.0.clone());
                this.picker = None;
                cx.notify();
            }),
            cx.subscribe(&picker, |this, _, _: &PickerDismissed, cx| {
                this.picker = None;
                cx.notify();
            }),
        ];
        self.picker = Some(picker);
        window.focus(&focus, cx);
        cx.notify();
    }

    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        cx.emit(StatusDialogEvent::Cancel);
    }
}

impl Render for StatusDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The host puts the dialog `LAYOUT_DIALOG_TOP` below the window top;
        // it ends a margin above the bottom, its body scrolling between the
        // pinned title and buttons.
        let max_h = (window.viewport_size().height
            - themes::metrics::LAYOUT_DIALOG_TOP
            - themes::metrics::SPACE_9)
            .max(themes::metrics::LAYOUT_DIALOG_TOP);
        let theme = cx.theme().clone();
        let colors = *theme.colors();
        let l = i18n::gpui::localizer(cx);
        let invalid = !self.valid(cx);
        let tokens = themes::parla_tokens(&theme);
        // `element_hover` equals the raised ground in Parla Dark, and
        // `element_selected` equals `accent_soft` in Parla Light.
        let hover = colors.element_active;
        let this = cx.entity();
        let field = |input: Entity<TextInput>, sel: &'static str| {
            crate::controls::field_frame(false, &colors, tokens.border_control)
                .debug_selector(move || sel.to_string())
                .flex_1()
                .child(input)
        };
        let emoji_label = self
            .emoji
            .as_ref()
            .map_or_else(|| "+".to_string(), crate::reactions::glyph);
        let v = this.clone();
        let emoji_button = crate::controls::field_frame(false, &colors, tokens.border_control)
            .id("status-emoji")
            .debug_selector(|| "status-emoji".into())
            .cursor_pointer()
            .hover(move |d| d.bg(hover))
            .child(emoji_label)
            .on_click(move |_, w, cx| v.update(cx, |d, cx| d.toggle_picker(w, cx)));
        let mut rows = div()
            .id("status-rows")
            .flex()
            .flex_col()
            .gap(themes::metrics::SPACE_1);
        for (ix, r) in expiry_rows(&l).into_iter().enumerate() {
            let on = self.choice == r.choice;
            let v = this.clone();
            let choice = r.choice;
            rows = rows.child(
                div()
                    .id(("status-expiry", ix))
                    .debug_selector(move || format!("status-expiry-{ix}"))
                    .flex()
                    .items_center()
                    .gap(themes::metrics::SPACE_5)
                    .h(themes::metrics::LAYOUT_ROW_COZY)
                    .px(themes::metrics::SPACE_5)
                    .rounded(themes::metrics::RADIUS_MD)
                    .cursor_pointer()
                    .text_size(themes::metrics::FONT_SIZE_MD)
                    .when(!on, |d| d.hover(move |d| d.bg(hover)))
                    .when(on, |d| {
                        d.bg(colors.accent_soft)
                            .text_color(tokens.accent_soft_text)
                            .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                    })
                    .child(div().w(themes::metrics::SPACE_5).children(on.then(|| {
                        crate::icon::Icon::new(crate::icon::IconName::Check)
                            .size(themes::metrics::ICON_SIZE_SM)
                            .color(tokens.accent_soft_text)
                    })))
                    .child(r.label)
                    .on_click(move |_, _, cx| v.update(cx, |d, cx| d.choose(choice, cx))),
            );
        }
        let current = self.status(cx);
        let expiry_label = |c: ExpiryChoice| {
            expiry_rows(&l)
                .into_iter()
                .find(|r| r.choice == c)
                .map(|r| r.label)
                .unwrap_or_default()
        };
        let mut presets = div()
            .debug_selector(|| "status-presets".into())
            .flex()
            .flex_col()
            .gap(themes::metrics::SPACE_1);
        for (ix, p) in status_presets(&l).into_iter().enumerate() {
            let on = current
                .as_ref()
                .is_some_and(|s| s.emoji.as_ref() == Some(&p.emoji) && *s.text == *p.text)
                && self.choice == p.choice;
            let v = this.clone();
            presets = presets.child(crate::controls::on_activate_app(
                div()
                    .id(("status-preset", ix))
                    .debug_selector(move || format!("status-preset-{ix}"))
                    .flex()
                    .items_center()
                    .gap(themes::metrics::SPACE_5)
                    .h(themes::metrics::LAYOUT_PRESET_ROW)
                    .px(themes::metrics::SPACE_5)
                    .rounded(themes::metrics::RADIUS_MD)
                    .cursor_pointer()
                    .text_size(themes::metrics::FONT_SIZE_MD)
                    .when(!on, |d| d.hover(move |d| d.bg(hover)))
                    .when(on, |d| {
                        d.bg(colors.accent_soft).text_color(tokens.accent_soft_text)
                    })
                    .child(
                        div()
                            .w(themes::metrics::SPACE_8)
                            .flex()
                            .justify_center()
                            .text_size(themes::metrics::FONT_SIZE_LG)
                            .child(crate::reactions::glyph(&p.emoji)),
                    )
                    .child(div().flex_1().min_w_0().child(p.text))
                    .child(
                        div()
                            .text_size(themes::metrics::FONT_SIZE_XS)
                            .text_color(if on {
                                tokens.accent_soft_text
                            } else {
                                tokens.text_subtle
                            })
                            .child(expiry_label(p.choice)),
                    )
                    .tab_stop(true)
                    .focusable()
                    .focus_visible(crate::controls::focus_style(&colors)),
                move |_, cx| v.update(cx, |d, cx| d.apply_preset(ix, cx)),
            ));
        }
        let button = |id: &'static str, label: String, accent: bool| {
            let kind = if accent {
                ButtonKind::Primary
            } else {
                ButtonKind::Secondary
            };
            crate::controls::no_focus_on_click(
                crate::controls::button(id, label, kind, true, &colors)
                    .debug_selector(move || id.to_string()),
            )
        };
        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .on_key_down(cx.listener(|this, ev: &gpui::KeyDownEvent, _, cx| {
                match ev.keystroke.key.as_str() {
                    "escape" => this.cancel(cx),
                    "enter" => this.save(cx),
                    _ => {}
                }
            }))
            .flex()
            .flex_col()
            .max_h(max_h)
            .gap(themes::metrics::SPACE_4)
            .p(themes::metrics::SPACE_6)
            .bg(colors.elevated_surface_background)
            .border_1()
            .border_color(colors.border)
            .rounded(themes::metrics::RADIUS_LG)
            .shadow_md()
            .text_color(colors.text)
            .child(
                div()
                    .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                    .text_size(themes::metrics::FONT_SIZE_2XL)
                    .child(t!(l, "status-title").into_owned()),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .child(emoji_button)
                    .child(field(self.text.clone(), "status-text")),
            )
            .children(self.picker.clone())
            .child(
                div()
                    .id("status-body")
                    .debug_selector(|| "status-body".into())
                    .flex()
                    .flex_col()
                    .gap(themes::metrics::SPACE_4)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(presets)
                    .child(
                        div()
                            .mt(themes::metrics::SPACE_4)
                            .text_size(themes::metrics::FONT_SIZE_XS)
                            .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                            .text_color(colors.text_muted)
                            .child(t!(l, "status-clear-after").into_owned()),
                    )
                    .child(rows)
                    .when(self.choice == ExpiryChoice::Custom, |d| {
                        d.child(field(self.custom.clone(), "status-custom"))
                            .when(invalid, |d| {
                                d.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.status().error)
                                        .child(t!(l, "status-custom-hint").into_owned()),
                                )
                            })
                    }),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .gap_2()
                    .justify_end()
                    .child(crate::controls::on_activate(
                        button("status-clear", t!(l, "status-clear").into_owned(), false),
                        cx,
                        |d, _, cx| d.clear(cx),
                    ))
                    .child(crate::controls::on_activate(
                        button("status-cancel", t!(l, "status-cancel").into_owned(), false),
                        cx,
                        |d, _, cx| d.cancel(cx),
                    ))
                    .child(crate::controls::on_activate(
                        button("status-save", t!(l, "status-save").into_owned(), true),
                        cx,
                        |d, _, cx| d.save(cx),
                    )),
            )
    }
}
