//! First inline pass: tokenise a line into atoms and delimiter candidates.

use super::inline::text;
use crate::emoji;
use crate::entities::{self, Angle, decode};
use domain::Inline;

pub(super) enum Tok<'a> {
    Text(&'a str),
    Atom(Inline),
    Mark { ch: char, open: bool, close: bool },
}

fn is_special(b: u8) -> bool {
    matches!(b, b'`' | b'<' | b':' | b'*' | b'_' | b'~')
}

pub(super) fn tokenize(line: &str) -> Vec<Tok<'_>> {
    let bytes = line.as_bytes();
    let mut toks = Vec::new();
    let (mut i, mut start) = (0, 0);
    // Forward-only URL state for `glue_ok`: last `://` seen and last
    // whitespace seen so far (`ws_scan` only ever advances).
    let mut url = UrlState::default();
    macro_rules! flush {
        () => {
            if start < i {
                toks.push(Tok::Text(&line[start..i]));
            }
        };
    }
    while i < bytes.len() {
        if !is_special(bytes[i]) {
            i += 1;
            continue;
        }
        let next_char = line[i + 1..].chars().next();
        let prev_char = line[..i].chars().next_back();
        match bytes[i] {
            b'`' => {
                if let Some(j) = line[i + 1..].find('`').filter(|j| *j > 0) {
                    flush!();
                    toks.push(Tok::Atom(Inline::Code(
                        decode(&line[i + 1..i + 1 + j]).into(),
                    )));
                    i += j + 2;
                    start = i;
                    continue;
                }
            }
            b'<' => {
                // The first `<` or `>` after this one decides; keeps scans disjoint.
                if let Some(j) = line[i + 1..]
                    .find(['<', '>'])
                    .filter(|j| line.as_bytes()[i + 1 + j] == b'>')
                {
                    let inner = &line[i + 1..i + 1 + j];
                    match entities::parse_angle(inner) {
                        Some(Angle::Inline(a)) => {
                            flush!();
                            toks.push(Tok::Atom(a));
                            i += j + 2;
                            start = i;
                            continue;
                        }
                        Some(Angle::Label(l)) => {
                            flush!();
                            toks.push(Tok::Atom(text(&l)));
                            i += j + 2;
                            start = i;
                            continue;
                        }
                        None => {}
                    }
                }
            }
            b':' => {
                url.advance(line, i);
                let in_url = url.in_word();
                if bytes[i + 1..].starts_with(b"//") {
                    url.colon = Some(i);
                }
                if let Some((e, len)) = emoji_at(&line[i..])
                    && glue_ok(line, i, len, prev_char, in_url)
                {
                    flush!();
                    toks.push(Tok::Atom(Inline::Emoji(e)));
                    i += len;
                    start = i;
                    continue;
                }
            }
            b => {
                let ch = char::from(b);
                let open = !prev_char.is_some_and(char::is_alphanumeric)
                    && next_char.is_some_and(|c| !c.is_whitespace());
                let close = prev_char.is_some_and(|c| !c.is_whitespace())
                    && !next_char.is_some_and(char::is_alphanumeric);
                // `\_` is a literal underscore: the shrug `¯\_(ツ)_/¯` is not italic.
                let escaped = ch == '_' && prev_char == Some('\\');
                if (open || close) && !escaped {
                    flush!();
                    toks.push(Tok::Mark { ch, open, close });
                    i += 1;
                    start = i;
                    continue;
                }
            }
        }
        i += 1;
    }
    flush!();
    toks
}

/// Where the last `://` and the last whitespace sit, so "is this colon inside
/// a URL word" is answered without looking back.
#[derive(Default)]
struct UrlState {
    colon: Option<usize>,
    ws: Option<usize>,
    scanned: usize,
}

impl UrlState {
    /// Records whitespace in `line[scanned..i]`; total work is linear.
    fn advance(&mut self, line: &str, i: usize) {
        for (off, c) in line[self.scanned..i].char_indices() {
            if c.is_whitespace() {
                self.ws = Some(self.scanned + off);
            }
        }
        self.scanned = i;
    }

    /// The word ending here already contained `://`.
    fn in_word(&self) -> bool {
        match (self.colon, self.ws) {
            (Some(c), Some(w)) => c > w,
            (Some(_), None) => true,
            _ => false,
        }
    }
}

/// Shortcodes may touch the text before them (`hey:wave:`), but not inside a
/// bare URL, not as the middle of a time or `a:b:c` list.
fn glue_ok(line: &str, i: usize, len: usize, prev: Option<char>, in_url: bool) -> bool {
    if in_url {
        return false;
    }
    if !prev.is_some_and(char::is_alphanumeric) {
        return true;
    }
    let name = &line[i + 1..i + len];
    let name = name.split(':').next().unwrap_or("");
    !name.bytes().all(|b| b.is_ascii_digit())
        && !line[i + len..]
            .chars()
            .next()
            .is_some_and(char::is_alphanumeric)
}

/// `:name:` optionally followed by `:skin-tone-N:`; returns bytes consumed.
fn emoji_at(s: &str) -> Option<(domain::EmojiRef, usize)> {
    let b = s.as_bytes();
    let n = b[1..]
        .iter()
        .take(100)
        .take_while(|c| emoji::is_name_char(**c))
        .count();
    if n == 0 || b.get(1 + n) != Some(&b':') {
        return None;
    }
    let name = &s[1..1 + n];
    let mut len = n + 2;
    let mut tone = None;
    if let Some(r) = s[len..].strip_prefix(":skin-tone-") {
        let d = r.bytes().take_while(u8::is_ascii_digit).count();
        if d > 0
            && r.as_bytes().get(d) == Some(&b':')
            && let Some(t) = emoji::parse_tone(&r[..d])
        {
            tone = Some(t);
            len += ":skin-tone-".len() + d + 1;
        }
    }
    Some((emoji::build(name, tone), len))
}
