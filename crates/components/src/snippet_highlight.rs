//! A small syntax highlighter for snippet lines. It is a tokenizer, not a parser: per
//! line it finds comments, strings, numbers, keywords, calls and type names for the
//! common languages. A block comment or multi-line string colours only the line it
//! starts on. Work per line is bounded by [`MAX_LINE_BYTES`].

use std::ops::Range;

use crate::snippet::Language;

/// Bytes of one line that are highlighted; the rest of a longer line is drawn plain.
pub const MAX_LINE_BYTES: usize = 2_000;

/// What a highlighted span is (`code.*` design tokens).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Keyword,
    String,
    Function,
    Comment,
    Number,
    Type,
}

struct Spec {
    keywords: &'static [&'static str],
    line_comments: &'static [&'static str],
    /// Keywords match whatever the case (SQL).
    fold_case: bool,
    /// A `Capitalised` word is a type.
    types: bool,
    /// A word followed by `(` is a call.
    calls: bool,
    /// `'` opens a string (not in Rust, where it also marks lifetimes).
    single_quote: bool,
    /// A backtick opens a string.
    backtick: bool,
}

const C_LIKE: &[&str] = &["//"];
const HASH: &[&str] = &["#"];

fn spec(id: &str) -> Option<Spec> {
    let s = |keywords, line_comments, types, calls| Spec {
        keywords,
        line_comments,
        fold_case: false,
        types,
        calls,
        single_quote: true,
        backtick: false,
    };
    Some(match id {
        "rust" => Spec {
            single_quote: false,
            ..s(
                &[
                    "as", "async", "await", "break", "const", "continue", "crate", "else", "enum",
                    "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match",
                    "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static",
                    "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while",
                ],
                C_LIKE,
                true,
                true,
            )
        },
        "python" => s(
            &[
                "and", "as", "assert", "async", "await", "break", "class", "continue", "def",
                "del", "elif", "else", "except", "False", "finally", "for", "from", "if", "import",
                "in", "is", "lambda", "None", "not", "or", "pass", "raise", "return", "True",
                "try", "while", "with", "yield",
            ],
            HASH,
            true,
            true,
        ),
        "javascript" | "typescript" => Spec {
            backtick: true,
            ..s(
                &[
                    "async",
                    "await",
                    "break",
                    "case",
                    "catch",
                    "class",
                    "const",
                    "continue",
                    "default",
                    "else",
                    "export",
                    "extends",
                    "false",
                    "finally",
                    "for",
                    "function",
                    "if",
                    "import",
                    "in",
                    "interface",
                    "let",
                    "new",
                    "null",
                    "of",
                    "return",
                    "switch",
                    "this",
                    "throw",
                    "true",
                    "try",
                    "type",
                    "typeof",
                    "undefined",
                    "var",
                    "while",
                    "yield",
                ],
                C_LIKE,
                true,
                true,
            )
        },
        "go" => Spec {
            backtick: true,
            ..s(
                &[
                    "break",
                    "case",
                    "chan",
                    "const",
                    "continue",
                    "default",
                    "defer",
                    "else",
                    "for",
                    "func",
                    "go",
                    "if",
                    "import",
                    "interface",
                    "map",
                    "nil",
                    "package",
                    "range",
                    "return",
                    "select",
                    "struct",
                    "switch",
                    "type",
                    "var",
                    "true",
                    "false",
                ],
                C_LIKE,
                true,
                true,
            )
        },
        "java" | "csharp" | "c" | "cpp" => s(
            &[
                "abstract",
                "break",
                "case",
                "catch",
                "class",
                "const",
                "continue",
                "default",
                "do",
                "else",
                "enum",
                "extends",
                "false",
                "final",
                "finally",
                "for",
                "if",
                "implements",
                "import",
                "include",
                "interface",
                "namespace",
                "new",
                "null",
                "package",
                "private",
                "protected",
                "public",
                "return",
                "static",
                "struct",
                "switch",
                "this",
                "throw",
                "true",
                "try",
                "using",
                "void",
                "while",
            ],
            C_LIKE,
            true,
            true,
        ),
        "ruby" => s(
            &[
                "alias", "and", "begin", "break", "case", "class", "def", "do", "else", "elsif",
                "end", "ensure", "false", "for", "if", "in", "module", "next", "nil", "not", "or",
                "require", "rescue", "return", "self", "then", "true", "unless", "until", "when",
                "while", "yield",
            ],
            HASH,
            true,
            false,
        ),
        "shell" => s(
            &[
                "case", "do", "done", "elif", "else", "esac", "export", "fi", "for", "function",
                "if", "in", "local", "then", "while",
            ],
            HASH,
            false,
            false,
        ),
        "sql" => Spec {
            fold_case: true,
            ..s(
                &[
                    "select", "from", "where", "insert", "into", "values", "update", "set",
                    "delete", "create", "table", "alter", "drop", "join", "left", "right", "inner",
                    "outer", "on", "group", "by", "order", "having", "limit", "and", "or", "not",
                    "null", "as", "distinct", "union", "index", "primary", "key",
                ],
                &["--"],
                false,
                false,
            )
        },
        "json" => s(&["true", "false", "null"], &[], false, false),
        "yaml" | "toml" => s(&["true", "false", "null"], HASH, false, false),
        _ => return None,
    })
}

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The highlighted spans of `line`, in order and not overlapping. Empty for plain text and
/// for languages without a tokenizer.
pub fn highlight(lang: &Language, line: &str) -> Vec<(Range<usize>, Class)> {
    highlight_within(lang, line, MAX_LINE_BYTES)
}

/// [`highlight`] of the first `max` bytes of `line` (the code viewer colours
/// as much of a line as it draws).
pub fn highlight_within(lang: &Language, line: &str, max: usize) -> Vec<(Range<usize>, Class)> {
    let Some(spec) = spec(lang.id) else {
        return Vec::new();
    };
    let mut end = line.len().min(max);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    let text = &line[..end];
    let b = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let rest = &text[i..];
        if spec.line_comments.iter().any(|c| rest.starts_with(c)) {
            out.push((i..b.len(), Class::Comment));
            break;
        }
        let c = b[i];
        let quote = match c {
            b'"' => true,
            b'\'' if spec.single_quote => true,
            b'`' if spec.backtick => true,
            // A Rust char literal closes within a few bytes; a lifetime does not.
            b'\'' => b[i + 1..].iter().take(4).any(|&x| x == b'\''),
            _ => false,
        };
        if quote {
            let mut j = i + 1;
            while j < b.len() && b[j] != c {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            let j = (j + 1).min(b.len());
            out.push((i..j, Class::String));
            i = j;
        } else if c.is_ascii_digit() && (i == 0 || !is_ident(b[i - 1])) {
            let mut j = i + 1;
            while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'.' || b[j] == b'_') {
                j += 1;
            }
            out.push((i..j, Class::Number));
            i = j;
        } else if c.is_ascii_alphabetic() || c == b'_' {
            let mut j = i + 1;
            while j < b.len() && is_ident(b[j]) {
                j += 1;
            }
            let word = &text[i..j];
            let is_kw = if spec.fold_case {
                spec.keywords.iter().any(|k| k.eq_ignore_ascii_case(word))
            } else {
                spec.keywords.contains(&word)
            };
            let call = spec.calls && text[j..].trim_start().starts_with('(');
            if is_kw {
                out.push((i..j, Class::Keyword));
            } else if call {
                out.push((i..j, Class::Function));
            } else if spec.types && c.is_ascii_uppercase() {
                out.push((i..j, Class::Type));
            }
            i = j;
        } else {
            // Step over one whole character.
            i += rest.chars().next().map_or(1, char::len_utf8);
        }
    }
    out
}
