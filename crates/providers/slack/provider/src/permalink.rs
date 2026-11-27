use std::sync::Arc;

use domain::{ConversationId, MessageTs, ParsedPermalink};
use url::Url;

/// Parses a Slack web permalink into a neutral target.
///
/// Shape: `https://<sub>.slack.com/archives/<C>/p<16 digits>[?thread_ts=..]`.
pub fn parse_permalink(url: &Url) -> Option<ParsedPermalink> {
    if url.scheme() != "https" {
        return None;
    }
    let host = url.host_str()?;
    if host.is_empty() {
        return None;
    }
    let mut segs = url.path_segments()?;
    if segs.next()? != "archives" {
        return None;
    }
    let conversation = segs.next()?.to_string();
    if conversation.is_empty() {
        return None;
    }
    let p = segs.next()?;
    if segs.next().is_some() {
        return None;
    }
    let ts = parse_p_ts(p)?;
    let thread = url
        .query_pairs()
        .find_map(|(k, v)| (k == "thread_ts").then(|| v.into_owned()))
        .and_then(|v| normalize_ts(&v));
    Some(ParsedPermalink {
        workspace_hint: Arc::from(host),
        conversation: ConversationId(Arc::from(conversation)),
        ts: Some(MessageTs(Arc::from(ts))),
        thread: thread.map(|t| MessageTs(Arc::from(t))),
    })
}

fn parse_p_ts(s: &str) -> Option<String> {
    let digits = s.strip_prefix('p')?;
    if digits.len() != 16 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let (sec, frac) = digits.split_at(10);
    Some(format!("{sec}.{frac}"))
}

fn normalize_ts(s: &str) -> Option<String> {
    if s.len() == 17
        && s.as_bytes().get(10) == Some(&b'.')
        && s.chars().enumerate().all(|(i, c)| {
            if i == 10 {
                c == '.'
            } else {
                c.is_ascii_digit()
            }
        })
    {
        return Some(s.to_string());
    }
    if s.len() == 16 && s.chars().all(|c| c.is_ascii_digit()) {
        let (sec, frac) = s.split_at(10);
        return Some(format!("{sec}.{frac}"));
    }
    None
}
