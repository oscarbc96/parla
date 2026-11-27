//! Code snippets: the offer made when a long text is pasted into the composer,
//! the languages a snippet can have, and the snippet card shown in the timeline.
//! Everything here is pure except [`render_snippet`], which only draws.

use gpui::{AnyElement, IntoElement, Stateful, div, prelude::*};
use i18n::Localizer;
use themes::{Theme, metrics, parla_tokens};

use crate::icon::{Icon, IconName};
use crate::snippet_highlight::{Class, highlight};

/// A paste of more than this many lines offers to become a snippet (the board's
/// "paste over 20 lines in the composer").
pub const OFFER_MIN_LINES: usize = 20;

/// A paste of more than this many characters offers to become a snippet even when it
/// has fewer lines (one very long minified line is as unreadable in a message).
pub const OFFER_MIN_CHARS: usize = 2_000;

/// Lines of a snippet the card shows before Expand.
pub const COLLAPSED_LINES: usize = 8;

/// Lines of a snippet the card ever holds. A longer text is cut here and the card says so;
/// the whole file stays one Download away. Bounds the memory one card can take.
pub const MAX_LINES: usize = 400;

/// Bytes of a snippet the card ever holds (cut on a line boundary), the same bound in bytes.
pub const MAX_BYTES: usize = 64 * 1024;

/// A snippet language: the id the provider receives, the label shown, the file extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Language {
    pub id: &'static str,
    pub label: &'static str,
    pub ext: &'static str,
}

impl Language {
    /// The label in the active language: plain text is translated, the others are names.
    pub fn display(&self, l: &Localizer) -> String {
        if self.id == "text" {
            t!(l, "snippet-language-text").into_owned()
        } else {
            self.label.to_string()
        }
    }
}

/// The languages offered in the picker. The first is plain text (no syntax).
pub const LANGUAGES: &[Language] = &[
    Language {
        id: "text",
        label: "Plain text",
        ext: "txt",
    },
    Language {
        id: "rust",
        label: "Rust",
        ext: "rs",
    },
    Language {
        id: "python",
        label: "Python",
        ext: "py",
    },
    Language {
        id: "javascript",
        label: "JavaScript",
        ext: "js",
    },
    Language {
        id: "typescript",
        label: "TypeScript",
        ext: "ts",
    },
    Language {
        id: "go",
        label: "Go",
        ext: "go",
    },
    Language {
        id: "java",
        label: "Java",
        ext: "java",
    },
    Language {
        id: "c",
        label: "C",
        ext: "c",
    },
    Language {
        id: "cpp",
        label: "C++",
        ext: "cpp",
    },
    Language {
        id: "csharp",
        label: "C#",
        ext: "cs",
    },
    Language {
        id: "ruby",
        label: "Ruby",
        ext: "rb",
    },
    Language {
        id: "shell",
        label: "Shell",
        ext: "sh",
    },
    Language {
        id: "sql",
        label: "SQL",
        ext: "sql",
    },
    Language {
        id: "json",
        label: "JSON",
        ext: "json",
    },
    Language {
        id: "yaml",
        label: "YAML",
        ext: "yaml",
    },
    Language {
        id: "toml",
        label: "TOML",
        ext: "toml",
    },
    Language {
        id: "html",
        label: "HTML",
        ext: "html",
    },
    Language {
        id: "css",
        label: "CSS",
        ext: "css",
    },
    Language {
        id: "markdown",
        label: "Markdown",
        ext: "md",
    },
    Language {
        id: "diff",
        label: "Diff",
        ext: "patch",
    },
];

/// Plain text, the language of anything not recognised.
pub fn plain() -> &'static Language {
    &LANGUAGES[0]
}

/// The language with provider id `id`.
pub fn language_by_id(id: &str) -> Option<&'static Language> {
    LANGUAGES.iter().find(|l| l.id.eq_ignore_ascii_case(id))
}

/// The language of a file name, by its extension.
pub fn language_for_name(name: &str) -> Option<&'static Language> {
    let ext = std::path::Path::new(name).extension()?.to_str()?;
    let ext = ext.to_ascii_lowercase();
    // Common aliases next to the canonical extension of each language.
    let ext = match ext.as_str() {
        "jsx" | "mjs" | "cjs" => "js",
        "tsx" => "ts",
        "py3" | "pyw" => "py",
        "cc" | "cxx" | "hpp" | "h" => "cpp",
        "bash" | "zsh" => "sh",
        "yml" => "yaml",
        "htm" => "html",
        "diff" => "patch",
        other => other,
    };
    LANGUAGES.iter().find(|l| l.ext == ext)
}

/// Whether pasting `text` offers a snippet. Stops reading as soon as a threshold is passed.
pub fn offers_snippet(text: &str) -> bool {
    text.chars().nth(OFFER_MIN_CHARS).is_some()
        || text.lines().take(OFFER_MIN_LINES + 1).count() > OFFER_MIN_LINES
}

/// The largest text offered as a snippet. A paste above it is refused with a notice: a
/// snippet is read in the timeline and held in memory, and a provider caps one anyway.
pub const MAX_SNIPPET_BYTES: usize = 1024 * 1024;

/// Bytes and lines of a paste read to guess its language (the rest is not looked at).
const DETECT_BYTES: usize = 64 * 1024;
const DETECT_LINES: usize = 200;

/// What a pasted text is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Paste {
    /// Short: goes into the draft.
    Short,
    /// Long enough for a snippet but over [`MAX_SNIPPET_BYTES`].
    TooLarge,
    /// Long: offered as a snippet of `lines` lines, probably in `language`.
    Snippet {
        lines: usize,
        language: &'static Language,
    },
}

/// Classifies a pasted text in one pass over it: a newline count, a bounded look at the
/// start for the language, and early exits for the thresholds.
pub fn classify_paste(text: &str) -> Paste {
    if !offers_snippet(text) {
        return Paste::Short;
    }
    if text.len() > MAX_SNIPPET_BYTES {
        return Paste::TooLarge;
    }
    let newlines = text.bytes().filter(|&b| b == b'\n').count();
    let lines = newlines + usize::from(!text.ends_with('\n') && !text.is_empty());
    Paste::Snippet {
        lines: lines.max(1),
        language: detect_language(text),
    }
}

/// The start of `text` that language detection reads: at most [`DETECT_LINES`] lines and
/// [`DETECT_BYTES`] bytes.
fn head(text: &str) -> &str {
    let mut end = 0;
    for line in text.split_inclusive('\n').take(DETECT_LINES) {
        if end + line.len() > DETECT_BYTES {
            end = DETECT_BYTES.min(text.len());
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            return &text[..end];
        }
        end += line.len();
    }
    &text[..end]
}

/// Guesses the language of `text` from its first lines. Simple cases only (a shebang,
/// a JSON document, a diff, and the keywords of a few common languages); anything else
/// is plain text. The user can always change it.
pub fn detect_language(text: &str) -> &'static Language {
    let t = head(text).trim_start();
    let first = t.lines().next().unwrap_or("");
    let by = |id: &str| language_by_id(id).unwrap_or_else(plain);
    if let Some(rest) = first.strip_prefix("#!") {
        return if rest.contains("python") {
            by("python")
        } else if rest.contains("ruby") {
            by("ruby")
        } else if rest.contains("node") {
            by("javascript")
        } else {
            by("shell")
        };
    }
    if t.starts_with("diff --git") || (t.starts_with("--- ") && t.contains("\n+++ ")) {
        return by("diff");
    }
    // Looks like a JSON document: braces or brackets around, and a key somewhere.
    let end = t.trim_end();
    if ((t.starts_with('{') && end.ends_with('}')) || (t.starts_with('[') && end.ends_with(']')))
        && t.contains("\":")
    {
        return by("json");
    }
    let starts = |p: &[&str]| {
        t.lines()
            .map(str::trim_start)
            .any(|l| p.iter().any(|p| l.starts_with(p)))
    };
    if starts(&[
        "fn ",
        "pub fn ",
        "use std::",
        "impl ",
        "let mut ",
        "#[derive",
        "pub struct ",
    ]) {
        by("rust")
    } else if starts(&["def ", "import ", "from ", "elif ", "print("])
        && !starts(&["function ", "const ", "{", "};"])
    {
        by("python")
    } else if starts(&["func ", "package ", "go func"]) {
        by("go")
    } else if starts(&[
        "function ",
        "const ",
        "export ",
        "let ",
        "=> {",
        "console.log",
    ]) {
        by("javascript")
    } else if starts(&["public class ", "public static void ", "import java."]) {
        by("java")
    } else if starts(&["#include "]) {
        by("c")
    } else if starts(&[
        "select ",
        "SELECT ",
        "insert into ",
        "INSERT INTO ",
        "create table ",
    ]) {
        by("sql")
    } else if t.starts_with('<') && (t.contains("</") || t.contains("/>")) {
        by("html")
    } else {
        plain()
    }
}

/// The file name a snippet is uploaded under: the title, with the language's
/// extension when the title has none; `snippet.<ext>` when the title is empty.
pub fn file_name(title: &str, lang: &Language) -> String {
    // Never let a typed title make a path.
    let title: String = title
        .trim()
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | '\0') {
                '_'
            } else {
                c
            }
        })
        .collect();
    if title.is_empty() {
        return format!("snippet.{}", lang.ext);
    }
    // `name.ext`: a short alphanumeric tail after a dot that follows something.
    let has_ext = title.rsplit_once('.').is_some_and(|(stem, ext)| {
        !stem.is_empty()
            && (1..=8).contains(&ext.len())
            && ext.chars().all(|c| c.is_ascii_alphanumeric())
    });
    if has_ext {
        title
    } else {
        format!("{title}.{}", lang.ext)
    }
}

/// The text of a snippet cut to [`MAX_LINES`] and [`MAX_BYTES`] on a line boundary.
/// The flag says whether anything was cut.
pub fn bound(text: &str) -> (String, bool) {
    let mut out = String::new();
    for (lines, line) in text.split_inclusive('\n').enumerate() {
        if lines == MAX_LINES || out.len() + line.len() > MAX_BYTES {
            return (out, true);
        }
        out.push_str(line);
    }
    (out, false)
}

/// The languages whose name or id contains `query` (any case); all of them when empty.
pub fn filter_languages(query: &str) -> Vec<&'static Language> {
    let q = query.trim().to_lowercase();
    LANGUAGES
        .iter()
        .filter(|l| {
            q.is_empty() || l.label.to_lowercase().contains(&q) || l.id.to_lowercase().contains(&q)
        })
        .collect()
}

/// The highlighted spans of every line of a snippet.
type Spans = std::sync::Arc<[Vec<(std::ops::Range<usize>, Class)>]>;

/// What the timeline draws for a snippet file. Pure: built by [`crate::files::card`] and
/// cached per file by the timeline, so a repaint builds nothing; cloning shares the lines.
#[derive(Clone, Debug, PartialEq)]
pub struct SnippetCard {
    pub name: String,
    pub language: Option<&'static Language>,
    /// Every line held (up to [`MAX_LINES`]), without the line ends.
    pub lines: std::sync::Arc<[String]>,
    /// Highlight spans, one list per line of `lines`.
    pub spans: Spans,
    /// Lines in the whole file, when the provider says.
    pub total_lines: Option<usize>,
    /// The card holds only part of the file: the preview is shorter than the file, or the
    /// text was cut at [`MAX_LINES`] or [`MAX_BYTES`]. Copy then copies the preview.
    pub partial: bool,
    pub expanded: bool,
}

impl SnippetCard {
    /// A card of `preview`, the text the provider gave; `total_lines` and `size` describe the
    /// whole file (`size` in bytes) and tell whether the preview is all of it.
    pub fn new(
        name: String,
        preview: &str,
        total_lines: Option<u32>,
        size: u64,
        expanded: bool,
    ) -> Self {
        let (text, cut) = bound(preview);
        let lines: Vec<String> = text.lines().map(str::to_string).collect();
        let language = language_for_name(&name);
        let spans: Vec<_> = lines
            .iter()
            .map(|l| language.map_or_else(Vec::new, |lang| highlight(lang, l)))
            .collect();
        let total_lines = total_lines.map(|t| t as usize);
        let partial = cut || total_lines.map_or(size > preview.len() as u64, |t| t > lines.len());
        Self {
            language,
            lines: lines.into(),
            spans: spans.into(),
            name,
            total_lines,
            partial,
            expanded,
        }
    }

    /// The lines drawn: the first [`COLLAPSED_LINES`], or all of them when expanded.
    pub fn shown(&self) -> &[String] {
        if self.expanded {
            &self.lines
        } else {
            &self.lines[..self.lines.len().min(COLLAPSED_LINES)]
        }
    }

    /// Lines Expand would reveal.
    pub fn hidden(&self) -> usize {
        self.lines.len().saturating_sub(COLLAPSED_LINES)
    }

    /// The text Copy puts on the clipboard: every line held.
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Lines the header states: the whole file's when known, else those held.
    pub fn line_count(&self) -> usize {
        self.total_lines.unwrap_or(self.lines.len())
    }
}

/// The colour of a highlight class.
pub fn class_color(t: &themes::ParlaTokens, c: Class) -> gpui::Hsla {
    match c {
        Class::Keyword => t.code_keyword,
        Class::String => t.code_string,
        Class::Function => t.code_function,
        Class::Comment => t.code_comment,
        Class::Number => t.code_number,
        Class::Type => t.code_type,
    }
}

/// One code line: its text with the highlight spans coloured.
fn code_line(
    text: &str,
    spans: &[(std::ops::Range<usize>, Class)],
    tokens: &themes::ParlaTokens,
) -> gpui::StyledText {
    // An empty line keeps its height.
    let shown = if text.is_empty() { " " } else { text };
    gpui::StyledText::new(shown.to_string()).with_highlights(spans.iter().map(|(r, c)| {
        (
            r.clone(),
            gpui::HighlightStyle {
                color: Some(class_color(tokens, *c)),
                font_style: (*c == Class::Comment).then_some(gpui::FontStyle::Italic),
                ..Default::default()
            },
        )
    }))
}

/// A preview of the first lines of `text`, highlighted as `language` (the composer's offer).
pub fn render_preview(
    text: &str,
    language: &Language,
    theme: &Theme,
    code_font: gpui::SharedString,
) -> gpui::Div {
    let colors = *theme.colors();
    let tokens = parla_tokens(theme);
    div()
        .overflow_hidden()
        .max_h(metrics::LAYOUT_PASTE_PREVIEW_H)
        .px(metrics::SPACE_5)
        .py(metrics::SPACE_3)
        .rounded(metrics::RADIUS_MD)
        .border_1()
        .border_color(tokens.code_border)
        .bg(colors.surface_background)
        .font_family(code_font)
        .text_size(metrics::FONT_SIZE_XS)
        .text_color(colors.text)
        .children(text.lines().take(COLLAPSED_LINES).map(|line| {
            div()
                .whitespace_nowrap()
                .child(code_line(line, &highlight(language, line), &tokens))
        }))
}

/// A snippet card's callback.
pub type SnippetAction = Box<dyn Fn(&mut gpui::Window, &mut gpui::App) + 'static>;

/// What a snippet card's buttons do: Expand or Collapse, Download, Open in viewer.
pub struct SnippetActions {
    pub toggle: SnippetAction,
    pub download: SnippetAction,
    pub open: SnippetAction,
}

/// Draws a snippet card: header (name, language, line count, Copy, Download, Open in
/// viewer), the numbered
/// lines, and Expand or Collapse when there are more than [`COLLAPSED_LINES`].
pub fn render_snippet(
    c: &SnippetCard,
    theme: &Theme,
    l: &Localizer,
    id: impl Into<gpui::ElementId>,
    code_font: gpui::SharedString,
    actions: SnippetActions,
) -> Stateful<gpui::Div> {
    let SnippetActions {
        toggle: on_toggle,
        download: on_download,
        open: on_open,
    } = actions;
    let colors = *theme.colors();
    let tokens = parla_tokens(theme);
    let id: gpui::ElementId = id.into();
    // The click handler joins the lines; nothing is built until the user copies.
    let lines = c.lines.clone();
    let copy_label = if c.partial {
        t!(l, "snippet-copy-preview").into_owned()
    } else {
        t!(l, "snippet-copy").into_owned()
    };
    let head = div()
        .flex()
        .items_center()
        .gap(metrics::SPACE_4)
        .h(metrics::LAYOUT_CODE_HEAD)
        .pl(metrics::SPACE_5)
        .pr(metrics::SPACE_3)
        .border_b_1()
        .border_color(tokens.code_border)
        .bg(tokens.bg_sunken)
        .text_size(metrics::FONT_SIZE_XS)
        .child(
            Icon::new(IconName::Code)
                .size(metrics::ICON_SIZE_SM)
                .color(colors.text_muted),
        )
        .child(
            div()
                .min_w_0()
                .truncate()
                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                .text_color(colors.text)
                .child(c.name.clone()),
        )
        .children(c.language.map(|lang| {
            div()
                .flex_none()
                .px(metrics::SPACE_3)
                .rounded(metrics::RADIUS_SM)
                .bg(colors.accent_soft)
                .text_color(tokens.accent_soft_text)
                .text_size(metrics::FONT_SIZE_2XS)
                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                .child(lang.display(l))
        }))
        .child(
            div()
                .flex_none()
                .text_color(tokens.text_subtle)
                .child(t!(l, "snippet-lines", count = c.line_count() as i64).into_owned()),
        )
        .child(div().flex_1())
        .child(
            crate::controls::icon_button(
                "snippet-copy",
                IconName::Copy,
                copy_label,
                &colors,
                move |_, cx| {
                    cx.write_to_clipboard(gpui::ClipboardItem::new_string(lines.join("\n")))
                },
            )
            .debug_selector(|| "snippet-copy".into()),
        )
        .child(
            crate::controls::icon_button(
                "snippet-download",
                IconName::Download,
                t!(l, "snippet-download").into_owned(),
                &colors,
                on_download,
            )
            .debug_selector(|| "snippet-download".into()),
        )
        .child(
            crate::controls::icon_button(
                "snippet-open",
                IconName::Maximize,
                t!(l, "snippet-open").into_owned(),
                &colors,
                on_open,
            )
            .debug_selector(|| "snippet-open".into()),
        );
    let gutter = metrics::LAYOUT_CODE_GUTTER;
    let rows = c.shown().iter().enumerate().map(|(i, line)| {
        let n = i + 1;
        div()
            .debug_selector(move || format!("snippet-line-{n}"))
            .flex()
            .child(
                div()
                    .flex_none()
                    .min_w(gutter)
                    .pr(metrics::SPACE_5)
                    .text_right()
                    .text_color(tokens.text_subtle)
                    .child(n.to_string()),
            )
            .child(
                div()
                    .flex_none()
                    .whitespace_nowrap()
                    .pr(metrics::SPACE_5)
                    .child(code_line(line, &c.spans[i], &tokens)),
            )
    });
    let fade = (!c.expanded && c.hidden() > 0).then(|| {
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom_0()
            .h(metrics::LAYOUT_CODE_FADE)
            .bg(gpui::linear_gradient(
                180.0,
                gpui::linear_color_stop(colors.surface_background.opacity(0.0), 0.0),
                gpui::linear_color_stop(colors.surface_background, 1.0),
            ))
    });
    // Long lines scroll sideways inside the card; the page never does.
    let body = div()
        .relative()
        .child(
            div()
                .id("snippet-body")
                .debug_selector(|| "snippet-body".into())
                .overflow_x_scroll()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_none()
                        .min_w_full()
                        .py(metrics::SPACE_4)
                        .font_family(code_font)
                        .text_size(metrics::FONT_SIZE_XS)
                        .text_color(colors.text)
                        .children(rows),
                ),
        )
        .children(fade);
    let toggle: Option<AnyElement> = (c.hidden() > 0).then(|| {
        let (icon, label) = if c.expanded {
            (IconName::ChevUp, t!(l, "snippet-collapse").into_owned())
        } else {
            (
                IconName::ChevDown,
                t!(l, "snippet-expand", count = c.hidden() as i64).into_owned(),
            )
        };
        let hover = colors.element_hover;
        crate::controls::on_activate_app(
            div()
                .id("snippet-toggle")
                .debug_selector(|| "snippet-toggle".into())
                .flex()
                .items_center()
                .justify_center()
                .gap(metrics::SPACE_3)
                .h(metrics::LAYOUT_ROW_COZY)
                .border_t_1()
                .border_color(tokens.code_border)
                .text_color(colors.text_accent)
                .text_size(metrics::FONT_SIZE_XS)
                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                .cursor_pointer()
                .tab_stop(true)
                .focusable()
                .focus_visible(crate::controls::focus_style(&colors))
                .hover(move |d| d.bg(hover))
                .child(
                    Icon::new(icon)
                        .size(metrics::ICON_SIZE_SM)
                        .color(colors.text_accent),
                )
                .child(label),
            on_toggle,
        )
        .into_any_element()
    });
    let note = c.partial.then(|| {
        let text = match c.total_lines {
            Some(total) => t!(
                l,
                "snippet-partial",
                shown = c.lines.len() as i64,
                total = total as i64
            )
            .into_owned(),
            None => t!(l, "snippet-partial-unknown").into_owned(),
        };
        div()
            .debug_selector(|| "snippet-cut-note".into())
            .px(metrics::SPACE_5)
            .py(metrics::SPACE_3)
            .border_t_1()
            .border_color(tokens.code_border)
            .text_size(metrics::FONT_SIZE_2XS)
            .text_color(tokens.text_subtle)
            .child(text)
    });
    div()
        .id(id)
        .debug_selector(|| "snippet-card".into())
        .flex()
        .flex_col()
        .my(metrics::SPACE_2)
        .w_full()
        .max_w(metrics::LAYOUT_CODE_MAX_W)
        .overflow_hidden()
        .rounded(metrics::RADIUS_LG)
        .border_1()
        .border_color(tokens.code_border)
        .bg(colors.surface_background)
        .child(head)
        .child(body)
        .children(note)
        .children(toggle)
}
