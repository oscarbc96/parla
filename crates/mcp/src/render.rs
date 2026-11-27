//! Domain to tool DTOs, truncation and the result size cap.

use std::collections::HashMap;

use domain::{Conversation, ConversationKind, Member, Message, UserId};
use rmcp::model::{CallToolResult, ContentBlock, MetaObject};
use serde::Serialize;
use serde_json::{Map, Value, json};
use settings::WorkspaceKey;

use crate::dto::{AuthorOut, ConversationOut, MessageOut, UserOut};

/// Longest message text a tool returns, in characters.
pub const MAX_TEXT_CHARS: usize = 4_000;
/// Largest serialised tool result, in bytes.
pub const MAX_RESULT_BYTES: usize = 64 * 1024;

pub fn kind_name(k: ConversationKind) -> &'static str {
    match k {
        ConversationKind::Channel => "channel",
        ConversationKind::PrivateChannel => "private",
        ConversationKind::Direct => "dm",
        ConversationKind::Group => "group",
    }
}

/// Cuts at a character boundary; the flag says whether anything was cut.
pub fn truncate_text(s: &str, max_chars: usize) -> (String, bool) {
    match s.char_indices().nth(max_chars) {
        Some((i, _)) => (s[..i].to_string(), true),
        None => (s.to_string(), false),
    }
}

pub fn conversation_out(ws: &WorkspaceKey, c: &Conversation) -> ConversationOut {
    let name = if c.name.is_empty() {
        c.peer.as_ref().map(|p| p.0.to_string()).unwrap_or_default()
    } else {
        c.name.to_string()
    };
    ConversationOut {
        workspace: ws.to_string(),
        conversation: c.id.0.to_string(),
        kind: kind_name(c.kind).to_string(),
        untrusted_name: name,
        unread: c.unread,
        mentions: c.mentions,
        latest: c.latest.as_ref().map(|t| t.0.to_string()),
        last_read: c.last_read.as_ref().map(|t| t.0.to_string()),
    }
}

pub fn user_out(ws: &WorkspaceKey, m: &Member) -> UserOut {
    let status = m
        .status
        .as_ref()
        .filter(|s| !s.text.is_empty() || s.emoji.is_some());
    UserOut {
        workspace: ws.to_string(),
        user: m.id.0.to_string(),
        untrusted_display_name: m.display_name.to_string(),
        untrusted_real_name: m.real_name.to_string(),
        is_bot: m.is_bot,
        deleted: m.deleted,
        untrusted_status_text: status.map(|s| truncate_text(&s.text, 200).0),
        status_emoji: status
            .and_then(|s| s.emoji.as_ref())
            .map(|e| e.shortcode.to_string()),
    }
}

/// Author display names resolved so far in one call.
pub type Names = HashMap<UserId, String>;

pub fn message_out(
    ws: &WorkspaceKey,
    m: &Message,
    names: &Names,
    link: Option<String>,
) -> MessageOut {
    let (text, truncated) = truncate_text(&m.plain, MAX_TEXT_CHARS);
    MessageOut {
        workspace: ws.to_string(),
        conversation: m.id.conversation.0.to_string(),
        ts: m.id.ts.0.to_string(),
        thread_ts: m.thread_root.as_ref().map(|t| t.0.to_string()),
        reply_count: m.reply_count,
        author: m.author.as_ref().map(|u| AuthorOut {
            user: u.0.to_string(),
            untrusted_name: names.get(u).cloned().unwrap_or_else(|| u.0.to_string()),
        }),
        untrusted_text: text,
        truncated,
        edited: m.edited.is_some(),
        reactions: m
            .reactions
            .iter()
            .map(|r| (r.emoji.shortcode.to_string(), r.count))
            .collect(),
        files: m.files.iter().map(|f| f.name.to_string()).collect(),
        link,
    }
}

/// The largest `k <= n` such that `size(k)` (the serialised bytes with the
/// first `k` items) fits the cap. `size` must grow with `k`.
pub fn fit(n: usize, size: impl Fn(usize) -> usize) -> usize {
    if size(n) <= MAX_RESULT_BYTES {
        return n;
    }
    let (mut lo, mut hi) = (0usize, n);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if size(mid) <= MAX_RESULT_BYTES {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    lo
}

pub fn json_len<T: Serialize>(v: &T) -> usize {
    serde_json::to_vec(v).map_or(usize::MAX, |b| b.len())
}

/// Caps `items` at the result size: `build` makes the reply from the first `k`
/// items and whether any were dropped (`more` or the cap). `build` returns a
/// value borrowing `items` (a small struct, not a `Value`), so the size probes
/// serialise without copying the items.
pub fn capped<'a, T, R>(
    items: &'a [T],
    more: bool,
    build: impl Fn(&'a [T], bool) -> R,
) -> CallToolResult
where
    R: Serialize,
{
    let k = fit(items.len(), |k| json_len(&build(&items[..k], true)));
    ok_result(&build(&items[..k], more || k < items.len()))
}

/// A successful result: the JSON once as `structuredContent` and once as the
/// text block, flagged as untrusted content.
pub fn ok_result<T: Serialize>(v: &T) -> CallToolResult {
    let value = serde_json::to_value(v).unwrap_or(Value::Null);
    let mut r = CallToolResult::success(vec![ContentBlock::text(value.to_string())]);
    r.structured_content = Some(value);
    let mut meta = Map::new();
    meta.insert("parla/untrusted_content".into(), json!(true));
    r.meta = Some(MetaObject(meta));
    r
}
