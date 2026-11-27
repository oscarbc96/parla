//! About window: version, licence, the non-affiliation notice and links.

use std::path::PathBuf;

use gpui::{App, Bounds, Context, IntoElement, Render, Window, div, prelude::*, size};
use themes::ActiveTheme;

pub const PRIVACY_URL: &str = "https://oscarbc96.github.io/parla/docs/privacy/";
pub const TERMS_URL: &str = "https://oscarbc96.github.io/parla/docs/terms/";
pub const SOURCE_URL: &str = "https://github.com/oscarbc96/parla";
pub const NOTICE_URL: &str = "https://github.com/oscarbc96/parla/blob/main/NOTICE";
pub const SECURITY_URL: &str = "https://github.com/oscarbc96/parla/blob/main/SECURITY.md";

/// URL for third-party licences: the bundled NOTICE file if present, else the
/// repository NOTICE.
pub fn licences_url() -> String {
    if let Some(path) = bundled_notice_path() {
        return format!("file://{}", path.display());
    }
    NOTICE_URL.to_string()
}

fn bundled_notice_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    for candidate in [
        dir.join("NOTICE"),
        dir.join("../Resources/NOTICE"),
        dir.join("../../Resources/NOTICE"),
    ] {
        if candidate.is_file() {
            return candidate.canonicalize().ok().or(Some(candidate));
        }
    }
    None
}

#[derive(Clone)]
pub struct Link {
    pub label: String,
    pub url: String,
}

/// The labelled links shown in the About window.
pub fn about_links(l: &i18n::Localizer) -> Vec<Link> {
    vec![
        Link {
            label: t!(l, "about-privacy").into_owned(),
            url: PRIVACY_URL.to_string(),
        },
        Link {
            label: t!(l, "about-terms").into_owned(),
            url: TERMS_URL.to_string(),
        },
        Link {
            label: t!(l, "about-licences").into_owned(),
            url: licences_url(),
        },
        Link {
            label: t!(l, "about-source").into_owned(),
            url: SOURCE_URL.to_string(),
        },
        Link {
            label: t!(l, "about-security").into_owned(),
            url: SECURITY_URL.to_string(),
        },
    ]
}

/// Plain text shown in the About window: version, licence, links and, for
/// each registered provider catalog, its unofficial notice.
pub fn about_text(
    l: &i18n::Localizer,
    catalogs: &[&'static i18n::Catalog],
    version: &str,
) -> String {
    let mut lines = vec![
        t!(l, "about-version", version = version).into_owned(),
        t!(l, "about-licence").into_owned(),
    ];
    lines.extend(catalogs.iter().map(|c| {
        l.format(c.namespace(), "unofficial-notice", None)
            .into_owned()
    }));
    for link in about_links(l) {
        lines.push(format!("{}: {}", link.label, link.url));
    }
    lines.join("\n")
}

pub struct About {
    catalogs: Vec<&'static i18n::Catalog>,
    /// Version shown in the details card (the crate version, fixed in visual tests).
    version: &'static str,
    /// The version info was just copied: the button says so until the window closes.
    copied: bool,
    /// Focus of the copy button, made on first render.
    copy_focus: Option<gpui::FocusHandle>,
}

/// One label and value row of the details card; rows after the first get a
/// hairline above.
fn row(
    label: String,
    value: impl IntoElement,
    first: bool,
    colors: &themes::ThemeColors,
    subtle: gpui::Hsla,
) -> gpui::Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(themes::metrics::SPACE_5)
        .h(themes::metrics::LAYOUT_INPUT_MIN)
        .text_size(themes::metrics::FONT_SIZE_SM)
        .when(!first, |d| d.border_t_1().border_color(subtle))
        .child(div().text_color(colors.text_muted).child(label))
        .child(value)
}

impl Render for About {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        use components::controls::{ButtonKind, button};
        use themes::metrics::{
            FONT_SIZE_SM, FONT_SIZE_XS, RADIUS_LG, RADIUS_SM, SPACE_1, SPACE_2, SPACE_4, SPACE_6,
            SPACE_7, SPACE_8, SPACE_10, SPACE_11,
        };
        let colors = *cx.theme().colors();
        let tokens = themes::parla_tokens(cx.theme());
        let l = i18n::gpui::localizer(cx);
        let mark =
            components::logo::logo(themes::metrics::LAYOUT_LOGO_ABOUT, cx).into_any_element();

        let card = div()
            .flex()
            .flex_col()
            .w_full()
            .px(SPACE_6)
            .py(SPACE_2)
            .rounded(RADIUS_LG)
            .border_1()
            .border_color(colors.border)
            .bg(colors.background)
            .child(row(
                t!(l, "about-version-label").into_owned(),
                div()
                    .font_weight(themes::metrics::FONT_WEIGHT_SEMIBOLD)
                    .child(self.version),
                true,
                &colors,
                tokens.border_subtle,
            ))
            .child(row(
                t!(l, "about-licence-label").into_owned(),
                "MPL-2.0",
                false,
                &colors,
                tokens.border_subtle,
            ));

        let text = about_text(&l, &self.catalogs, self.version);
        let copy_focus = self
            .copy_focus
            .get_or_insert_with(|| cx.focus_handle())
            .clone();
        let copy_label = if self.copied {
            t!(l, "about-copied")
        } else {
            t!(l, "about-copy-version")
        }
        .into_owned();
        let copy = button(
            "about-copy",
            copy_label,
            ButtonKind::Secondary,
            true,
            &colors,
        )
        .debug_selector(|| "about-copy".into())
        .hover(move |s| s.bg(colors.element_hover))
        .track_focus(&copy_focus)
        .on_click(cx.listener({
            let text = text.clone();
            move |this, _, _, cx| this.copy(&text, cx)
        }))
        .on_key_down(cx.listener(move |this, ev: &gpui::KeyDownEvent, _, cx| {
            if matches!(ev.keystroke.key.as_str(), "enter" | "space") {
                this.copy(&text, cx);
                cx.stop_propagation();
            }
        }));

        let mut links = div()
            .flex()
            .flex_wrap()
            .justify_center()
            .gap_x(SPACE_6)
            .gap_y(SPACE_2)
            .mt(SPACE_7)
            .text_size(FONT_SIZE_SM);
        for link in about_links(&l) {
            let url = link.url.clone();
            links = links.child(
                div()
                    .id(gpui::SharedString::from(format!(
                        "about-link-{}",
                        link.label
                    )))
                    .cursor_pointer()
                    .tab_stop(true)
                    .focusable()
                    .rounded(RADIUS_SM)
                    .px(SPACE_1)
                    .text_color(colors.text_accent)
                    .hover(|s| s.underline())
                    .focus_visible(move |s| {
                        s.underline().shadow(components::controls::ring(&colors))
                    })
                    .child(link.label)
                    .on_click(move |_, _, cx| cx.open_url(&url)),
            );
        }

        // The independence line: the provider's exact notice when one is
        // registered, else the neutral sentence the sign-in screen uses.
        let mut footer = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(SPACE_2)
            .px(SPACE_10)
            .pt(SPACE_6)
            .pb(SPACE_7)
            .border_t_1()
            .border_color(tokens.border_subtle)
            .text_size(FONT_SIZE_XS)
            .text_color(tokens.text_subtle)
            .text_center()
            .child(t!(l, "about-made-by").into_owned());
        if self.catalogs.is_empty() {
            footer = footer.child(
                l.format("components", "sign-in-independent", None)
                    .into_owned(),
            );
        }
        for c in &self.catalogs {
            footer = footer.child(
                l.format(c.namespace(), "unofficial-notice", None)
                    .into_owned(),
            );
        }

        div()
            .flex()
            .flex_col()
            .size_full()
            .bg(colors.surface_background)
            .text_color(colors.text)
            .child(crate::titlebar::strip())
            .child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .items_center()
                    .px(SPACE_11)
                    .pt(SPACE_4)
                    .text_center()
                    .child(mark)
                    .child(
                        div()
                            .mt(SPACE_6)
                            .text_size(themes::metrics::FONT_SIZE_4XL)
                            .font_weight(themes::metrics::FONT_WEIGHT_BOLD)
                            .child(t!(l, "about-name").into_owned()),
                    )
                    .child(
                        div()
                            .mt(SPACE_2)
                            .text_size(themes::metrics::FONT_SIZE_MD)
                            .text_color(colors.text_muted)
                            .child(t!(l, "about-tagline").into_owned()),
                    )
                    .child(div().mt(SPACE_8).w_full().child(card))
                    .child(div().flex().mt(SPACE_6).child(copy))
                    .child(links),
            )
            .child(footer)
    }
}

impl About {
    fn copy(&mut self, text: &str, cx: &mut Context<Self>) {
        cx.write_to_clipboard(gpui::ClipboardItem::new_string(text.to_string()));
        self.copied = true;
        cx.notify();
    }

    /// Focuses the copy button (a test hook).
    pub fn focus_copy(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(h) = &self.copy_focus {
            window.focus(h, cx);
        }
    }

    /// Whether the version info was copied (a test hook).
    pub fn copied(&self) -> bool {
        self.copied
    }
}

/// The About view, for the window and for visual tests.
pub fn view(catalogs: Vec<&'static i18n::Catalog>, version: &'static str) -> About {
    About {
        catalogs,
        version,
        copied: false,
        copy_focus: None,
    }
}

/// Opens the About window; `catalogs` are the registered providers' catalogs.
pub fn open(cx: &mut App, catalogs: Vec<&'static i18n::Catalog>) {
    let bounds = Bounds::centered(
        None,
        size(
            themes::metrics::LAYOUT_WINDOW_ABOUT_WIDTH,
            themes::metrics::LAYOUT_WINDOW_ABOUT_HEIGHT,
        ),
        cx,
    );
    cx.open_window(crate::window_options::about(bounds), |_window, cx| {
        cx.new(|_cx| view(catalogs, env!("CARGO_PKG_VERSION")))
    })
    .ok();
}
