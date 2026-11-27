//! Search query parsing, name resolution and hit merging.

use std::sync::Arc;

use domain::test_support::{cid, conv, member, msg, uid};
use domain::{
    ConversationKind, Has, HitSource, MessageHit, WorkspaceState, local_supported, merge_hits,
    parse_query, resolve_query,
};
use jiff::civil::date;
use settings::WorkspaceKey;

fn hit(c: &str, t: &str, source: HitSource) -> MessageHit {
    MessageHit {
        message: msg(c, t, "x"),
        snippet: Arc::from(""),
        source,
    }
}

fn ids(v: &[MessageHit]) -> Vec<(String, String)> {
    v.iter()
        .map(|h| {
            (
                h.message.id.conversation.0.to_string(),
                h.message.id.ts.0.to_string(),
            )
        })
        .collect()
}

#[test]
fn parses_all_operators() {
    let p = parse_query("deploy in:#ops from:@ana after:2026-01-02 has:file \"rolled back\"");
    assert_eq!(p.text, "deploy \"rolled back\"");
    assert_eq!(p.in_names, ["ops"]);
    assert_eq!(p.from_names, ["ana"]);
    assert_eq!(p.after, Some(date(2026, 1, 2)));
    assert_eq!(p.before, None);
    assert_eq!(p.has, [Has::File]);
    assert!(p.warnings.is_empty());
    let q = parse_query("in:@bob from:me has:link has:reaction has:pin");
    assert_eq!(q.in_names, ["@bob"]);
    assert_eq!(q.from_names, ["me"]);
    assert_eq!(q.has, [Has::Link, Has::Reaction, Has::Pin]);
}

#[test]
fn on_and_during_set_bounds() {
    // Bounds are exclusive, like before: and after:.
    let p = parse_query("on:2026-03-01");
    assert_eq!(p.after, Some(date(2026, 2, 28)));
    assert_eq!(p.before, Some(date(2026, 3, 2)));
    let p = parse_query("during:2026-02");
    assert_eq!(p.after, Some(date(2026, 1, 31)));
    assert_eq!(p.before, Some(date(2026, 3, 1)));
    let p = parse_query("during:2025-12");
    assert_eq!(p.after, Some(date(2025, 11, 30)));
    assert_eq!(p.before, Some(date(2026, 1, 1)));
    assert_eq!(p.text, "");
}

#[test]
fn malformed_operator_stays_text_with_warning() {
    let p = parse_query("before:yesterday hello");
    assert_eq!(p.text, "before:yesterday hello");
    assert_eq!(p.before, None);
    assert_eq!(p.warnings, ["before:yesterday"]);
    let p = parse_query("has:banana in:");
    assert_eq!(p.text, "has:banana in:");
    assert_eq!(p.warnings.len(), 2);
    let p = parse_query("foo:bar http://example.com 12:30");
    assert_eq!(p.text, "foo:bar http://example.com 12:30");
    assert_eq!(p.warnings, ["foo:bar"]);
}

fn state() -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    s.self_user = Some(uid("me"));
    s.conversations.insert(cid("c1"), conv("c1", "Ops"));
    let mut dm = conv("d1", "ana");
    dm.kind = ConversationKind::Direct;
    dm.peer = Some(uid("u1"));
    s.conversations.insert(cid("d1"), dm);
    for m in [
        member("me", "Me Self"),
        member("u1", "Ana"),
        member("u2", "Bob"),
    ] {
        s.members.insert(m.id.clone(), m);
    }
    s
}

#[test]
fn resolve_names_case_insensitive_and_me() {
    let s = state();
    let p = parse_query("hi in:#ops in:@ANA from:me from:@bob");
    let (q, warnings) = resolve_query(&p, &s);
    assert!(warnings.is_empty());
    assert_eq!(&*q.text, "hi");
    assert_eq!(q.conversations, [cid("c1"), cid("d1")]);
    assert_eq!(q.authors, [uid("me"), uid("u2")]);
}

#[test]
fn unknown_name_is_warning_not_error() {
    let s = state();
    let p = parse_query("in:#nope from:@ghost from:@ana");
    let (q, warnings) = resolve_query(&p, &s);
    assert_eq!(warnings, ["in:#nope", "from:@ghost"]);
    assert!(q.conversations.is_empty());
    assert_eq!(q.authors, [uid("u1")]);
}

#[test]
fn parser_never_fails_on_corpus() {
    for line in include_str!("fixtures/search_corpus.txt").lines() {
        let _ = parse_query(line);
    }
    let junk = ":\"#@".repeat(2_600);
    assert!(junk.len() >= 10_000);
    let _ = parse_query(&junk);
    let _ = parse_query(&format!("in:{junk} from:{junk} has:{junk}"));
}

#[test]
fn merge_dedupes_by_id() {
    let local = vec![
        hit("c1", "5.000000", HitSource::Local),
        hit("c1", "2.000000", HitSource::Local),
    ];
    let remote = vec![
        hit("c1", "2.000000", HitSource::Remote),
        hit("c1", "9.000000", HitSource::Remote),
        hit("c2", "7.000000", HitSource::Remote),
        hit("c2", "7.000000", HitSource::Remote),
    ];
    let m = merge_hits(local, remote);
    assert_eq!(
        ids(&m),
        [
            ("c1".into(), "5.000000".into()),
            ("c1".into(), "2.000000".into()),
            ("c1".into(), "9.000000".into()),
            ("c2".into(), "7.000000".into()),
        ]
    );
    assert_eq!(m[0].source, HitSource::Local);
    assert_eq!(m[2].source, HitSource::Remote);
}

#[test]
fn link_and_pin_are_remote_only() {
    let mut q = domain::SearchQuery::default();
    assert!(local_supported(&q));
    q.has = vec![Has::File, Has::Reaction];
    assert!(local_supported(&q));
    q.has.push(Has::Link);
    assert!(!local_supported(&q));
    q.has = vec![Has::Pin];
    assert!(!local_supported(&q));
}
