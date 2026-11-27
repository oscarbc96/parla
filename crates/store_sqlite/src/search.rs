//! FTS5 query building and snippets.

use std::sync::Arc;

use domain::{ConversationId, Has, MessageId, MessageTs, SearchHit, SearchQuery};
use rusqlite::Connection;
use rusqlite::types::Value;
use store::StoreError;

use crate::err::other as db;

struct Term {
    text: String,
    prefix: bool,
    phrase: bool,
}

fn parse(user: &str) -> Vec<Term> {
    let mut out = Vec::new();
    let mut chars = user.chars().peekable();
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '"' {
            chars.next();
            let mut s = String::new();
            for c in chars.by_ref() {
                if c == '"' {
                    break;
                }
                s.push(c);
            }
            if !s.trim().is_empty() {
                out.push(Term {
                    text: s,
                    prefix: false,
                    phrase: true,
                });
            }
        } else {
            let mut s = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() {
                    break;
                }
                s.push(c);
                chars.next();
            }
            let prefix = s.ends_with('*');
            if prefix {
                s.pop();
            }
            if !s.is_empty() {
                out.push(Term {
                    text: s,
                    prefix,
                    phrase: false,
                });
            }
        }
    }
    out
}

/// Turns user input into a safe FTS5 query: every term is a quoted string
/// (`"` doubled), a trailing `*` keeps prefix search, a quoted phrase stays
/// one phrase. `None` when there is nothing to search for.
pub fn fts_query(user: &str) -> Option<String> {
    let terms = parse(user);
    if terms.is_empty() {
        return None;
    }
    let parts: Vec<String> = terms
        .iter()
        .map(|t| {
            let q = t.text.replace('"', "\"\"");
            if t.prefix {
                format!("\"{q}\"*")
            } else {
                format!("\"{q}\"")
            }
        })
        .collect();
    Some(parts.join(" "))
}

fn norm(w: &str) -> String {
    w.chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

/// The 12 words of `plain` around the first word matching a query term.
pub(crate) fn snippet(plain: &str, user: &str) -> String {
    const WORDS: usize = 12;
    let targets: Vec<(String, bool)> = parse(user)
        .into_iter()
        .filter_map(|t| {
            let first = if t.phrase {
                t.text
                    .split_whitespace()
                    .map(norm)
                    .find(|w| !w.is_empty())?
            } else {
                norm(&t.text)
            };
            (!first.is_empty()).then_some((first, t.prefix))
        })
        .collect();
    let words: Vec<&str> = plain.split_whitespace().collect();
    let at = words
        .iter()
        .position(|w| {
            let n = norm(w);
            targets
                .iter()
                .any(|(t, prefix)| n == *t || (*prefix && n.starts_with(t.as_str())))
        })
        .unwrap_or(0);
    let start = at.saturating_sub(WORDS / 2);
    words[start..(start + WORDS).min(words.len())].join(" ")
}

pub(crate) fn search_filtered(
    c: &Connection,
    q: &SearchQuery,
    limit: usize,
) -> Result<Vec<SearchHit>, StoreError> {
    let fts = crate::search::fts_query(&q.text);
    let mut args: Vec<Value> = Vec::new();
    let mut wh: Vec<String> = Vec::new();
    let push = |args: &mut Vec<Value>, v: Value| {
        args.push(v);
        format!("?{}", args.len())
    };
    if let Some(f) = &fts {
        let p = push(&mut args, Value::Text(f.clone()));
        wh.push(format!("messages_fts MATCH {p}"));
    }
    if !q.conversations.is_empty() {
        let ps: Vec<String> = q
            .conversations
            .iter()
            .map(|c| push(&mut args, Value::Text(c.0.to_string())))
            .collect();
        wh.push(format!("m.conversation_id IN ({})", ps.join(",")));
    }
    if !q.authors.is_empty() {
        let ps: Vec<String> = q
            .authors
            .iter()
            .map(|a| push(&mut args, Value::Text(a.0.to_string())))
            .collect();
        wh.push(format!("m.author IN ({})", ps.join(",")));
    }
    if let Some(d) = q.before {
        let p = push(
            &mut args,
            Value::Integer(store::day_start_secs(d).saturating_mul(1_000_000)),
        );
        wh.push(format!("m.ts_key < {p}"));
    }
    if let Some(d) = q.after {
        let next = store::day_start_secs(d).saturating_add(86_400);
        let p = push(&mut args, Value::Integer(next.saturating_mul(1_000_000)));
        wh.push(format!("m.ts_key >= {p}"));
    }
    for h in &q.has {
        match h {
            Has::File => wh.push("(m.files IS NOT NULL AND length(m.files) > 0)".into()),
            Has::Reaction => {
                wh.push("(m.reactions IS NOT NULL AND length(m.reactions) > 0)".into());
            }
            // Not stored locally.
            Has::Link | Has::Pin => {}
        }
    }
    let lim = push(
        &mut args,
        Value::Integer(i64::try_from(limit).unwrap_or(i64::MAX)),
    );
    let cond = if wh.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", wh.join(" AND "))
    };
    let sql = if fts.is_some() {
        format!(
            "SELECT m.conversation_id, m.ts, m.plain, bm25(messages_fts) AS r \
             FROM messages_fts \
             JOIN messages_fts_map f ON f.rowid = messages_fts.rowid \
             JOIN messages m ON m.conversation_id = f.conversation_id AND m.ts = f.ts \
             {cond} ORDER BY r, m.ts_key DESC LIMIT {lim}"
        )
    } else {
        format!(
            "SELECT m.conversation_id, m.ts, m.plain, 0.0 AS r FROM messages m \
             {cond} ORDER BY m.ts_key DESC, m.ts DESC LIMIT {lim}"
        )
    };
    let mut st = c.prepare(&sql).map_err(db)?;
    let mut rows = st.query(rusqlite::params_from_iter(args)).map_err(db)?;
    let mut out = Vec::new();
    while let Some(r) = rows.next().map_err(db)? {
        let plain: String = r.get(2).map_err(db)?;
        out.push(SearchHit {
            id: MessageId {
                conversation: ConversationId(Arc::from(r.get::<_, String>(0).map_err(db)?)),
                ts: MessageTs(Arc::from(r.get::<_, String>(1).map_err(db)?)),
            },
            snippet: Arc::from(crate::search::snippet(&plain, &q.text)),
            rank: -r.get::<_, f64>(3).map_err(db)?,
        });
    }
    Ok(out)
}
