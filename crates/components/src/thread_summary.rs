//! The "N replies" row shown under a thread root.

use domain::{Message, MessageTs};
use i18n::Localizer;
use jiff::Timestamp;

/// At most this many reply authors are listed.
pub const MAX_AUTHORS: usize = 3;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadSummary {
    /// `"1 reply"` or `"N replies"`.
    pub label: String,
    pub authors: Vec<String>,
    /// Relative time of the latest reply, e.g. `"5m ago"`; empty when unknown.
    pub latest: String,
}

fn secs(ts: &MessageTs) -> Option<i64> {
    ts.0.split('.').next()?.parse().ok()
}

/// "just now", "5 minutes ago", ... for `then` as seen at `now` (Unix seconds).
pub fn relative_time(then: i64, now: i64, l: &Localizer) -> String {
    match (Timestamp::from_second(then), Timestamp::from_second(now)) {
        (Ok(then), Ok(now)) => l.relative(then, now),
        _ => String::new(),
    }
}

/// Summary of the thread rooted at `m`, if it has replies. `names` are the
/// display names of `m.reply_users` in order.
pub fn summary_for(
    m: &Message,
    names: Vec<String>,
    now: i64,
    l: &Localizer,
) -> Option<ThreadSummary> {
    if m.thread_root.is_some() || m.reply_count == 0 {
        return None;
    }
    let label = t!(l, "thread-replies", count = m.reply_count).into_owned();
    let latest = m
        .latest_reply
        .as_ref()
        .and_then(secs)
        .map(|t| relative_time(t, now, l))
        .unwrap_or_default();
    Some(ThreadSummary {
        label,
        authors: names.into_iter().take(MAX_AUTHORS).collect(),
        latest,
    })
}

/// One line: `"Bob, Cy  2 replies  5m ago"`.
pub fn summary_text(s: &ThreadSummary) -> String {
    let mut out = String::new();
    if !s.authors.is_empty() {
        out.push_str(&s.authors.join(", "));
        out.push_str("  ");
    }
    out.push_str(&s.label);
    if !s.latest.is_empty() {
        out.push_str("  ");
        out.push_str(&s.latest);
    }
    out
}
