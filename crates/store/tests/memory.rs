use std::sync::Arc;

use domain::{
    Conversation, ConversationId, ConversationKind, Message, MessageId, MessageTs, PersistOp,
    ProviderExt,
};
use futures::executor::block_on;
use settings::WorkspaceKey;
use store::{MemoryStore, Store};

fn cid() -> ConversationId {
    ConversationId("C1".into())
}
fn ts(n: u32) -> MessageTs {
    MessageTs(format!("{n}.000000").into())
}
fn msg(c: &ConversationId, n: u32, text: &str) -> Message {
    Message::new(
        MessageId {
            conversation: c.clone(),
            ts: ts(n),
        },
        None,
        Arc::from(text),
    )
}
fn put(s: &MemoryStore, ws: &WorkspaceKey, c: &ConversationId, msgs: Vec<Message>) {
    block_on(s.apply(
        ws,
        PersistOp::Messages {
            conversation: c.clone(),
            messages: msgs,
        },
    ))
    .unwrap();
}
fn ws(id: &str) -> WorkspaceKey {
    WorkspaceKey::new("fake", id)
}

#[test]
fn messages_before_is_newest_first_window() {
    let s = MemoryStore::default();
    let w = ws("a");
    put(
        &s,
        &w,
        &cid(),
        (1..=10).map(|n| msg(&cid(), n, "x")).collect(),
    );
    let got = block_on(s.messages_before(&w, &cid(), Some(ts(5)), 3)).unwrap();
    let got: Vec<_> = got.iter().map(|m| m.id.ts.clone()).collect();
    assert_eq!(got, vec![ts(4), ts(3), ts(2)]);
    let newest = block_on(s.messages_before(&w, &cid(), None, 2)).unwrap();
    assert_eq!(newest[0].id.ts, ts(10));
}

#[test]
fn upsert_same_ts_replaces() {
    let s = MemoryStore::default();
    let w = ws("a");
    put(&s, &w, &cid(), vec![msg(&cid(), 1, "old")]);
    put(&s, &w, &cid(), vec![msg(&cid(), 1, "new")]);
    let got = block_on(s.messages_before(&w, &cid(), None, 10)).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(&*got[0].text, "new");
}

#[test]
fn edit_and_delete_remove() {
    let s = MemoryStore::default();
    let w = ws("a");
    put(
        &s,
        &w,
        &cid(),
        vec![msg(&cid(), 1, "a"), msg(&cid(), 2, "b")],
    );
    block_on(s.apply(
        &w,
        PersistOp::EditMessage {
            id: msg(&cid(), 1, "").id,
            text: "edited".into(),
            plain: "edited".into(),
            edited: ts(3),
        },
    ))
    .unwrap();
    block_on(s.apply(&w, PersistOp::DeleteMessage(msg(&cid(), 2, "").id))).unwrap();
    let got = block_on(s.messages_before(&w, &cid(), None, 10)).unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(&*got[0].text, "edited");
    assert_eq!(got[0].edited, Some(ts(3)));
}

#[test]
fn teams_are_isolated() {
    let s = MemoryStore::default();
    put(&s, &ws("a"), &cid(), vec![msg(&cid(), 1, "a")]);
    assert!(
        block_on(s.messages_before(&ws("b"), &cid(), None, 10))
            .unwrap()
            .is_empty()
    );
    assert_eq!(block_on(s.latest_ts(&ws("b"), &cid())).unwrap(), None);
}

#[test]
fn latest_ts_tracks_max() {
    let s = MemoryStore::default();
    let w = ws("a");
    put(
        &s,
        &w,
        &cid(),
        vec![msg(&cid(), 5, "a"), msg(&cid(), 2, "b")],
    );
    assert_eq!(block_on(s.latest_ts(&w, &cid())).unwrap(), Some(ts(5)));
    block_on(s.apply(&w, PersistOp::DeleteMessage(msg(&cid(), 5, "").id))).unwrap();
    assert_eq!(block_on(s.latest_ts(&w, &cid())).unwrap(), Some(ts(2)));
}

#[test]
fn read_marker_updates_conversation() {
    let s = MemoryStore::default();
    let w = ws("a");
    let c = Conversation {
        id: cid(),
        kind: ConversationKind::Channel,
        name: "g".into(),
        peer: None,
        is_member: true,
        is_archived: false,
        is_open: true,
        is_muted: false,
        last_read: None,
        latest: None,
        unread: true,
        mentions: 0,
        participants: Default::default(),
        member_ids: Default::default(),
        topic: Default::default(),
        member_count: None,
        purpose: Default::default(),
        ext: ProviderExt::none(),
    };
    block_on(s.apply(&w, PersistOp::Conversations(vec![c]))).unwrap();
    block_on(s.apply(
        &w,
        PersistOp::ReadMarker {
            conversation: cid(),
            ts: ts(4),
        },
    ))
    .unwrap();
    let cs = block_on(s.conversations(&w)).unwrap();
    assert_eq!(cs[0].last_read, Some(ts(4)));
}

/// Outbox ties and drafts come back in the order the SQLite store returns.
#[test]
fn outbox_ties_and_drafts_are_ordered_like_sqlite() {
    use domain::{ClientMsgId, ComposedText, DraftKey, OutboxItem, OutboxState};
    let (s, w) = (MemoryStore::default(), ws("w"));
    for id in ["c", "a", "b", "d", "e", "f"] {
        let item = OutboxItem {
            id: ClientMsgId(id.into()),
            conversation: cid(),
            thread: None,
            broadcast: false,
            text: id.into(),
            created_ms: 5,
            attempts: 0,
            state: OutboxState::Queued,
        };
        block_on(s.outbox_put(&w, item)).unwrap();
    }
    let ids: Vec<_> = block_on(s.outbox(&w))
        .unwrap()
        .into_iter()
        .map(|i| i.id.0.to_string())
        .collect();
    assert_eq!(ids, ["a", "b", "c", "d", "e", "f"]);

    let key = |c: &str, t: Option<u32>| DraftKey {
        conversation: ConversationId(c.into()),
        thread: t.map(ts),
    };
    for k in [
        key("B", None),
        key("A", Some(2)),
        key("A", None),
        key("A", Some(1)),
    ] {
        block_on(s.draft_put(&w, k, Some(ComposedText::plain("x")), 0)).unwrap();
    }
    let order: Vec<_> = block_on(s.drafts(&w))
        .unwrap()
        .into_iter()
        .map(|(k, _, _)| {
            (
                k.conversation.0.to_string(),
                k.thread.map(|t| t.0.to_string()),
            )
        })
        .collect();
    assert_eq!(order[0], ("A".to_string(), None));
    assert_eq!(order[3], ("B".to_string(), None));
    assert!(order[1].1 < order[2].1);
}
