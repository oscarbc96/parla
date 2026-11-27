//! Composed text to Slack mrkdwn.

use crate::entities::{Angle, decode, parse_angle};
use domain::{Broadcast, ComposedSpan, ComposedText, ContentEncoder, Inline};

/// Escapes `&`, `<` and `>` in typed text and writes picked mentions as tokens.
/// Text the user typed never becomes markup: only spans do.
pub struct SlackEncoder;

fn escape_into(out: &mut String, s: &str) {
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
}

fn token(i: &Inline) -> Option<String> {
    Some(match i {
        Inline::User { id, .. } => format!("<@{}>", id.0),
        Inline::Conversation { id, .. } => format!("<#{}>", id.0),
        Inline::Group { id, .. } => format!("<!subteam^{id}>"),
        Inline::Broadcast(Broadcast::Here) => "<!here>".into(),
        Inline::Broadcast(Broadcast::Channel) => "<!channel>".into(),
        Inline::Broadcast(Broadcast::Everyone) => "<!everyone>".into(),
        Inline::Emoji(e) => format!(":{}:", e.shortcode),
        _ => return None,
    })
}

impl ContentEncoder for SlackEncoder {
    fn encode(&self, t: &ComposedText) -> String {
        let text: &str = &t.text;
        let mut spans: Vec<_> = t
            .spans
            .iter()
            .filter(|s| {
                s.range.start <= s.range.end
                    && s.range.end <= text.len()
                    && text.is_char_boundary(s.range.start)
                    && text.is_char_boundary(s.range.end)
            })
            .collect();
        spans.sort_by_key(|s| s.range.start);
        let mut out = String::with_capacity(text.len() + 16);
        let mut at = 0;
        for s in spans {
            // An overlapping span is dropped; its text stays literal.
            if s.range.start < at {
                continue;
            }
            let label = &text[s.range.clone()];
            let tok = match &s.inline {
                Inline::Link { url, .. } if !url.is_empty() => {
                    let mut t = String::from("<");
                    escape_into(&mut t, url);
                    if label != &**url {
                        t.push('|');
                        escape_into(&mut t, label);
                    }
                    t.push('>');
                    t
                }
                other => match token(other) {
                    Some(t) => t,
                    None => continue,
                },
            };
            escape_into(&mut out, &text[at..s.range.start]);
            out.push_str(&tok);
            at = s.range.end;
        }
        escape_into(&mut out, &text[at..]);
        out
    }

    fn decode(&self, markup: &str) -> ComposedText {
        let mut text = String::with_capacity(markup.len());
        let mut spans = Vec::new();
        let mut rest = markup;
        while let Some(open) = rest.find('<') {
            let Some(close) = rest[open..].find('>').map(|i| open + i) else {
                break;
            };
            text.push_str(&decode(&rest[..open]));
            let inner = &rest[open + 1..close];
            match parse_angle(inner) {
                // A date or an unknown command has no neutral form: keep its token as one
                // link-like unit that encodes back verbatim.
                Some(Angle::Inline(Inline::Date { .. })) | Some(Angle::Label(_)) => {
                    let (target, label) = inner.split_once('|').unwrap_or((inner, ""));
                    let (target, label) = (decode(target), decode(label));
                    let start =
                        shown_start(&mut text, if label.is_empty() { &target } else { &label });
                    spans.push(ComposedSpan {
                        range: start..text.len(),
                        inline: Inline::Link {
                            url: target.as_str().into(),
                            label: None,
                            style: Default::default(),
                        },
                    });
                }
                Some(Angle::Inline(i)) => {
                    let start = text.len();
                    text.push_str(&span_text(&i));
                    spans.push(ComposedSpan {
                        range: start..text.len(),
                        inline: i,
                    });
                }
                None => text.push_str(&decode(&rest[open..=close])),
            }
            rest = &rest[close + 1..];
        }
        text.push_str(&decode(rest));
        ComposedText::new(&text, spans)
    }
}

/// Appends `shown` to `text` and returns where it starts.
fn shown_start(text: &mut String, shown: &str) -> usize {
    let start = text.len();
    text.push_str(shown);
    start
}

/// The text a decoded token shows in the composer.
fn span_text(i: &Inline) -> String {
    match i {
        Inline::User { id, label } => format!("@{}", label.as_deref().unwrap_or(&id.0)),
        Inline::Conversation { id, label } => format!("#{}", label.as_deref().unwrap_or(&id.0)),
        Inline::Group { id, label } => format!(
            "@{}",
            label
                .as_deref()
                .map_or(&**id, |l| l.trim_start_matches('@'))
        ),
        Inline::Broadcast(Broadcast::Here) => "@here".into(),
        Inline::Broadcast(Broadcast::Channel) => "@channel".into(),
        Inline::Broadcast(Broadcast::Everyone) => "@everyone".into(),
        Inline::Link { url, label, .. } => label.as_deref().unwrap_or(url).to_string(),
        _ => String::new(),
    }
}
