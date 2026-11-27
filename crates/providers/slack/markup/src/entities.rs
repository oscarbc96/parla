//! HTML entity decoding and angle-bracket token parsing (winnow).

use domain::{Broadcast, ConversationId, Inline, Style, UserId};
use winnow::combinator::{alt, opt, preceded, terminated};
use winnow::prelude::*;
use winnow::token::{take_till, take_while};

/// Decode `&amp;`, `&lt;`, `&gt;` exactly once, left to right.
pub(crate) fn decode(s: &str) -> String {
    if !s.contains('&') {
        return s.to_owned();
    }
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(p) = rest.find('&') {
        out.push_str(&rest[..p]);
        rest = &rest[p..];
        let (rep, len) = if rest.starts_with("&amp;") {
            ("&", 5)
        } else if rest.starts_with("&lt;") {
            ("<", 4)
        } else if rest.starts_with("&gt;") {
            (">", 4)
        } else {
            ("&", 1)
        };
        out.push_str(rep);
        rest = &rest[len..];
    }
    out.push_str(rest);
    out
}

/// What the text between `<` and `>` means.
pub(crate) enum Angle {
    Inline(Inline),
    /// Unknown `!command` with an optional label: shown as the label.
    Label(String),
}

fn id<'a>(i: &mut &'a str) -> ModalResult<&'a str> {
    take_while(1.., |c: char| c.is_ascii_alphanumeric()).parse_next(i)
}

fn label(i: &mut &str) -> ModalResult<Option<String>> {
    opt(preceded('|', take_till(0.., |_| false)))
        .map(|l: Option<&str>| l.filter(|l| !l.is_empty()).map(decode))
        .parse_next(i)
}

fn user(i: &mut &str) -> ModalResult<Angle> {
    (preceded('@', id), label)
        .map(|(id, l)| {
            Angle::Inline(Inline::User {
                id: UserId(id.into()),
                label: l.map(Into::into),
            })
        })
        .parse_next(i)
}

fn conversation(i: &mut &str) -> ModalResult<Angle> {
    (preceded('#', id), label)
        .map(|(id, l)| {
            Angle::Inline(Inline::Conversation {
                id: ConversationId(id.into()),
                label: l.map(Into::into),
            })
        })
        .parse_next(i)
}

fn broadcast(i: &mut &str) -> ModalResult<Angle> {
    terminated(
        preceded(
            '!',
            alt((
                "here".value(Broadcast::Here),
                "channel".value(Broadcast::Channel),
                "everyone".value(Broadcast::Everyone),
            )),
        ),
        label,
    )
    .map(|b| Angle::Inline(Inline::Broadcast(b)))
    .parse_next(i)
}

fn subteam(i: &mut &str) -> ModalResult<Angle> {
    (preceded("!subteam^", id), label)
        .map(|(id, l)| {
            Angle::Inline(Inline::Group {
                id: id.into(),
                label: l.map(Into::into),
            })
        })
        .parse_next(i)
}

fn date(i: &mut &str) -> ModalResult<Angle> {
    let (unix, _fmt, _link, fallback) = (
        preceded(
            "!date^",
            take_while(1.., |c: char| c.is_ascii_digit() || c == '-'),
        ),
        preceded('^', take_till(0.., |c| c == '^' || c == '|')),
        opt(preceded('^', take_till(0.., |c| c == '|'))),
        label,
    )
        .parse_next(i)?;
    // Out-of-range or malformed timestamps fall back to plain text.
    let Ok(unix) = unix.parse::<i64>() else {
        return Err(winnow::error::ErrMode::Backtrack(
            winnow::error::ContextError::new(),
        ));
    };
    Ok(Angle::Inline(Inline::Date {
        unix,
        fallback: fallback.unwrap_or_default().into(),
    }))
}

fn other_command(i: &mut &str) -> ModalResult<Angle> {
    (preceded('!', take_till(1.., |c| c == '|')), label)
        .map(|(c, l): (&str, _)| Angle::Label(l.unwrap_or_else(|| decode(c))))
        .parse_next(i)
}

fn link(i: &mut &str) -> ModalResult<Angle> {
    let (url, l) = (
        take_while(1.., |c: char| !c.is_whitespace() && c != '|'),
        label,
    )
        .parse_next(i)?;
    // Needs a scheme: `letters:` then something.
    let ok = url.split_once(':').is_some_and(|(s, r)| {
        !s.is_empty()
            && !r.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
    });
    if !ok {
        return Err(winnow::error::ErrMode::Backtrack(
            winnow::error::ContextError::new(),
        ));
    }
    Ok(Angle::Inline(Inline::Link {
        url: decode(url).into(),
        label: l.map(Into::into),
        style: Style::default(),
    }))
}

/// Parse the text between `<` and `>`; `None` means "not a token".
pub(crate) fn parse_angle(inner: &str) -> Option<Angle> {
    alt((
        user,
        conversation,
        broadcast,
        subteam,
        date,
        other_command,
        link,
    ))
    .parse(inner)
    .ok()
}
