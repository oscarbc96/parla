//! Key symbols as icons. The real window draws text with the user's UI font and
//! its system fallbacks, and those lack the Mac modifier symbols (shift, option,
//! control), the return and arrow keys and the check mark, so they show as
//! missing-glyph boxes; the visual harness bundles Inter and would not notice.
//! Every place that shows such a symbol draws it from the UI icon set instead,
//! which does not depend on a font. The data stays plain text ("⌘⇧K");
//! [`glyph_row`] turns the symbols into icons when it draws, and gives the row an
//! accessible label that speaks the key names.
//!
//! The symbols live here as constants (and in `parla::shortcuts`, which maps
//! keymap keys to caps); no other source file may write them (see
//! `tests/glyph_guard.rs`).

use gpui::{
    Div, ElementId, Hsla, InteractiveElement as _, IntoElement, ParentElement as _, Pixels, Role,
    SharedString, Stateful, StatefulInteractiveElement as _, Styled as _, div,
};
use std::hash::{Hash as _, Hasher as _};

use crate::icon::{Icon, IconName};

pub const COMMAND: &str = "\u{2318}";
pub const SHIFT: &str = "\u{21E7}";
pub const OPTION: &str = "\u{2325}";
pub const CONTROL: &str = "\u{2303}";
pub const RETURN: &str = "\u{23CE}";
pub const SHIFT_RETURN: &str = "\u{21E7}\u{23CE}";
pub const BACKSPACE: &str = "\u{232B}";
pub const TAB: &str = "\u{21E5}";
pub const UP: &str = "\u{2191}";
pub const DOWN: &str = "\u{2193}";
pub const LEFT: &str = "\u{2190}";
pub const RIGHT: &str = "\u{2192}";
pub const UP_DOWN: &str = "\u{2191}\u{2193}";
pub const CHECK: &str = "\u{2713}";

/// The icon that stands for a key symbol, if the symbol is one we draw as an icon.
pub fn glyph_icon(c: char) -> Option<IconName> {
    Some(match c {
        '\u{2318}' => IconName::Command,
        '\u{21E7}' => IconName::Shift,
        '\u{2325}' => IconName::OptionKey,
        '\u{2303}' => IconName::Control,
        '\u{23CE}' => IconName::Return,
        '\u{232B}' => IconName::Backspace,
        '\u{21E5}' => IconName::KeyTab,
        '\u{2191}' => IconName::ArrowUp,
        '\u{2193}' => IconName::ArrowDown,
        '\u{2190}' => IconName::ArrowLeft,
        '\u{2192}' => IconName::ArrowRight,
        '\u{2713}' | '\u{2714}' => IconName::Check,
        _ => return None,
    })
}

/// The catalog id of the spoken name of an icon drawn for a symbol.
fn spoken_id(icon: IconName) -> Option<&'static str> {
    Some(match icon {
        IconName::Command => "keycap-command",
        IconName::Shift => "keycap-shift",
        IconName::OptionKey => "keycap-option",
        IconName::Control => "keycap-control",
        IconName::Return => "keycap-return",
        IconName::Backspace => "keycap-backspace",
        IconName::KeyTab => "keycap-tab",
        IconName::ArrowUp => "keycap-up",
        IconName::ArrowDown => "keycap-down",
        IconName::ArrowLeft => "keycap-left",
        IconName::ArrowRight => "keycap-right",
        IconName::Check => "keycap-check",
        _ => return None,
    })
}

/// Whether `text` holds a symbol drawn as an icon.
pub fn has_glyphs(text: &str) -> bool {
    text.chars().any(|c| glyph_icon(c).is_some())
}

/// One piece of drawn text: words as text, symbols as icons.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Piece {
    Text(String),
    Icon(IconName),
}

/// Splits one whitespace-free token into text runs and icons. No text run holds
/// a symbol that has an icon.
pub fn pieces(token: &str) -> Vec<Piece> {
    let mut out = Vec::new();
    let mut run = String::new();
    for c in token.chars() {
        if let Some(icon) = glyph_icon(c) {
            if !run.is_empty() {
                out.push(Piece::Text(std::mem::take(&mut run)));
            }
            out.push(Piece::Icon(icon));
        } else {
            run.push(c);
        }
    }
    if !run.is_empty() {
        out.push(Piece::Text(run));
    }
    out
}

/// `text` with every symbol replaced by its spoken name ("⇧⏎ new line" becomes
/// "Shift Return new line"), in the catalog's language.
pub fn spoken(text: &str, cx: &gpui::App) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    for c in text.chars() {
        match glyph_icon(c).and_then(spoken_id) {
            Some(id) => {
                if !out.is_empty() && !out.ends_with(' ') {
                    out.push(' ');
                }
                out.push_str(&spoken_name(id, cx));
                out.push(' ');
            }
            None => out.push(c),
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn spoken_name(id: &str, cx: &gpui::App) -> String {
    // The catalog takes literal ids, so each is spelled out here.
    match id {
        "keycap-command" => t!(cx, "keycap-command").into_owned(),
        "keycap-shift" => t!(cx, "keycap-shift").into_owned(),
        "keycap-option" => t!(cx, "keycap-option").into_owned(),
        "keycap-control" => t!(cx, "keycap-control").into_owned(),
        "keycap-return" => t!(cx, "keycap-return").into_owned(),
        "keycap-backspace" => t!(cx, "keycap-backspace").into_owned(),
        "keycap-tab" => t!(cx, "keycap-tab").into_owned(),
        "keycap-up" => t!(cx, "keycap-up").into_owned(),
        "keycap-down" => t!(cx, "keycap-down").into_owned(),
        "keycap-left" => t!(cx, "keycap-left").into_owned(),
        "keycap-right" => t!(cx, "keycap-right").into_owned(),
        _ => t!(cx, "keycap-check").into_owned(),
    }
}

/// Text split into words, each into text runs and icons, with its spoken label.
/// Built once for a string that stays (the composer hint) and rebuilt only when
/// the string changes; a string without symbols holds just the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glyphs {
    text: SharedString,
    /// Empty when `text` has no symbols.
    words: Vec<Vec<Piece>>,
    label: Option<SharedString>,
}

impl Glyphs {
    pub fn new(text: impl Into<SharedString>, cx: &gpui::App) -> Self {
        let text = text.into();
        if !has_glyphs(&text) {
            return Self {
                text,
                words: Vec::new(),
                label: None,
            };
        }
        let words = text.split_whitespace().map(pieces).collect();
        let label = Some(SharedString::from(spoken(&text, cx)));
        Self { text, words, label }
    }

    /// The text as given (symbols included).
    pub fn text(&self) -> &SharedString {
        &self.text
    }

    /// The spoken label, when the text has symbols.
    pub fn label(&self) -> Option<&SharedString> {
        self.label.as_ref()
    }

    /// Draws the text: words as text, symbols as icons of `size` in `color`,
    /// a space apart. Text without symbols is drawn as it is. Call
    /// `.flex_wrap()` on the result where the text may wrap.
    pub fn row(&self, size: Pixels, color: Hsla) -> Stateful<Div> {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.text.hash(&mut hasher);
        let id = ElementId::NamedInteger("keycap".into(), hasher.finish());
        let Some(label) = &self.label else {
            return div().id(id).child(self.text.clone());
        };
        let mut row = div()
            .id(id)
            .role(Role::Label)
            .aria_label(label.clone())
            .flex()
            .items_center();
        for (i, word) in self.words.iter().enumerate() {
            if i > 0 {
                // A real space between words, as wide as the font's.
                row = row.child(div().child("\u{a0}"));
            }
            row = row.child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .children(word.iter().map(|p| {
                        match p {
                            Piece::Text(t) => div()
                                .child(SharedString::from(t.clone()))
                                .into_any_element(),
                            Piece::Icon(i) => {
                                Icon::new(*i).size(size).color(color).into_any_element()
                            }
                        }
                    })),
            );
        }
        row
    }
}

/// `text` with its key symbols drawn as icons ([`Glyphs::row`]), built on the
/// spot. A text without symbols costs one clone.
pub fn glyph_row(text: &SharedString, size: Pixels, color: Hsla, cx: &gpui::App) -> Stateful<Div> {
    if !has_glyphs(text) {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        text.hash(&mut hasher);
        return div()
            .id(ElementId::NamedInteger("keycap".into(), hasher.finish()))
            .child(text.clone());
    }
    Glyphs::new(text.clone(), cx).row(size, color)
}
