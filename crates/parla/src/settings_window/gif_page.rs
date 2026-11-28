//! The Plugins page of the Settings window: the built-in GIF search providers
//! (GIPHY, KLIPY) with their switch, their API key and the hosts they talk to,
//! plus the rating limit and how a picked GIF is sent.
//!
//! A provider runs only when it is on and has an `api_key`: the key lives in
//! the system keychain (the development credentials file in a debug build),
//! never in `settings.json` and never on screen. `plugins.<id>.enabled`,
//! `gifs.rating` and `gifs.send_as` are ordinary settings keys, written
//! through [`SettingChange`] like the other pages'.

use std::collections::BTreeMap;

use components::controls::{ButtonKind, button, compact};
use components::settings_controls::{segment, segmented, settings_row, switch, tag};
use components::text_input::TextInput;
use gpui::{Context, Entity, Focusable as _, Window, div, prelude::*};
use settings::{GifSendAs, Settings};
use themes::{ActiveTheme as _, metrics};

use super::{SettingChange, SettingsView, json_text};
use crate::plugins::{PluginManager, ProviderInfo};

/// One edit the page's settings controls make.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GifChange {
    /// `plugins.<id>.enabled`.
    Enabled(String, bool),
    /// `gifs.rating`, one of `g`, `pg`, `pg-13` or `r`.
    Rating(&'static str),
    /// `gifs.send_as`.
    SendAs(GifSendAs),
}

impl GifChange {
    pub fn key_path(&self) -> Vec<String> {
        match self {
            Self::Enabled(id, _) => vec!["plugins".into(), id.clone(), "enabled".into()],
            Self::Rating(_) => vec!["gifs".into(), "rating".into()],
            Self::SendAs(_) => vec!["gifs".into(), "send_as".into()],
        }
    }

    pub fn raw_json(&self) -> String {
        match self {
            Self::Enabled(_, on) => json_text(on),
            Self::Rating(value) => json_text(value),
            Self::SendAs(value) => json_text(value),
        }
    }
}

impl SettingChange for GifChange {
    fn path(&self) -> Vec<String> {
        self.key_path()
    }
    fn raw_json(&self) -> String {
        GifChange::raw_json(self)
    }
}

/// The key fields of the page, made on first use, and the last key failure.
pub(super) struct GifUi {
    inputs: BTreeMap<String, Entity<TextInput>>,
    /// Why the last Save or Remove failed, in the active language.
    error: Option<String>,
}

impl GifUi {
    pub(super) fn new() -> Self {
        Self {
            inputs: BTreeMap::new(),
            error: None,
        }
    }
}

/// A provider row: what the page shows and what the controls write.
struct Row {
    id: String,
    title: String,
    hosts: Vec<String>,
    key_url: Option<String>,
    enabled: bool,
    /// `Some(false)` when the provider has no key; `None` while unknown.
    key_saved: Option<bool>,
}

fn rows(s: &Settings, cx: &gpui::App) -> Vec<Row> {
    let manager = cx.try_global::<PluginManager>();
    match manager {
        Some(m) => m
            .providers_cached(&s.gifs, &s.plugins)
            .into_iter()
            .map(|p| row_from_info(p, s, m))
            .collect(),
        None => s
            .gifs
            .providers
            .iter()
            .filter_map(|id| crate::plugins::builtin::get(id))
            .filter(|b| b.bundled())
            .map(|b| Row {
                id: b.id.to_owned(),
                title: b.title(),
                hosts: b
                    .manifest()
                    .capabilities
                    .network
                    .iter()
                    .map(|h| h.as_str().to_owned())
                    .collect(),
                key_url: Some(b.key_url.to_owned()),
                enabled: s.plugins.entry(b.id).is_some_and(|e| e.enabled),
                key_saved: None,
            })
            .collect(),
    }
}

fn row_from_info(p: ProviderInfo, s: &Settings, m: &PluginManager) -> Row {
    Row {
        // The switch shows the settings key, not whether the key happens to
        // be present: an enabled provider without a key still says on.
        enabled: s.plugins.entry(&p.id).is_some_and(|e| e.enabled),
        key_saved: m.key_known(&p.id),
        id: p.id,
        title: p.title,
        hosts: p.hosts,
        key_url: p.key_url,
    }
}

impl SettingsView {
    pub(super) fn plugins_page(
        &mut self,
        window: &mut Window,
        l: &i18n::Localizer,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let colors = *cx.theme().colors();
        let tokens = themes::parla_tokens(cx.theme());
        let subtle = tokens.text_subtle;
        let s = self.settings(cx);
        let rows = rows(&s, cx);

        // Read every provider's key names once, so "Key saved" is true; the
        // answer comes back on the main thread through the secrets store.
        self.prime_gif_keys(cx);

        let mut providers = div().flex().flex_col().gap(metrics::SPACE_5);
        if rows.is_empty() {
            providers = providers.child(
                div()
                    .text_size(metrics::FONT_SIZE_SM)
                    .text_color(colors.text_muted)
                    .child(t!(l, "settings-gif-no-providers").into_owned()),
            );
        }
        for r in rows {
            let id = r.id.clone();
            let toggle_id = id.clone();
            let enabled = self.wire(
                switch(format!("settings-gif-{id}-enabled"), r.enabled, &colors),
                cx,
                move |v, _, cx| {
                    let now = v
                        .settings(cx)
                        .plugins
                        .entry(&toggle_id)
                        .is_some_and(|e| e.enabled);
                    v.apply(GifChange::Enabled(toggle_id.clone(), !now), cx)
                },
            );
            let input = self.gif_input(&id, l, cx);
            let focused = input.focus_handle(cx).is_focused(window);
            let field_id = id.clone();
            let key_field = div()
                .debug_selector(move || format!("settings-gif-{field_id}-key"))
                .flex()
                .items_center()
                .min_h(metrics::SPACE_10)
                .px(metrics::SPACE_5)
                .py(metrics::SPACE_2)
                .rounded(metrics::RADIUS_MD)
                .border_1()
                .border_color(if focused {
                    gpui::transparent_black()
                } else {
                    tokens.border_control
                })
                .bg(colors.surface_background)
                .text_size(metrics::FONT_SIZE_MD)
                .when(focused, |d| d.shadow(components::controls::ring(&colors)))
                .w(metrics::LAYOUT_FIELD_MD)
                .child(input);
            let save_id = id.clone();
            let save = self.wire(
                compact(button(
                    format!("settings-gif-{id}-save"),
                    t!(l, "settings-gif-save").into_owned(),
                    ButtonKind::Primary,
                    true,
                    &colors,
                ))
                .debug_selector({
                    let id = id.clone();
                    move || format!("settings-gif-{id}-save")
                }),
                cx,
                move |v, _, cx| v.save_gif_key(&save_id, cx),
            );
            let remove_id = id.clone();
            let has_key = r.key_saved != Some(false);
            let remove = self.wire(
                compact(button(
                    format!("settings-gif-{id}-remove"),
                    t!(l, "settings-gif-remove").into_owned(),
                    ButtonKind::Secondary,
                    has_key,
                    &colors,
                ))
                .debug_selector({
                    let id = id.clone();
                    move || format!("settings-gif-{id}-remove")
                }),
                cx,
                move |v, _, cx| v.remove_gif_key(&remove_id, cx),
            );
            let key_control = div()
                .flex()
                .items_center()
                .gap(metrics::SPACE_3)
                .child(key_field)
                .child(save)
                .child(remove);
            let key_help = if r.key_saved == Some(true) {
                Some(t!(l, "settings-gif-key-saved").into_owned().into())
            } else {
                Some(t!(l, "settings-gif-key-help").into_owned().into())
            };

            let mut card = div()
                .flex()
                .flex_col()
                .gap(metrics::SPACE_3)
                .p(metrics::SPACE_5)
                .rounded(metrics::RADIUS_MD)
                .border_1()
                .border_color(colors.border)
                .bg(colors.surface_background)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(metrics::SPACE_3)
                        .child(
                            div()
                                .flex_1()
                                .text_size(metrics::FONT_SIZE_MD)
                                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                                .child(r.title.clone()),
                        )
                        .children((r.key_saved == Some(true)).then(|| {
                            tag(
                                t!(l, "settings-gif-key-saved").into_owned(),
                                tokens.success_soft,
                                colors.text,
                            )
                        })),
                )
                .child(settings_row(
                    t!(l, "settings-gif-enabled").into_owned(),
                    None,
                    enabled,
                    &colors,
                    subtle,
                ))
                .child(settings_row(
                    t!(l, "settings-gif-key").into_owned(),
                    key_help,
                    key_control,
                    &colors,
                    subtle,
                ))
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_XS)
                        .text_color(subtle)
                        .child(
                            t!(l, "settings-gif-hosts", hosts = r.hosts.join(", ")).into_owned(),
                        ),
                );
            if let Some(url) = r.key_url.clone() {
                let link_id = format!("settings-gif-{id}-get-key");
                card = card.child(
                    div()
                        .id(link_id)
                        .cursor_pointer()
                        .tab_stop(true)
                        .focusable()
                        .rounded(metrics::RADIUS_SM)
                        .w(metrics::LAYOUT_FIELD_MD)
                        .text_size(metrics::FONT_SIZE_SM)
                        .text_color(colors.text_accent)
                        .hover(|s| s.underline())
                        .child(t!(l, "settings-gif-get-key").into_owned())
                        .on_click(move |_, _, cx| cx.open_url(&url)),
                );
            }
            providers = providers.child(card);
        }
        if let Some(error) = self.gifs.as_ref().and_then(|u| u.error.clone()) {
            providers = providers.child(
                div()
                    .debug_selector(|| "settings-gif-key-error".into())
                    .text_size(metrics::FONT_SIZE_SM)
                    .text_color(colors.text)
                    .child(error),
            );
        }

        let rating_seg = |this: &Self,
                          cx: &mut Context<Self>,
                          value: &'static str,
                          label: String,
                          selected: bool| {
            let el = segment(
                format!("settings-gif-rating-{value}"),
                label,
                selected,
                &colors,
            );
            this.wire(el, cx, move |v, _, cx| {
                v.apply(GifChange::Rating(value), cx)
            })
        };
        let rating = segmented(
            [
                rating_seg(self, cx, "g", "G".into(), s.gifs.rating == "g"),
                rating_seg(self, cx, "pg", "PG".into(), s.gifs.rating == "pg"),
                rating_seg(self, cx, "pg-13", "PG-13".into(), s.gifs.rating == "pg-13"),
                rating_seg(self, cx, "r", "R".into(), s.gifs.rating == "r"),
            ],
            &colors,
        );
        let send_seg = |this: &Self,
                        cx: &mut Context<Self>,
                        value: GifSendAs,
                        id: &'static str,
                        label: String| {
            let el = segment(id, label, s.gifs.send_as == value, &colors);
            this.wire(el, cx, move |v, _, cx| {
                v.apply(GifChange::SendAs(value), cx)
            })
        };
        let send = segmented(
            [
                send_seg(
                    self,
                    cx,
                    GifSendAs::Link,
                    "settings-gif-send-link",
                    t!(l, "settings-gif-send-link").into_owned(),
                ),
                send_seg(
                    self,
                    cx,
                    GifSendAs::Upload,
                    "settings-gif-send-upload",
                    t!(l, "settings-gif-send-upload").into_owned(),
                ),
            ],
            &colors,
        );
        let sending = div()
            .flex()
            .flex_col()
            .gap(metrics::SPACE_3)
            .child(settings_row(
                t!(l, "settings-gif-rating").into_owned(),
                Some(t!(l, "settings-gif-rating-help").into_owned().into()),
                rating,
                &colors,
                subtle,
            ))
            .child(settings_row(
                t!(l, "settings-gif-send-as").into_owned(),
                Some(t!(l, "settings-gif-send-help").into_owned().into()),
                send,
                &colors,
                subtle,
            ));

        div()
            .flex()
            .flex_col()
            .child(Self::page_header(
                t!(l, "settings-gif-title").into_owned(),
                Some(t!(l, "settings-gif-subtitle").into_owned()),
                &colors,
            ))
            .child(div().h(metrics::SPACE_4))
            .child(Self::section(
                l,
                "settings-gif-providers-title",
                "settings-gif-providers-desc",
                providers,
                true,
                cx,
            ))
            .child(Self::section(
                l,
                "settings-gif-send-title",
                "settings-gif-send-desc",
                sending,
                false,
                cx,
            ))
            .pb(metrics::SPACE_8)
    }

    /// The key field of provider `id`, made once.
    fn gif_input(
        &mut self,
        id: &str,
        l: &i18n::Localizer,
        cx: &mut Context<Self>,
    ) -> Entity<TextInput> {
        let ui = self.gifs.get_or_insert_with(GifUi::new);
        if let Some(input) = ui.inputs.get(id) {
            return input.clone();
        }
        let input = cx.new(TextInput::new);
        let placeholder = t!(l, "settings-gif-key-placeholder").into_owned();
        input.update(cx, |t, _| {
            t.set_masked(true);
            t.set_single_line(true);
            t.set_placeholder(placeholder);
        });
        ui.inputs.insert(id.to_owned(), input.clone());
        input
    }

    fn save_gif_key(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(input) = self.gifs.as_ref().and_then(|u| u.inputs.get(id).cloned()) else {
            return;
        };
        let key = input.read(cx).text().trim().to_owned();
        if key.is_empty() {
            return;
        }
        let Some(manager) = cx.try_global::<PluginManager>().cloned() else {
            let l = i18n::gpui::localizer(cx);
            let ui = self.gifs.get_or_insert_with(GifUi::new);
            ui.error = Some(t!(l, "settings-gif-unavailable").into_owned());
            cx.notify();
            return;
        };
        let id = id.to_owned();
        let handle = crate::runtime::handle(cx);
        cx.spawn(async move |this, cx| {
            let future = {
                let manager = manager.clone();
                let id = id.clone();
                async move { manager.set_up(&id, &key).await }
            };
            let result = match handle {
                Some(handle) => handle.spawn(future).await.unwrap_or_else(|e| {
                    Err(components::gif_picker::GifError::Failed(e.to_string()))
                }),
                None => future.await,
            };
            this.update(cx, |v, cx| {
                let ui = v.gifs.get_or_insert_with(GifUi::new);
                match result {
                    Ok(()) => {
                        ui.error = None;
                        input.update(cx, |t, cx| t.set_text("", cx));
                    }
                    Err(_) => {
                        let l = i18n::gpui::localizer(cx);
                        ui.error = Some(t!(l, "settings-gif-key-failed").into_owned());
                    }
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    fn remove_gif_key(&mut self, id: &str, cx: &mut Context<Self>) {
        let Some(manager) = cx.try_global::<PluginManager>().cloned() else {
            let l = i18n::gpui::localizer(cx);
            let ui = self.gifs.get_or_insert_with(GifUi::new);
            ui.error = Some(t!(l, "settings-gif-unavailable").into_owned());
            cx.notify();
            return;
        };
        let id = id.to_owned();
        let handle = crate::runtime::handle(cx);
        cx.spawn(async move |this, cx| {
            let future = {
                let manager = manager.clone();
                let id = id.clone();
                async move { manager.remove_key(&id).await }
            };
            let result = match handle {
                Some(handle) => handle.spawn(future).await.unwrap_or_else(|e| {
                    Err(components::gif_picker::GifError::Failed(e.to_string()))
                }),
                None => future.await,
            };
            this.update(cx, |v, cx| {
                let ui = v.gifs.get_or_insert_with(GifUi::new);
                if result.is_err() {
                    let l = i18n::gpui::localizer(cx);
                    ui.error = Some(t!(l, "settings-gif-key-failed").into_owned());
                } else {
                    ui.error = None;
                }
                cx.notify();
            })
            .ok();
        })
        .detach();
    }

    /// Reads every bundled provider's key names once, then repaints. The flag
    /// stays set: a Save or Remove updates the manager's cache itself and
    /// notifies, so priming again on every render would only loop.
    fn prime_gif_keys(&mut self, cx: &mut Context<Self>) {
        if self.gifs_primed {
            return;
        }
        let Some(manager) = cx.try_global::<PluginManager>().cloned() else {
            return;
        };
        self.gifs_primed = true;
        let handle = crate::runtime::handle(cx);
        cx.spawn(async move |this, cx| {
            let future = async move { manager.prime_keys().await };
            match handle {
                Some(handle) => {
                    let _ = handle.spawn(future).await;
                }
                None => future.await,
            }
            this.update(cx, |_, cx| cx.notify()).ok();
        })
        .detach();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changes_name_their_settings_keys() {
        let e = GifChange::Enabled("giphy".into(), true);
        assert_eq!(e.key_path(), vec!["plugins", "giphy", "enabled"]);
        assert_eq!(e.raw_json(), "true");
        let r = GifChange::Rating("pg-13");
        assert_eq!(r.key_path(), vec!["gifs", "rating"]);
        assert_eq!(r.raw_json(), "\"pg-13\"");
        let s = GifChange::SendAs(GifSendAs::Upload);
        assert_eq!(s.key_path(), vec!["gifs", "send_as"]);
        assert_eq!(s.raw_json(), "\"upload\"");
    }
}
