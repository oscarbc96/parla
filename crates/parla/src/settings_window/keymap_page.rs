//! The Keymap page of the Settings window (the SettingsKeymap board): every
//! action with its live binding, a search over titles and keys, All/Changed
//! filters, and per-row editing. Recording a binding captures the next
//! keystroke (before any binding runs), warns when the keys already run
//! another action in the same context, and writes `keymap.json` through
//! [`settings::edit_keymap_file`]; the keymap is rebound at once.

use components::controls::{ButtonKind, button, button_small, compact, on_activate_app};
use components::icon::{Icon, IconName};
use components::settings_controls::{segment, segmented};
use components::shortcuts_sheet::key_caps;
use components::text_input::{TextInput, TextInputEvent};
use gpui::{
    Context, Entity, FocusHandle, Focusable as _, SharedString, Subscription, Window, div,
    prelude::*,
};
use themes::{ActiveTheme as _, metrics};

use super::SettingsView;
use std::rc::Rc;

use crate::shortcuts::{self, Shortcut, Source};

/// A binding being recorded: the row and the keys pressed so far.
pub(super) struct Recording {
    row: Shortcut,
    keys: Option<String>,
    _intercept: Subscription,
}

/// The Keymap page's own state, made on first use.
pub(super) struct KeymapUi {
    search: Entity<TextInput>,
    record_focus: FocusHandle,
    recording: Option<Recording>,
    filter: Filter,
    /// The rows of the user keymap text they were built from.
    rows: Option<(String, Rc<Vec<Shortcut>>)>,
    _subs: Vec<Subscription>,
}

/// Which rows the page lists.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Filter {
    All,
    Changed,
    Conflicts,
}

/// The id of a row's control: the action, plus the keys for a second binding.
fn row_id(prefix: &str, row: &Shortcut, first: bool) -> String {
    if first {
        format!("{prefix}-{}", row.action)
    } else {
        format!(
            "{prefix}-{}-{}",
            row.action,
            row.keystrokes.as_deref().unwrap_or("none")
        )
    }
}

impl SettingsView {
    fn keymap_path(&self) -> std::path::PathBuf {
        self.path.with_file_name("keymap.json")
    }

    /// The user keymap text: the one bound now, else the file read once.
    fn keymap_text(&self, cx: &mut Context<Self>) -> String {
        if cx.try_global::<shortcuts::LiveKeymap>().is_none() {
            let text = std::fs::read_to_string(self.keymap_path()).unwrap_or_default();
            cx.set_global(shortcuts::LiveKeymap { user: text });
        }
        cx.global::<shortcuts::LiveKeymap>().user.clone()
    }

    fn keymap_ui(&mut self, l: &i18n::Localizer, cx: &mut Context<Self>) -> &mut KeymapUi {
        if self.keymap.is_none() {
            let search = cx.new(TextInput::new);
            let ph = t!(l, "settings-keymap-search").into_owned();
            search.update(cx, |t, _| {
                t.set_single_line(true);
                t.set_placeholder(ph);
            });
            let subs = vec![cx.subscribe(&search, |_, _, _: &TextInputEvent, cx| cx.notify())];
            self.keymap = Some(KeymapUi {
                search,
                record_focus: cx.focus_handle(),
                recording: None,
                filter: Filter::All,
                rows: None,
                _subs: subs,
            });
        }
        self.keymap.as_mut().expect("made above")
    }

    /// Replaces the search text (tests and the page).
    pub fn set_keymap_query(&mut self, q: &str, cx: &mut Context<Self>) {
        let l = i18n::gpui::localizer(cx);
        let search = self.keymap_ui(&l, cx).search.clone();
        search.update(cx, |t, cx| t.set_text(q, cx));
        cx.notify();
    }

    /// The keys being recorded and the conflicting action, for tests.
    pub fn keymap_recording(&self) -> Option<(String, Option<String>)> {
        let r = self.keymap.as_ref()?.recording.as_ref()?;
        Some((r.row.action.clone(), r.keys.clone()))
    }

    fn start_recording(&mut self, row: Shortcut, window: &mut Window, cx: &mut Context<Self>) {
        let l = i18n::gpui::localizer(cx);
        let ui = self.keymap_ui(&l, cx);
        let focus = ui.record_focus.clone();
        let window_id = window.window_handle().window_id();
        let this = cx.entity().downgrade();
        let watched = focus.clone();
        // Runs before key bindings, so a recorded cmd-q does not quit.
        let intercept = cx.intercept_keystrokes(move |ev, window, cx| {
            if window.window_handle().window_id() != window_id || !watched.is_focused(window) {
                return;
            }
            let k = &ev.keystroke;
            let plain = !k.modifiers.platform
                && !k.modifiers.control
                && !k.modifiers.alt
                && !k.modifiers.shift;
            if plain && k.key == "tab" {
                return;
            }
            let keys = k.unparse(cx.platform_info().os);
            let escape = plain && k.key == "escape";
            let enter = plain && k.key == "enter";
            this.update(cx, |v, cx| {
                if escape {
                    v.cancel_recording(cx);
                } else if enter && v.keymap_recording().is_some_and(|(_, k)| k.is_some()) {
                    v.save_recording(cx);
                } else {
                    v.recorded(keys, cx);
                }
            })
            .ok();
            cx.stop_propagation();
        });
        ui.recording = Some(Recording {
            row,
            keys: None,
            _intercept: intercept,
        });
        window.focus(&focus, cx);
        cx.notify();
    }

    /// A keystroke arrived while recording: a second one makes a sequence
    /// (`cmd-k cmd-s`), a third starts over.
    pub fn recorded(&mut self, keys: String, cx: &mut Context<Self>) {
        if let Some(r) = self.keymap.as_mut().and_then(|k| k.recording.as_mut()) {
            r.keys = Some(match r.keys.take() {
                Some(first) if !first.contains(' ') => [first, keys].join(" "),
                _ => keys,
            });
            cx.notify();
        }
    }

    /// Starts recording a new binding for the first row of `action` (the
    /// page and the visual tests).
    pub fn begin_recording(&mut self, action: &str, window: &mut Window, cx: &mut Context<Self>) {
        let user = self.keymap_text(cx);
        let row = self
            .keymap_rows(&user, cx)
            .iter()
            .find(|r| r.action == action)
            .cloned();
        if let Some(row) = row {
            self.start_recording(row, window, cx);
        }
    }

    /// The rows for the user keymap `user`, parsed again only when it changed.
    fn keymap_rows(&mut self, user: &str, cx: &mut Context<Self>) -> Rc<Vec<Shortcut>> {
        let l = i18n::gpui::localizer(cx);
        let ui = self.keymap_ui(&l, cx);
        match &ui.rows {
            Some((text, rows)) if text == user => rows.clone(),
            _ => {
                let rows = Rc::new(shortcuts::shortcuts(shortcuts::DEFAULT_KEYMAP, user));
                ui.rows = Some((user.to_string(), rows.clone()));
                rows
            }
        }
    }

    fn cancel_recording(&mut self, cx: &mut Context<Self>) {
        if let Some(k) = self.keymap.as_mut() {
            k.recording = None;
        }
        cx.notify();
    }

    /// Writes `edit` of the user keymap and rebinds it.
    fn write_keymap(&mut self, edit: impl FnOnce(&str) -> Option<String>, cx: &mut Context<Self>) {
        let path = self.keymap_path();
        let result = settings::edit_keymap_file(&path, edit);
        let l = i18n::gpui::localizer(cx);
        match result {
            Ok(true) => {
                self.error = None;
                let text = std::fs::read_to_string(&path).unwrap_or_default();
                crate::app_state::keymap_written(cx, text);
            }
            Ok(false) => {
                crate::log::error("keymap: the file is not a keymap array");
                self.error = Some(t!(l, "settings-keymap-error-not-array").into_owned());
            }
            Err(e) => {
                crate::log::error(&format!("keymap: cannot write: {e}"));
                self.error =
                    Some(t!(l, "settings-error-write", reason = e.to_string()).into_owned());
            }
        }
        cx.notify();
    }

    /// Saves the recorded keys for the recorded row.
    pub fn save_recording(&mut self, cx: &mut Context<Self>) {
        let Some(r) = self.keymap.as_mut().and_then(|k| k.recording.take()) else {
            return;
        };
        let Some(keys) = r.keys.clone() else {
            return;
        };
        if shortcuts::needs_modifier(&keys, r.row.context.as_deref()) {
            // Refused: keep recording so another key can be pressed.
            if let Some(k) = self.keymap.as_mut() {
                k.recording = Some(r);
            }
            return;
        }
        let row = r.row;
        self.write_keymap(move |t| shortcuts::rebind(t, &row, &keys), cx);
    }

    fn reset_row(&mut self, row: Shortcut, cx: &mut Context<Self>) {
        self.write_keymap(
            move |t| shortcuts::reset(t, shortcuts::DEFAULT_KEYMAP, &row),
            cx,
        );
    }

    pub(super) fn keymap_page(
        &mut self,
        window: &mut Window,
        l: &i18n::Localizer,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let colors = *cx.theme().colors();
        let tokens = themes::parla_tokens(cx.theme());
        let status = *cx.theme().status();
        let mac = cfg!(target_os = "macos");
        let user = self.keymap_text(cx);
        let rows = self.keymap_rows(&user, cx);
        let ui = self.keymap_ui(l, cx);
        let search = ui.search.clone();
        let record_focus = ui.record_focus.clone();
        let filter = ui.filter;
        let recording = ui
            .recording
            .as_ref()
            .map(|r| (r.row.clone(), r.keys.clone()));
        let query = search.read(cx).text().to_string();
        let title =
            |r: &Shortcut| crate::actions::title(l, &r.action).unwrap_or_else(|| r.action.clone());
        let shown: Vec<&Shortcut> = rows
            .iter()
            .filter(|r| match filter {
                Filter::All => true,
                Filter::Changed => r.source == Source::User,
                Filter::Conflicts => shortcuts::in_conflict(&rows, r),
            })
            .filter(|r| shortcuts::matches(r, &title(r), &query, mac))
            .collect();
        let total = {
            let mut names: Vec<&str> = rows.iter().map(|r| r.action.as_str()).collect();
            names.dedup();
            names.len()
        };

        let open = {
            let el = compact(button(
                "settings-keymap-open-file",
                t!(l, "settings-keymap-open-file").into_owned(),
                ButtonKind::Secondary,
                true,
                &colors,
            ))
            .debug_selector(|| "settings-keymap-open-file".into());
            on_activate_app(el, |window, cx| {
                window.dispatch_action(Box::new(crate::actions::OpenKeymap), cx);
            })
        };
        let head = div()
            .flex()
            .items_start()
            .gap(metrics::SPACE_6)
            .child(div().flex_1().min_w_0().child(Self::page_header(
                t!(l, "settings-page-keymap").into_owned(),
                Some(t!(l, "settings-keymap-subtitle").into_owned()),
                &colors,
            )))
            .child(open);

        let focused = search.read(cx).focus_handle(cx).is_focused(window);
        let field = div()
            .debug_selector(|| "settings-keymap-search".into())
            .flex()
            .flex_1()
            .min_w(metrics::LAYOUT_FIELD_MD)
            .items_center()
            .gap(metrics::SPACE_3)
            .h(metrics::LAYOUT_INPUT_MIN)
            .px(metrics::SPACE_4)
            .rounded(metrics::RADIUS_MD)
            .border_1()
            .border_color(tokens.border_control)
            .bg(colors.surface_background)
            .text_size(metrics::FONT_SIZE_SM)
            .when(focused, |d| {
                d.border_color(gpui::transparent_black())
                    .shadow(components::controls::ring(&colors))
            })
            .child(
                Icon::new(IconName::Search)
                    .size(metrics::ICON_SIZE_BASE)
                    .color(tokens.text_subtle),
            )
            .child(div().flex_1().min_w_0().child(search));
        let conflicts = rows
            .iter()
            .filter(|r| shortcuts::in_conflict(&rows, r))
            .count();
        let filters = segmented(
            [
                (
                    Filter::All,
                    "settings-keymap-filter-all",
                    t!(l, "settings-keymap-filter-all").into_owned(),
                ),
                (
                    Filter::Changed,
                    "settings-keymap-filter-changed",
                    t!(l, "settings-keymap-filter-changed").into_owned(),
                ),
                (
                    Filter::Conflicts,
                    "settings-keymap-filter-conflicts",
                    t!(l, "settings-keymap-filter-conflicts", count = conflicts).into_owned(),
                ),
            ]
            .map(|(f, id, label)| {
                let el =
                    segment(id, label, filter == f, &colors).debug_selector(move || id.to_string());
                self.wire(el, cx, move |v, _, cx| {
                    if let Some(k) = v.keymap.as_mut() {
                        k.filter = f;
                    }
                    cx.notify();
                })
            }),
            &colors,
        );
        let toolbar = div()
            .flex()
            .flex_wrap()
            .items_center()
            .gap(metrics::SPACE_5)
            .child(field)
            .child(
                div()
                    .debug_selector(|| "settings-keymap-count".into())
                    .flex_none()
                    .text_size(metrics::FONT_SIZE_XS)
                    .text_color(colors.text_muted)
                    .child(
                        t!(
                            l,
                            "settings-keymap-count",
                            shown = {
                                let mut names: Vec<&str> =
                                    shown.iter().map(|r| r.action.as_str()).collect();
                                names.dedup();
                                names.len()
                            },
                            total = total
                        )
                        .into_owned(),
                    ),
            )
            .child(filters);

        let cell_head = |label: String| {
            div()
                .text_size(metrics::FONT_SIZE_2XS)
                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                .text_color(tokens.text_subtle)
                .child(label.to_uppercase())
        };
        let header = div()
            .flex()
            .items_center()
            .gap(metrics::SPACE_5)
            .px(metrics::SPACE_5)
            .py(metrics::SPACE_4)
            .bg(tokens.bg_sunken)
            .border_b_1()
            .border_color(colors.border)
            .child(
                cell_head(t!(l, "settings-keymap-col-action").into_owned())
                    .flex_1()
                    .min_w(metrics::LAYOUT_FIELD_XS),
            )
            .child(
                cell_head(t!(l, "settings-keymap-col-shortcut").into_owned())
                    .w(metrics::LAYOUT_FIELD_SM)
                    .min_w(metrics::LAYOUT_FIELD_XS)
                    .overflow_hidden(),
            )
            .child(
                cell_head(t!(l, "settings-keymap-col-context").into_owned())
                    .w(metrics::LAYOUT_FIELD_XS)
                    .min_w_0()
                    .overflow_hidden(),
            )
            .child(
                cell_head(t!(l, "settings-keymap-col-source").into_owned())
                    .w(metrics::LAYOUT_FIELD_XS)
                    .min_w_0()
                    .overflow_hidden(),
            )
            .child(div().w(metrics::LAYOUT_COLUMN_WIDE).min_w_0());

        let mut table = div()
            .debug_selector(|| "settings-keymap-table".into())
            .flex()
            .flex_col()
            .rounded(metrics::RADIUS_LG)
            .border_1()
            .border_color(colors.border)
            .bg(colors.surface_background)
            .overflow_hidden()
            .child(header);
        let mut seen: Vec<&str> = Vec::new();
        for r in shown {
            let first = !seen.contains(&r.action.as_str());
            seen.push(&r.action);
            let is_recording = recording.as_ref().is_some_and(|(row, _)| row == r);
            let shortcut_cell = if is_recording {
                let keys = recording.as_ref().and_then(|(_, k)| k.clone());
                let rec_focused = record_focus.is_focused(window);
                let id = row_id("settings-keymap-record", r, first);
                let sel = id.clone();
                let mut b = div()
                    .id(SharedString::from(id))
                    .debug_selector(move || sel.clone())
                    .track_focus(&record_focus)
                    .tab_stop(true)
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap(metrics::SPACE_4)
                    .h(metrics::LAYOUT_ROW_COMPACT)
                    .px(metrics::SPACE_4)
                    .rounded(metrics::RADIUS_SM)
                    .border_2()
                    .border_color(colors.border_focused)
                    .bg(colors.surface_background)
                    .text_size(metrics::FONT_SIZE_XS)
                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                    .when(rec_focused, |d| {
                        d.shadow(components::controls::ring(&colors))
                    })
                    .child(
                        div()
                            .size(metrics::SPACE_4)
                            .rounded(metrics::RADIUS_FULL)
                            .bg(status.error),
                    );
                b = match keys.as_deref() {
                    Some(k) => b.child(key_caps(
                        &shortcuts::key_caps(k, mac)
                            .into_iter()
                            .map(SharedString::from)
                            .collect::<Vec<_>>(),
                        &colors,
                        cx,
                    )),
                    None => b.child(t!(l, "settings-keymap-press-keys").into_owned()),
                };
                div().flex().items_center().gap(metrics::SPACE_3).child(b)
            } else {
                let id = row_id("settings-keymap-edit", r, first);
                let sel = id.clone();
                let label = t!(l, "settings-keymap-edit-label", action = title(r)).into_owned();
                let inner = match r.keystrokes.as_deref() {
                    Some(k) => key_caps(
                        &shortcuts::key_caps(k, mac)
                            .into_iter()
                            .map(SharedString::from)
                            .collect::<Vec<_>>(),
                        &colors,
                        cx,
                    ),
                    None => div()
                        .text_size(metrics::FONT_SIZE_XS)
                        .text_color(tokens.text_subtle)
                        .child(t!(l, "settings-keymap-unbound").into_owned()),
                };
                let hover = colors.element_hover;
                let el = div()
                    .id(SharedString::from(id))
                    .debug_selector(move || sel.clone())
                    .flex()
                    .items_center()
                    .min_h(metrics::LAYOUT_ROW_COMPACT)
                    .px(metrics::SPACE_3)
                    .rounded(metrics::RADIUS_SM)
                    .cursor_pointer()
                    .tab_stop(true)
                    .focusable()
                    .focus_visible(move |s| s.shadow(components::controls::ring(&colors)))
                    .hover(move |s| s.bg(hover))
                    .tooltip(components::reactions::tooltip_shared(label.into()))
                    .child(inner);
                let row = r.clone();
                div().child(self.wire(el, cx, move |v, window, cx| {
                    v.start_recording(row.clone(), window, cx)
                }))
            };
            let source = match r.source {
                Source::Default => div()
                    .text_color(tokens.text_subtle)
                    .child(t!(l, "settings-keymap-source-default").into_owned()),
                Source::User => div()
                    .text_color(tokens.signal_text)
                    .child(t!(l, "settings-keymap-source-user").into_owned()),
            };
            let mut actions_cell = div()
                .flex()
                .items_center()
                .gap(metrics::SPACE_3)
                .flex_none();
            if is_recording {
                let has_keys = recording.as_ref().is_some_and(|(_, k)| {
                    k.as_deref()
                        .is_some_and(|k| !shortcuts::needs_modifier(k, r.context.as_deref()))
                });
                let save = button_small(
                    "settings-keymap-save",
                    t!(l, "settings-keymap-save").into_owned(),
                    ButtonKind::Primary,
                    &colors,
                )
                .debug_selector(|| "settings-keymap-save".into());
                let cancel = button_small(
                    "settings-keymap-cancel",
                    t!(l, "settings-keymap-cancel").into_owned(),
                    ButtonKind::Secondary,
                    &colors,
                )
                .debug_selector(|| "settings-keymap-cancel".into());
                if has_keys {
                    actions_cell =
                        actions_cell.child(self.wire(save, cx, |v, _, cx| v.save_recording(cx)));
                }
                actions_cell =
                    actions_cell.child(self.wire(cancel, cx, |v, _, cx| v.cancel_recording(cx)));
            } else if r.source == Source::User {
                let id = row_id("settings-keymap-reset", r, first);
                let sel = id.clone();
                let reset = button_small(
                    SharedString::from(id),
                    t!(l, "settings-keymap-reset").into_owned(),
                    ButtonKind::Secondary,
                    &colors,
                )
                .debug_selector(move || sel.clone());
                let row = r.clone();
                actions_cell = actions_cell
                    .child(self.wire(reset, cx, move |v, _, cx| v.reset_row(row.clone(), cx)));
            }
            let line =
                div()
                    .flex()
                    .items_center()
                    .gap(metrics::SPACE_5)
                    .min_h(metrics::LAYOUT_SETTING_ROW)
                    .px(metrics::SPACE_5)
                    .text_size(metrics::FONT_SIZE_SM)
                    .when(!is_recording && shortcuts::in_conflict(&rows, r), |d| {
                        d.bg(tokens.warning_soft)
                    })
                    .when(is_recording, |d| d.bg(colors.accent_soft))
                    .child(
                        div()
                            .flex_1()
                            .min_w(metrics::LAYOUT_FIELD_XS)
                            .py(metrics::SPACE_3)
                            .child(title(r)),
                    )
                    .child(
                        div()
                            .w(metrics::LAYOUT_FIELD_SM)
                            .min_w(metrics::LAYOUT_FIELD_XS)
                            .overflow_hidden()
                            .child(shortcut_cell),
                    )
                    .child(
                        div()
                            .w(metrics::LAYOUT_FIELD_XS)
                            .min_w_0()
                            .overflow_hidden()
                            .text_size(metrics::FONT_SIZE_XS)
                            .text_color(colors.text_muted)
                            .when(r.context.is_some(), |d| {
                                d.font_family(components::rich_text::DEFAULT_CODE_FONT)
                            })
                            .child(r.context.clone().unwrap_or_else(|| {
                                t!(l, "settings-keymap-context-any").into_owned()
                            })),
                    )
                    .child(
                        div()
                            .w(metrics::LAYOUT_FIELD_XS)
                            .min_w_0()
                            .overflow_hidden()
                            .text_size(metrics::FONT_SIZE_2XS)
                            .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                            .child(source),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .w(metrics::LAYOUT_COLUMN_WIDE)
                            .min_w_0()
                            .child(actions_cell),
                    );
            let recorded = if is_recording {
                recording.as_ref().and_then(|(_, k)| k.as_deref())
            } else {
                None
            };
            let refused = recorded.filter(|k| shortcuts::needs_modifier(k, r.context.as_deref()));
            let conflict = recorded
                .filter(|_| refused.is_none())
                .and_then(|k| shortcuts::conflict(&rows, k, r));
            let banner = |selector: &'static str, text: String| {
                div()
                    .debug_selector(move || selector.into())
                    .flex()
                    .items_center()
                    .gap(metrics::SPACE_3)
                    .px(metrics::SPACE_5)
                    .py(metrics::SPACE_3)
                    .bg(tokens.warning_soft)
                    .text_size(metrics::FONT_SIZE_XS)
                    .text_color(tokens.warning_text)
                    .child(
                        Icon::new(IconName::Alert)
                            .size(metrics::ICON_SIZE_SM)
                            .color(tokens.warning_text),
                    )
                    .child(div().flex_1().min_w_0().child(text))
            };
            let note = if refused.is_some() {
                Some(banner(
                    "settings-keymap-needs-modifier",
                    t!(l, "settings-keymap-needs-modifier").into_owned(),
                ))
            } else {
                conflict.map(|c| {
                    let text = if c.context == r.context {
                        t!(l, "settings-keymap-conflict", action = title(c)).into_owned()
                    } else {
                        let ctx = c
                            .context
                            .clone()
                            .or_else(|| r.context.clone())
                            .unwrap_or_default();
                        t!(
                            l,
                            "settings-keymap-shadow",
                            action = title(c),
                            context = ctx
                        )
                        .into_owned()
                    };
                    banner("settings-keymap-conflict", text)
                })
            };
            table = table.child(
                div()
                    .flex()
                    .flex_col()
                    .border_b_1()
                    .border_color(tokens.border_subtle)
                    .child(line)
                    .children(note),
            );
        }

        div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_6)
            .pb(metrics::SPACE_9)
            .child(head)
            .child(toolbar)
            .child(table)
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_XS)
                    .text_color(tokens.text_subtle)
                    .child(t!(l, "settings-keymap-footer").into_owned()),
            )
    }
}
