use std::sync::Arc;

use domain::test_support::{cid, conv, msg, ts, uid};
use domain::{
    Bootstrap, Input, Mention, ProviderEvent, WorkspaceRef, WorkspaceState, all_mentions,
    all_scheduled, all_unreads, reduce, unread_previews,
};
use settings::WorkspaceKey;

fn ws(id: &str, convs: Vec<domain::Conversation>) -> (WorkspaceKey, WorkspaceState) {
    let key = WorkspaceKey::new("fake", id);
    let mut s = WorkspaceState::new(key.clone(), 200);
    reduce(
        &mut s,
        Input::Bootstrapped(Bootstrap {
            self_user: uid("me"),
            workspace_name: Arc::from("W"),
            conversations: convs,
            members: vec![],
        }),
    );
    (key, s)
}
fn refs(v: &[(WorkspaceKey, WorkspaceState)]) -> Vec<WorkspaceRef<'_>> {
    v.iter()
        .map(|(k, s)| WorkspaceRef { key: k, state: s })
        .collect()
}
fn unread(id: &str, name: &str, latest: &str, mentions: u32, muted: bool) -> domain::Conversation {
    let mut c = conv(id, name);
    c.unread = true;
    c.latest = Some(ts(latest));
    c.mentions = mentions;
    c.is_muted = muted;
    c
}

#[test]
fn all_unreads_mentions_first_then_latest() {
    let a = ws(
        "a",
        vec![
            unread("C1", "old", "1.000001", 0, false),
            unread("C2", "new", "9.000001", 0, false),
            unread("C3", "meant", "2.000001", 2, false),
            conv("C4", "read"),
        ],
    );
    let b = ws("b", vec![unread("C5", "other", "5.000001", 0, false)]);
    let v = [a, b];
    let rows = all_unreads(refs(&v));
    let names: Vec<&str> = rows.iter().map(|r| &*r.name).collect();
    assert_eq!(names, ["meant", "new", "other", "old"]);
    assert_eq!(rows[2].workspace, v[1].0);
}

#[test]
fn muted_only_with_mentions() {
    let a = ws(
        "a",
        vec![
            unread("C1", "quiet", "1.000001", 0, true),
            unread("C2", "loud", "2.000001", 1, true),
        ],
    );
    let v = [a];
    let rows = all_unreads(refs(&v));
    assert_eq!(rows.len(), 1);
    assert_eq!(&*rows[0].name, "loud");
}

#[test]
fn all_mentions_lists_held_mentions_newest_first() {
    let mut c = conv("C1", "general");
    c.last_read = Some(ts("1.000001"));
    let (k, mut s) = ws("a", vec![c, conv("C2", "none")]);
    reduce(&mut s, Input::Open(cid("C1")));
    for (t, mine) in [
        ("1.000001", true),
        ("2.000001", true),
        ("3.000001", false),
        ("4.000001", true),
    ] {
        let mut m = msg("C1", t, "x");
        m.author = Some(uid("other"));
        if mine {
            m.mentions = Arc::from([Mention::User(uid("me"))]);
        }
        reduce(&mut s, Input::Provider(ProviderEvent::MessagePosted(m)));
    }
    let v = [(k, s)];
    let out = all_mentions(refs(&v));
    assert_eq!(out.len(), 1);
    let ids: Vec<String> = out[0].1.iter().map(|m| m.ts.0.to_string()).collect();
    assert_eq!(ids, ["4.000001", "2.000001"]);
}

#[test]
fn all_scheduled_merges_workspaces_by_time_and_tags_local() {
    use domain::{LocalSchedule, LocalScheduleState, ScheduleId, ScheduledMessage};
    let (ka, mut a) = ws("a", vec![conv("C1", "x")]);
    let (kb, mut b) = ws("b", vec![conv("C2", "y")]);
    a.scheduled.push(ScheduledMessage {
        id: ScheduleId(Arc::from("S1")),
        conversation: cid("C1"),
        thread: None,
        text: Arc::from("late"),
        post_at: 3000,
    });
    b.scheduled.push(ScheduledMessage {
        id: ScheduleId(Arc::from("S2")),
        conversation: cid("C2"),
        thread: None,
        text: Arc::from("early"),
        post_at: 1000,
    });
    b.local_schedules.push(LocalSchedule {
        id: ScheduleId(Arc::from("L1")),
        conversation: cid("C2"),
        thread: None,
        text: Arc::from("middle"),
        files: vec![],
        post_at: 2000,
        created_ms: 0,
        state: LocalScheduleState::Pending,
    });
    let v = [(ka, a), (kb.clone(), b)];
    let out = all_scheduled(refs(&v));
    let texts: Vec<&str> = out.iter().map(|r| &*r.text).collect();
    assert_eq!(texts, ["early", "middle", "late"]);
    assert_eq!(out[0].workspace, kb);
    assert_eq!(
        out.iter().map(|r| r.local).collect::<Vec<_>>(),
        [false, true, false]
    );
}

#[test]
fn mark_read_input_clears_unread_and_asks_the_provider() {
    let (_, mut s) = ws("a", vec![unread("C1", "x", "5.000001", 2, false)]);
    let fx = reduce(
        &mut s,
        Input::MarkRead {
            conversation: cid("C1"),
            ts: ts("5.000001"),
        },
    );
    let c = &s.conversations[&cid("C1")];
    assert!(!c.unread);
    assert_eq!(c.mentions, 0);
    assert_eq!(c.last_read, Some(ts("5.000001")));
    assert!(fx.iter().any(|e| matches!(
        e,
        domain::Effect::MarkRead { conversation, .. } if *conversation == cid("C1")
    )));
}

#[test]
fn all_mentions_skips_broadcasts_in_a_muted_conversation() {
    let mut c = conv("C1", "general");
    c.is_muted = true;
    c.last_read = Some(ts("1.000001"));
    let (k, mut s) = ws("a", vec![c]);
    reduce(&mut s, Input::Open(cid("C1")));
    let mut here = msg("C1", "2.000001", "x");
    here.author = Some(uid("other"));
    here.mentions = Arc::from([Mention::Broadcast(domain::Broadcast::Here)]);
    let mut direct = msg("C1", "3.000001", "x");
    direct.author = Some(uid("other"));
    direct.mentions = Arc::from([Mention::User(uid("me"))]);
    for m in [here, direct] {
        reduce(&mut s, Input::Provider(ProviderEvent::MessagePosted(m)));
    }
    let v = [(k, s)];
    let out = all_mentions(refs(&v));
    let ids: Vec<String> = out[0].1.iter().map(|m| m.ts.0.to_string()).collect();
    assert_eq!(ids, ["3.000001"]);
}

#[test]
fn unread_previews_are_the_newest_held_after_last_read_oldest_first() {
    let mut c = unread("C1", "x", "5.000001", 0, false);
    c.last_read = Some(ts("1.000001"));
    let (_, mut s) = ws("a", vec![c, unread("C2", "y", "1.000001", 0, false)]);
    reduce(&mut s, Input::Open(cid("C1")));
    for t in ["1.000001", "2.000001", "3.000001", "4.000001", "5.000001"] {
        let mut m = msg("C1", t, "x");
        m.author = Some(uid("other"));
        reduce(&mut s, Input::Provider(ProviderEvent::MessagePosted(m)));
    }
    // Opening marks the conversation read; set the marker back.
    if let Some(c) = s.conversations.get_mut(&cid("C1")) {
        c.last_read = Some(ts("1.000001"));
    }
    let ids: Vec<String> = unread_previews(&s, &cid("C1"), 3)
        .iter()
        .map(|m| m.id.ts.0.to_string())
        .collect();
    assert_eq!(ids, ["3.000001", "4.000001", "5.000001"]);
    let two: Vec<String> = unread_previews(&s, &cid("C1"), 10)
        .iter()
        .map(|m| m.id.ts.0.to_string())
        .collect();
    assert_eq!(two.len(), 4, "only messages after last_read");
    assert!(
        unread_previews(&s, &cid("C2"), 3).is_empty(),
        "no held window"
    );
}

#[test]
fn unread_previews_skip_own_and_unsent_and_keep_mentions_of_muted() {
    let mut c = unread("C1", "x", "6.000001", 1, true);
    c.last_read = Some(ts("1.000001"));
    let (_, mut s) = ws("a", vec![c]);
    reduce(&mut s, Input::Open(cid("C1")));
    for (t, who, mention) in [
        ("2.000001", "other", false),
        ("3.000001", "other", true),
        ("4.000001", "me", false),
        ("5.000001", "other", false),
    ] {
        let mut m = msg("C1", t, "x");
        m.author = Some(uid(who));
        if mention {
            m.mentions = Arc::from([Mention::User(uid("me"))]);
        }
        reduce(&mut s, Input::Provider(ProviderEvent::MessagePosted(m)));
    }
    if let Some(c) = s.conversations.get_mut(&cid("C1")) {
        c.last_read = Some(ts("1.000001"));
    }
    let ids = |s: &WorkspaceState| -> Vec<String> {
        unread_previews(s, &cid("C1"), 3)
            .iter()
            .map(|m| m.id.ts.0.to_string())
            .collect()
    };
    assert_eq!(
        ids(&s),
        ["3.000001"],
        "a muted row previews its mention only"
    );
    if let Some(c) = s.conversations.get_mut(&cid("C1")) {
        c.is_muted = false;
    }
    assert_eq!(
        ids(&s),
        ["2.000001", "3.000001", "5.000001"],
        "never my own messages"
    );
    let mut pending = msg("C1", "7.000001", "sending");
    pending.author = Some(uid("other"));
    pending.state = domain::DeliveryState::Pending;
    reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessagePosted(pending)),
    );
    if let Some(c) = s.conversations.get_mut(&cid("C1")) {
        c.last_read = Some(ts("1.000001"));
    }
    assert!(
        !ids(&s).contains(&"7.000001".to_string()),
        "never unsent ones"
    );
}
