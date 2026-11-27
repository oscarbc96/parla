//! Search: query text in the web client's modifier syntax and hit mapping
//! (UNVERIFIED, see docs/providers/slack/web-client/search.md).

use std::sync::Arc;

use domain::{
    ConversationId, FileHit, Has, HitSource, Member, MessageHit, Page, ProviderError, SearchQuery,
    UserId,
};
use slack_api::{SearchFile, SearchMessage};

use crate::SlackProvider;
use crate::map::{map_conversation, map_message_as, map_user, ts_opt};
use crate::provider::OrFail;

/// Match delimiters the server wraps around hits when `highlight` is on.
const HL_START: char = '\u{e000}';
const HL_END: char = '\u{e001}';

/// A name that can follow `in:#` or `from:@` as one token.
fn safe(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// Free text made safe: `"` balanced (an unpaired last quote is dropped) and
/// every word containing `:` quoted, so user text can never act as a
/// server-side operator (`-in:`, `is:`, `to:`, malformed dates, ...). Quoted
/// phrases are kept as typed.
fn free_text(text: &str) -> String {
    let quotes = text.matches('"').count();
    let mut text = text.to_string();
    if !quotes.is_multiple_of(2) {
        let at = text.rfind('"').unwrap_or(0);
        text.remove(at);
    }
    let mut out: Vec<String> = Vec::new();
    let mut rest = text.as_str();
    loop {
        rest = rest.trim_start();
        let Some(c) = rest.chars().next() else { break };
        if c == '"' {
            let end = rest[1..].find('"').map_or(rest.len(), |i| i + 2);
            out.push(rest[..end].to_string());
            rest = &rest[end..];
        } else {
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '"')
                .unwrap_or(rest.len());
            let w = &rest[..end];
            out.push(if w.contains(':') {
                format!("\"{w}\"")
            } else {
                w.to_string()
            });
            rest = &rest[end..];
        }
    }
    out.join(" ")
}

/// The provider's search text for a neutral query. Names that cannot be one
/// token (spaces, quotes) or are unknown use the id form `in:<#C..>` and
/// `from:<@U..>`. Dates are `before:`/`after:` days as the server reads them.
pub fn to_query(
    q: &SearchQuery,
    conv_name: &impl Fn(&ConversationId) -> Option<Arc<str>>,
    user_name: &impl Fn(&UserId) -> Option<Arc<str>>,
) -> String {
    let mut parts: Vec<String> = Vec::new();
    let text = free_text(&q.text);
    if !text.is_empty() {
        parts.push(text);
    }
    for c in &q.conversations {
        match conv_name(c) {
            Some(n) if safe(&n) => parts.push(format!("in:#{n}")),
            _ => parts.push(format!("in:<#{}>", c.0)),
        }
    }
    for u in &q.authors {
        match user_name(u) {
            Some(n) if safe(&n) => parts.push(format!("from:@{n}")),
            _ => parts.push(format!("from:<@{}>", u.0)),
        }
    }
    if let Some(d) = q.before {
        parts.push(format!("before:{d}"));
    }
    if let Some(d) = q.after {
        parts.push(format!("after:{d}"));
    }
    for h in &q.has {
        parts.push(
            match h {
                Has::File => "has:file",
                Has::Link => "has:link",
                Has::Reaction => "has:reaction",
                Has::Pin => "has:pin",
            }
            .to_string(),
        );
    }
    parts.join(" ")
}

/// Snippet text: highlight delimiters removed, markup flattened.
pub fn snippet(text: &str) -> String {
    slack_markup::plain_text(&strip_hl(text))
}

impl SlackProvider {
    pub(crate) async fn do_search_messages(
        &self,
        q: &SearchQuery,
        cursor: Option<Arc<str>>,
        limit: u32,
    ) -> Result<Page<MessageHit>, ProviderError> {
        // Ids only: the provider keeps no name table, and the id form is always valid.
        let query = to_query(q, &|_| None, &|_| None);
        let r = self
            .sh
            .client
            .search_messages(&query, cursor.as_deref(), limit)
            .await
            .or_fail(&self.sh)?;
        let me = Some(self.sh.self_user.as_str());
        let items = r
            .items
            .into_iter()
            .map(|SearchMessage { channel, message }| {
                // Markers go before mapping so `plain` and `mentions` are clean.
                let mut message = message;
                message.text = strip_hl(&message.text);
                let snippet = Arc::from(slack_markup::plain_text(&message.text));
                let message = map_message_as(&channel, message, me);
                MessageHit {
                    message,
                    snippet,
                    source: HitSource::Remote,
                }
            })
            .collect();
        Ok(Page {
            items,
            next: r.next_cursor.map(Arc::from),
        })
    }

    pub(crate) async fn do_search_files(
        &self,
        q: &SearchQuery,
        cursor: Option<Arc<str>>,
        limit: u32,
    ) -> Result<Page<FileHit>, ProviderError> {
        let query = to_query(q, &|_| None, &|_| None);
        let r = self
            .sh
            .client
            .search_files(&query, cursor.as_deref(), limit)
            .await
            .or_fail(&self.sh)?;
        let items = r
            .items
            .into_iter()
            .map(
                |SearchFile {
                     file,
                     channel,
                     user,
                     ts,
                 }| {
                    let shared = file.shares.as_ref().and_then(|_| {
                        let ch = channel.clone().or_else(|| first_share_channel(&file))?;
                        Some((file.share_ts(&ch), ch))
                    });
                    let conversation = channel.or_else(|| shared.as_ref().map(|(_, c)| c.clone()));
                    let ts = ts.or_else(|| shared.and_then(|(t, _)| t));
                    FileHit {
                        file: crate::files::map_file(&file),
                        conversation: conversation.map(|c| ConversationId(Arc::from(c))),
                        author: user.map(|u| UserId(Arc::from(u))),
                        ts: ts.as_deref().and_then(ts_opt),
                    }
                },
            )
            .collect();
        Ok(Page {
            items,
            next: r.next_cursor.map(Arc::from),
        })
    }

    pub(crate) async fn do_search_people(
        &self,
        text: &str,
        limit: u32,
    ) -> Result<Vec<Member>, ProviderError> {
        let users = self
            .sh
            .client
            .search_people(text, limit)
            .await
            .or_fail(&self.sh)?;
        Ok(users.into_iter().map(map_user).collect())
    }

    pub(crate) async fn do_search_conversations(
        &self,
        text: &str,
        limit: u32,
    ) -> Result<Vec<domain::Conversation>, ProviderError> {
        let chans = self
            .sh
            .client
            .search_channels(text, limit)
            .await
            .or_fail(&self.sh)?;
        Ok(chans
            .iter()
            .map(|c| map_conversation(c, None, &|_| None))
            .collect())
    }
}

fn strip_hl(s: &str) -> String {
    s.chars()
        .filter(|c| *c != HL_START && *c != HL_END)
        .collect()
}

fn first_share_channel(f: &slack_api::File) -> Option<String> {
    f.shares
        .as_ref()?
        .as_object()?
        .values()
        .filter_map(|scope| scope.as_object()?.keys().next().cloned())
        .next()
}
