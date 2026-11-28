//! Composer pickers: `@` people and groups, `#` conversations and `:` emoji.

use domain::{
    Broadcast, ComposedSpan, ConversationId, ConversationKind, CustomEmoji, EmojiRef, Inline,
    WorkspaceState,
};
use gpui::SharedString;

use crate::emoji_picker::fuzzy_score;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompletionKind {
    Person,
    Conversation,
    Emoji,
}

/// One row of a picker. `label` is what accepting inserts.
#[derive(Clone, Debug, PartialEq)]
pub struct Completion {
    pub label: SharedString,
    pub detail: Option<SharedString>,
    /// Members of a user group row (shown as "Group · N people"); `None` otherwise.
    pub members: Option<u32>,
    /// "Group · N people" for a group row, localised once when the picker opens.
    pub members_label: Option<SharedString>,
    /// A custom emoji's image, drawn before the label; `None` for other rows.
    pub image: Option<domain::ImageRef>,
    pub inline: domain::Inline,
}

/// A trigger word at the caret: `@`, `#` or `:` at the start of a word.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trigger {
    pub kind: CompletionKind,
    /// Byte offset of the trigger character.
    pub start: usize,
    /// What was typed after it.
    pub query: String,
}

/// The picker the caret is in, if any. `@` and `#` open at once, `:` after two characters;
/// the trigger must start a word (so `a@b` and `mail#x` stay text), and a picked span
/// never reopens one.
pub fn detect(text: &str, caret: usize, spans: &[ComposedSpan]) -> Option<Trigger> {
    let prefix = text.get(..caret)?;
    let start = prefix
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map_or(0, |(i, c)| i + c.len_utf8());
    if spans
        .iter()
        .any(|s| s.range.start < caret && start < s.range.end)
    {
        return None;
    }
    let word = &prefix[start..];
    let mut chars = word.chars();
    let kind = match chars.next()? {
        '@' => CompletionKind::Person,
        '#' => CompletionKind::Conversation,
        ':' => CompletionKind::Emoji,
        _ => return None,
    };
    let query = chars.as_str();
    if kind == CompletionKind::Emoji
        && (query.chars().count() < 2
            || !query
                .chars()
                .all(|c| c.is_alphanumeric() || matches!(c, '_' | '+' | '-')))
    {
        return None;
    }
    Some(Trigger {
        kind,
        start,
        query: query.to_string(),
    })
}

/// People and user groups (and `@here`, `@channel`, `@everyone` outside direct messages)
/// matching `query`. Broadcasts lead, then names that start with the query (alphabetically),
/// then the other fuzzy matches by score. A group matches on its handle and name.
pub fn people(
    state: &WorkspaceState,
    conversation: Option<&ConversationId>,
    query: &str,
) -> Vec<Completion> {
    let q = query.to_lowercase();
    // (bucket: 0 broadcast, 1 prefix, 2 fuzzy; score; sort name; row)
    let mut rows: Vec<(u8, i32, String, Completion)> = Vec::new();
    let direct = conversation
        .and_then(|c| state.conversations.get(c))
        .is_some_and(|c| c.kind == ConversationKind::Direct);
    if !direct {
        for (name, b) in [
            ("here", Broadcast::Here),
            ("channel", Broadcast::Channel),
            ("everyone", Broadcast::Everyone),
        ] {
            if name.starts_with(&q) {
                // The three keep their order: `here` < `channel` < `everyone` is by index.
                rows.push((
                    0,
                    -(rows.len() as i32),
                    String::new(),
                    Completion {
                        label: format!("@{name}").into(),
                        detail: None,
                        members: None,
                        members_label: None,
                        image: None,
                        inline: Inline::Broadcast(b),
                    },
                ));
            }
        }
    }
    for m in state.members.values().filter(|m| !m.deleted) {
        let Some(name) = crate::sidebar::best_name(m) else {
            continue;
        };
        let name: &str = &name;
        let score = [&*m.display_name, &*m.real_name, &*m.handle]
            .into_iter()
            .filter_map(|n| fuzzy_score(query, n))
            .max();
        let Some(score) = score else { continue };
        let prefix = !q.is_empty() && name.to_lowercase().starts_with(&q);
        let detail = (!m.handle.is_empty() && *m.handle != *name).then(|| format!("@{}", m.handle));
        rows.push((
            if prefix { 1 } else { 2 },
            score,
            name.to_lowercase(),
            Completion {
                label: format!("@{name}").into(),
                detail: detail.map(Into::into),
                members: None,
                members_label: None,
                image: None,
                inline: Inline::User {
                    id: m.id.clone(),
                    label: None,
                },
            },
        ));
    }
    for g in state.user_groups.values() {
        let score = [&*g.handle, &*g.name]
            .into_iter()
            .filter_map(|n| fuzzy_score(query, n))
            .max();
        let Some(score) = score else { continue };
        let prefix = !q.is_empty()
            && (g.handle.to_lowercase().starts_with(&q) || g.name.to_lowercase().starts_with(&q));
        let detail = (!g.name.is_empty() && *g.name != *g.handle).then(|| g.name.clone());
        rows.push((
            if prefix { 1 } else { 2 },
            score,
            g.handle.to_lowercase(),
            Completion {
                label: format!("@{}", g.handle).into(),
                detail: detail.map(|d| d.to_string().into()),
                members: Some(g.member_count),
                members_label: None,
                image: None,
                inline: Inline::Group {
                    id: g.id.clone(),
                    label: None,
                },
            },
        ));
    }
    rows.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| match a.0 {
                // Prefix matches and the empty query sort by name alone.
                1 => std::cmp::Ordering::Equal,
                0 => b.1.cmp(&a.1),
                _ if q.is_empty() => std::cmp::Ordering::Equal,
                _ => b.1.cmp(&a.1),
            })
            .then_with(|| a.2.cmp(&b.2))
    });
    rows.into_iter().take(SOURCE_LIMIT).map(|r| r.3).collect()
}

/// Channels and private channels matching `query`.
pub fn conversations(state: &WorkspaceState, query: &str) -> Vec<Completion> {
    let mut rows: Vec<(i32, &str, Completion)> = state
        .conversations
        .values()
        .filter(|c| {
            matches!(
                c.kind,
                ConversationKind::Channel | ConversationKind::PrivateChannel
            ) && !c.is_archived
                && !c.name.is_empty()
        })
        .filter_map(|c| {
            fuzzy_score(query, &c.name).map(|s| {
                (
                    s,
                    &*c.name,
                    Completion {
                        label: format!("#{}", c.name).into(),
                        detail: None,
                        members: None,
                        members_label: None,
                        image: None,
                        inline: Inline::Conversation {
                            id: c.id.clone(),
                            label: None,
                        },
                    },
                )
            })
        })
        .collect();
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(b.1)));
    rows.into_iter().take(SOURCE_LIMIT).map(|r| r.2).collect()
}

/// Standard and custom emoji whose shortcode matches `query`, best first.
pub fn emoji<'a>(
    custom: impl IntoIterator<Item = &'a CustomEmoji>,
    query: &str,
) -> Vec<Completion> {
    let mut rows: Vec<(i32, String, Completion)> = Vec::new();
    for c in custom {
        if let Some(s) = fuzzy_score(query, &c.shortcode) {
            rows.push((
                s,
                c.shortcode.to_string(),
                Completion {
                    label: format!(":{}:", c.shortcode).into(),
                    detail: None,
                    members: None,
                    members_label: None,
                    image: Some(c.image.clone()),
                    inline: Inline::Emoji(EmojiRef {
                        shortcode: c.shortcode.clone(),
                        unicode: None,
                        skin_tone: None,
                    }),
                },
            ));
        }
    }
    for e in emojis::iter().filter(|e| e.skin_tone().is_none_or(|t| t == emojis::SkinTone::Default))
    {
        let best = e
            .shortcodes()
            .filter_map(|c| fuzzy_score(query, c).map(|s| (s, c)))
            .max_by_key(|(s, _)| *s);
        if let Some((s, code)) = best {
            rows.push((
                s,
                code.to_string(),
                Completion {
                    label: e.as_str().to_string().into(),
                    detail: Some(format!(":{code}:").into()),
                    members: None,
                    members_label: None,
                    image: None,
                    inline: Inline::Emoji(EmojiRef {
                        shortcode: code.into(),
                        unicode: Some(e.as_str().into()),
                        skin_tone: None,
                    }),
                },
            ));
        }
    }
    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    rows.into_iter().take(SOURCE_LIMIT).map(|r| r.2).collect()
}

/// A provider's decoded markup with each mention shown by its current name, as the
/// completion inserts it: people by name, channels by `#name`, groups by `@handle`
/// (each falling back to the token's own label).
pub fn relabel_decoded(state: &WorkspaceState, t: &domain::ComposedText) -> domain::ComposedText {
    t.relabel(|i| match i {
        Inline::User { id, label } => state
            .members
            .get(id)
            .and_then(crate::sidebar::best_name)
            .map(|n| format!("@{n}"))
            .or_else(|| label.as_ref().map(|l| format!("@{l}"))),
        Inline::Conversation { id, label } => state
            .conversations
            .get(id)
            .filter(|c| !c.name.is_empty())
            .map(|c| format!("#{}", c.name))
            .or_else(|| label.as_ref().map(|l| format!("#{l}"))),
        Inline::Group { id, label } => state
            .user_groups
            .get(id)
            .filter(|g| !g.handle.is_empty())
            .map(|g| format!("@{}", g.handle))
            .or_else(|| {
                label
                    .as_ref()
                    .map(|l| format!("@{}", l.trim_start_matches('@')))
            }),
        _ => None,
    })
}

/// Rows kept per query before the composer applies its own limit.
pub const SOURCE_LIMIT: usize = 40;

#[cfg(test)]
mod tests {
    use super::*;

    fn kq(t: &str) -> Option<(CompletionKind, usize, String)> {
        detect(t, t.len(), &[]).map(|t| (t.kind, t.start, t.query))
    }

    #[test]
    fn triggers_start_a_word() {
        assert_eq!(kq("@"), Some((CompletionKind::Person, 0, "".into())));
        assert_eq!(
            kq("hi #ge"),
            Some((CompletionKind::Conversation, 3, "ge".into()))
        );
        assert_eq!(kq("x\n:ta"), Some((CompletionKind::Emoji, 2, "ta".into())));
        assert_eq!(kq("a@b"), None);
        assert_eq!(kq("mail#x"), None);
        assert_eq!(kq(":t"), None);
        assert_eq!(kq(":+1"), Some((CompletionKind::Emoji, 0, "+1".into())));
        assert_eq!(kq(":tada:"), None);
        assert_eq!(kq("@ana "), None);
        assert_eq!(kq("日本 @a"), Some((CompletionKind::Person, 7, "a".into())));
    }
}
