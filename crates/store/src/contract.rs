//! Backend contract suite: every `Store` implementation runs each case.

use std::sync::Arc;

use domain::test_support::{cid, cmid, conv, msg, ts, uid};
use domain::{
    ComposedText, DraftKey, EmojiRef, FileId, FileKind, FileRef, Gap, Has, LocalSchedule,
    LocalScheduleState, Message, OutboxItem, OutboxState, PersistOp, ProviderExt, Reaction,
    ScheduleId, SearchQuery, Upload,
};
use settings::WorkspaceKey;

use crate::Store;

fn ws(id: &str) -> WorkspaceKey {
    WorkspaceKey::new("fake", id)
}

async fn put(s: &dyn Store, w: &WorkspaceKey, c: &str, messages: Vec<Message>) {
    s.apply(
        w,
        PersistOp::Messages {
            conversation: cid(c),
            messages,
        },
    )
    .await
    .unwrap();
}

/// `latest_all` reads the conversation list from the store's conversations.
async fn put_conv(s: &dyn Store, w: &WorkspaceKey, c: &str) {
    s.apply(w, PersistOp::Conversations(vec![conv(c, c)]))
        .await
        .unwrap();
}

fn reply(c: &str, t: &str, root: &str, text: &str, broadcast: bool) -> Message {
    let mut m = msg(c, t, text);
    m.thread_root = Some(ts(root));
    m.broadcast = broadcast;
    m
}

fn tss(v: &[Message]) -> Vec<String> {
    v.iter().map(|m| m.id.ts.0.to_string()).collect()
}

fn gap(a: &str, b: &str) -> Gap {
    Gap {
        conversation: cid("C1"),
        after: ts(a),
        before: ts(b),
    }
}

fn outbox_item(id: &str, created_ms: i64, text: &str) -> OutboxItem {
    OutboxItem {
        id: cmid(id),
        conversation: cid("C1"),
        thread: None,
        broadcast: false,
        text: Arc::from(text),
        created_ms,
        attempts: 0,
        state: OutboxState::Queued,
    }
}

pub async fn channel_window_hides_unbroadcast_replies(s: &dyn Store) {
    let w = ws("a");
    put(
        s,
        &w,
        "C1",
        vec![
            msg("C1", "1.000000", "root"),
            reply("C1", "2.000000", "1.000000", "quiet", false),
            reply("C1", "3.000000", "1.000000", "loud", true),
            msg("C1", "4.000000", "plain"),
        ],
    )
    .await;
    let got = s.messages_before(&w, &cid("C1"), None, 10).await.unwrap();
    assert_eq!(tss(&got), ["4.000000", "3.000000", "1.000000"]);
}

pub async fn thread_returns_root_then_replies_ascending(s: &dyn Store) {
    let w = ws("a");
    put(
        s,
        &w,
        "C1",
        vec![
            reply("C1", "4.000000", "1.000000", "r3", false),
            msg("C1", "1.000000", "root"),
            reply("C1", "3.000000", "1.000000", "r2", true),
            reply("C1", "2.000000", "1.000000", "r1", false),
            msg("C1", "5.000000", "other"),
            reply("C1", "6.000000", "5.000000", "elsewhere", false),
        ],
    )
    .await;
    let got = s.thread(&w, &cid("C1"), &ts("1.000000"), 10).await.unwrap();
    assert_eq!(tss(&got), ["1.000000", "2.000000", "3.000000", "4.000000"]);
    let got = s.thread(&w, &cid("C1"), &ts("1.000000"), 2).await.unwrap();
    assert_eq!(tss(&got), ["1.000000", "2.000000"]);
}

pub async fn replace_message_updates_in_place(s: &dyn Store) {
    let w = ws("a");
    put(s, &w, "C1", vec![msg("C1", "1.000000", "old")]).await;
    let mut n = msg("C1", "1.000000", "new");
    n.files = Arc::from([FileRef {
        id: FileId(Arc::from("F1")),
        name: Arc::from("a.png"),
        title: None,
        mime: Arc::from("image/png"),
        size: 3,
        kind: FileKind::Image,
        thumbs: Arc::from([]),
        download: None,
        preview: None,
        video: None,
        tombstone: false,
        lines: None,
        ext: ProviderExt::none(),
    }]);
    s.apply(&w, PersistOp::ReplaceMessage(n)).await.unwrap();
    let got = s.messages_before(&w, &cid("C1"), None, 10).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(&*got[0].text, "new");
    assert_eq!(got[0].id.ts, ts("1.000000"));
    assert_eq!(got[0].files.len(), 1);
}

pub async fn reactions_replace_whole_set(s: &dyn Store) {
    let w = ws("a");
    put(s, &w, "C1", vec![msg("C1", "1.000000", "x")]).await;
    let r = |code: &str, count: u32| Reaction {
        emoji: EmojiRef {
            shortcode: Arc::from(code),
            unicode: None,
            skin_tone: None,
        },
        count,
        users: Arc::from([uid("u1")]),
        me: false,
    };
    let id = domain::test_support::mid("C1", "1.000000");
    s.apply(
        &w,
        PersistOp::Reactions {
            id: id.clone(),
            reactions: Arc::from([r("a", 1), r("b", 2)]),
        },
    )
    .await
    .unwrap();
    s.apply(
        &w,
        PersistOp::Reactions {
            id,
            reactions: Arc::from([r("c", 3)]),
        },
    )
    .await
    .unwrap();
    let got = s.messages_before(&w, &cid("C1"), None, 10).await.unwrap();
    assert_eq!(got[0].reactions.len(), 1);
    assert_eq!(&*got[0].reactions[0].emoji.shortcode, "c");
    assert_eq!(got[0].reactions[0].count, 3);
}

pub async fn thread_summary_updates_root(s: &dyn Store) {
    let w = ws("a");
    put(s, &w, "C1", vec![msg("C1", "1.000000", "root")]).await;
    s.apply(
        &w,
        PersistOp::ThreadSummary {
            root: domain::test_support::mid("C1", "1.000000"),
            reply_count: 4,
            latest_reply: Some(ts("9.000000")),
        },
    )
    .await
    .unwrap();
    let got = s.messages_before(&w, &cid("C1"), None, 10).await.unwrap();
    assert_eq!(got[0].reply_count, 4);
    assert_eq!(got[0].latest_reply, Some(ts("9.000000")));
}

pub async fn gaps_add_and_close_split(s: &dyn Store) {
    let w = ws("a");
    s.apply(&w, PersistOp::AddGap(gap("1.000000", "10.000000")))
        .await
        .unwrap();
    assert_eq!(
        s.gaps(&w, &cid("C1")).await.unwrap(),
        [gap("1.000000", "10.000000")]
    );
    s.apply(&w, PersistOp::CloseGap(gap("3.000000", "5.000000")))
        .await
        .unwrap();
    assert_eq!(
        s.gaps(&w, &cid("C1")).await.unwrap(),
        [gap("1.000000", "3.000000"), gap("5.000000", "10.000000")]
    );
    assert!(s.gaps(&w, &cid("C2")).await.unwrap().is_empty());
}

pub async fn latest_all_lists_each_conversation(s: &dyn Store) {
    let w = ws("a");
    put_conv(s, &w, "C1").await;
    put_conv(s, &w, "C2").await;
    put(
        s,
        &w,
        "C1",
        vec![msg("C1", "1.000000", "a"), msg("C1", "7.000000", "b")],
    )
    .await;
    put(s, &w, "C2", vec![msg("C2", "3.000000", "c")]).await;
    let all = s.latest_all(&w).await.unwrap();
    assert_eq!(all.len(), 2);
    assert_eq!(all[&cid("C1")], ts("7.000000"));
    assert_eq!(all[&cid("C2")], ts("3.000000"));
}

pub async fn latest_all_ignores_unbroadcast_replies(s: &dyn Store) {
    let w = ws("a");
    put_conv(s, &w, "C1").await;
    put_conv(s, &w, "C2").await;
    put(
        s,
        &w,
        "C1",
        vec![
            msg("C1", "1.000000", "a"),
            reply("C1", "9.000000", "1.000000", "r", false),
        ],
    )
    .await;
    put(
        s,
        &w,
        "C2",
        vec![
            msg("C2", "1.000000", "a"),
            reply("C2", "9.000000", "1.000000", "r", true),
        ],
    )
    .await;
    let all = s.latest_all(&w).await.unwrap();
    assert_eq!(all[&cid("C1")], ts("1.000000"));
    assert_eq!(all[&cid("C2")], ts("9.000000"));
}

pub async fn outbox_round_trip_ordered(s: &dyn Store) {
    let w = ws("a");
    s.outbox_put(&w, outbox_item("c", 30, "c")).await.unwrap();
    s.outbox_put(&w, outbox_item("a", 10, "a")).await.unwrap();
    s.outbox_put(&w, outbox_item("b", 20, "b")).await.unwrap();
    let ids = |v: &[OutboxItem]| v.iter().map(|i| i.id.0.to_string()).collect::<Vec<_>>();
    assert_eq!(ids(&s.outbox(&w).await.unwrap()), ["a", "b", "c"]);
    let mut b2 = outbox_item("b", 20, "b2");
    b2.attempts = 2;
    b2.state = OutboxState::Failed("no".into());
    s.outbox_put(&w, b2.clone()).await.unwrap();
    let got = s.outbox(&w).await.unwrap();
    assert_eq!(got.len(), 3);
    assert_eq!(got[1], b2);
    s.outbox_remove(&w, &cmid("a")).await.unwrap();
    assert_eq!(ids(&s.outbox(&w).await.unwrap()), ["b", "c"]);
}

pub async fn drafts_put_and_blank_deletes(s: &dyn Store) {
    let w = ws("a");
    let chan = DraftKey {
        conversation: cid("C1"),
        thread: None,
    };
    let in_thread = DraftKey {
        conversation: cid("C1"),
        thread: Some(ts("1.000000")),
    };
    s.draft_put(&w, chan.clone(), Some(ComposedText::plain("hello")), 100)
        .await
        .unwrap();
    let picked = ComposedText::new(
        "ask @Ana now",
        vec![domain::ComposedSpan {
            range: 4..8,
            inline: domain::Inline::User {
                id: domain::UserId("U1".into()),
                label: Some("Ana".into()),
            },
        }],
    );
    s.draft_put(&w, in_thread.clone(), Some(picked.clone()), 200)
        .await
        .unwrap();
    assert_eq!(s.drafts(&w).await.unwrap().len(), 2);
    s.draft_put(&w, chan.clone(), Some(ComposedText::plain("  \n ")), 300)
        .await
        .unwrap();
    let left = s.drafts(&w).await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].0, in_thread);
    assert_eq!(left[0].1, picked, "spans survive the store");
    assert_eq!(left[0].2, 200, "the edit time survives the store");
    s.draft_put(&w, in_thread, None, 400).await.unwrap();
    assert!(s.drafts(&w).await.unwrap().is_empty());
}

pub async fn search_finds_plain_text(s: &dyn Store) {
    let w = ws("a");
    put(
        s,
        &w,
        "C1",
        vec![
            msg("C1", "1.000000", "deploy finished"),
            msg("C1", "2.000000", "lunch?"),
        ],
    )
    .await;
    let hits = s.search(&w, "deploy", 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id.ts, ts("1.000000"));
    assert!(s.search(&w, "nothing", 10).await.unwrap().is_empty());
}

pub async fn edit_updates_search_text(s: &dyn Store) {
    let w = ws("a");
    put(s, &w, "C1", vec![msg("C1", "1.000000", "deploy finished")]).await;
    s.apply(
        &w,
        PersistOp::EditMessage {
            id: domain::test_support::mid("C1", "1.000000"),
            text: Arc::from("lunch time"),
            plain: Arc::from("lunch time"),
            edited: ts("2.000000"),
        },
    )
    .await
    .unwrap();
    assert!(s.search(&w, "deploy", 10).await.unwrap().is_empty());
    let hits = s.search(&w, "lunch", 10).await.unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].id.ts, ts("1.000000"));
}

pub async fn message_lookup_by_ts(s: &dyn Store) {
    let w = ws("a");
    put(s, &w, "C1", vec![msg("C1", "1.000000", "hello")]).await;
    let m = s.message(&w, &cid("C1"), &ts("1.000000")).await.unwrap();
    assert_eq!(m.map(|m| m.text.to_string()), Some("hello".to_string()));
    assert!(
        s.message(&w, &cid("C1"), &ts("9.000000"))
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        s.message(&w, &cid("C2"), &ts("1.000000"))
            .await
            .unwrap()
            .is_none()
    );
}

/// `MessageTs` compares by numeric value, so two spellings of the same
/// instant are one message; every backend must store them as one row.
pub async fn equivalent_timestamps_are_one_message(s: &dyn Store) {
    let w = ws("a");
    put(s, &w, "C1", vec![msg("C1", "1.5", "first")]).await;
    put(s, &w, "C1", vec![msg("C1", "1.500000", "second")]).await;
    let got = s.messages_before(&w, &cid("C1"), None, 10).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].id.ts, ts("1.500000"));
    // `MessageTs` equality is numeric: pin the stored TEXT spelling too.
    assert_eq!(&*got[0].id.ts.0, "1.500000");
    assert_eq!(got[0].text.as_ref(), "second");
    assert!(
        s.message(&w, &cid("C1"), &ts("1.5"))
            .await
            .unwrap()
            .is_some()
    );
}

pub async fn drafts_keep_the_latest_edit_time(s: &dyn Store) {
    let w = ws("a");
    let key = DraftKey {
        conversation: cid("C1"),
        thread: None,
    };
    s.draft_put(&w, key.clone(), Some(ComposedText::plain("a")), 10)
        .await
        .unwrap();
    s.draft_put(&w, key.clone(), Some(ComposedText::plain("ab")), 20)
        .await
        .unwrap();
    let got = s.drafts(&w).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!((got[0].1.text.as_ref(), got[0].2), ("ab", 20));
}

pub async fn sidebar_collapsed_round_trip_and_expand_deletes(s: &dyn Store) {
    let w = ws("a");
    for (section, collapsed) in [("channels", true), ("starred", true), ("channels", true)] {
        s.sidebar_collapsed_put(&w, section.into(), collapsed)
            .await
            .unwrap();
    }
    let got = s.sidebar_collapsed(&w).await.unwrap();
    assert_eq!(got.len(), 2);
    assert!(got.contains("channels") && got.contains("starred"));
    s.sidebar_collapsed_put(&w, "channels".into(), false)
        .await
        .unwrap();
    let got = s.sidebar_collapsed(&w).await.unwrap();
    assert_eq!(got.into_iter().collect::<Vec<_>>(), ["starred"]);
    assert!(s.sidebar_collapsed(&ws("b")).await.unwrap().is_empty());
}

pub async fn wipe_removes_one_workspace_only(s: &dyn Store) {
    let a = ws("a");
    let b = ws("b");
    for w in [&a, &b] {
        put(s, w, "C1", vec![msg("C1", "1.000000", "x")]).await;
        s.outbox_put(w, outbox_item("o", 1, "o")).await.unwrap();
        s.draft_put(
            w,
            DraftKey {
                conversation: cid("C1"),
                thread: None,
            },
            Some(ComposedText::plain("d")),
            1,
        )
        .await
        .unwrap();
        s.apply(w, PersistOp::AddGap(gap("1.000000", "2.000000")))
            .await
            .unwrap();
    }
    for w in [&a, &b] {
        s.sidebar_collapsed_put(w, "channels".into(), true)
            .await
            .unwrap();
    }
    s.wipe(&a).await.unwrap();
    assert!(s.sidebar_collapsed(&a).await.unwrap().is_empty());
    assert_eq!(s.sidebar_collapsed(&b).await.unwrap().len(), 1);
    assert!(
        s.messages_before(&a, &cid("C1"), None, 10)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(s.outbox(&a).await.unwrap().is_empty());
    assert!(s.drafts(&a).await.unwrap().is_empty());
    assert!(s.gaps(&a, &cid("C1")).await.unwrap().is_empty());
    assert_eq!(
        s.messages_before(&b, &cid("C1"), None, 10)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(s.outbox(&b).await.unwrap().len(), 1);
    assert_eq!(s.drafts(&b).await.unwrap().len(), 1);
    assert_eq!(s.gaps(&b, &cid("C1")).await.unwrap().len(), 1);
}

fn local_schedule(id: &str, post_at: i64, thread: Option<&str>) -> LocalSchedule {
    LocalSchedule {
        id: ScheduleId(Arc::from(id)),
        conversation: cid("C1"),
        thread: thread.map(ts),
        text: Arc::from("later"),
        files: vec![Upload {
            path: "/tmp/a.txt".into(),
            name: Arc::from("a.txt"),
            mime: Arc::from("text/plain"),
            size: 3,
            filetype: None,
        }],
        post_at,
        created_ms: 7,
        state: LocalScheduleState::Pending,
    }
}

pub async fn local_schedules_round_trip_ordered(s: &dyn Store) {
    let w = ws("a");
    s.apply(
        &w,
        PersistOp::LocalSchedulePut(local_schedule("s2", 200, None)),
    )
    .await
    .unwrap();
    s.apply(
        &w,
        PersistOp::LocalSchedulePut(local_schedule("s1", 100, Some("1.000000"))),
    )
    .await
    .unwrap();
    let got = s.local_schedules(&w).await.unwrap();
    assert_eq!(
        got.iter().map(|l| &*l.id.0).collect::<Vec<_>>(),
        ["s1", "s2"]
    );
    assert_eq!(got[0].thread, Some(ts("1.000000")));
    assert_eq!(got[0].files.len(), 1);
    assert_eq!(got[0].files[0].name.as_ref(), "a.txt");
    assert_eq!(got[0].files[0].path, std::path::PathBuf::from("/tmp/a.txt"));
    assert_eq!(got[0].files[0].mime.as_ref(), "text/plain");
    assert_eq!(got[0].files[0].size, 3);
    assert_eq!(got[0].state, LocalScheduleState::Pending);
    // Put replaces by id.
    let mut failed = local_schedule("s1", 100, None);
    failed.state = LocalScheduleState::Failed("boom".into());
    s.apply(&w, PersistOp::LocalSchedulePut(failed))
        .await
        .unwrap();
    let got = s.local_schedules(&w).await.unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got[0].state, LocalScheduleState::Failed("boom".into()));
    s.apply(
        &w,
        PersistOp::LocalScheduleRemove(ScheduleId(Arc::from("s1"))),
    )
    .await
    .unwrap();
    assert_eq!(s.local_schedules(&w).await.unwrap().len(), 1);
    assert!(s.local_schedules(&ws("b")).await.unwrap().is_empty());
}

pub async fn local_notify_prefs_round_trip_and_default_deletes(s: &dyn Store) {
    let w = ws("a");
    let nothing = domain::ConversationPrefs {
        level: domain::NotifyLevel::Nothing,
        muted: false,
    };
    let muted = domain::ConversationPrefs {
        level: domain::NotifyLevel::Default,
        muted: true,
    };
    for (c, prefs) in [("c1", nothing), ("c2", muted)] {
        s.apply(
            &w,
            PersistOp::LocalNotifyPrefsPut {
                conversation: cid(c),
                prefs,
            },
        )
        .await
        .unwrap();
    }
    let got = s.local_notify_prefs(&w).await.unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got.get(&cid("c1")), Some(&nothing));
    assert_eq!(got.get(&cid("c2")), Some(&muted));
    // Put replaces; the default row deletes.
    s.apply(
        &w,
        PersistOp::LocalNotifyPrefsPut {
            conversation: cid("c1"),
            prefs: muted,
        },
    )
    .await
    .unwrap();
    s.apply(
        &w,
        PersistOp::LocalNotifyPrefsPut {
            conversation: cid("c2"),
            prefs: domain::ConversationPrefs::default(),
        },
    )
    .await
    .unwrap();
    let got = s.local_notify_prefs(&w).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got.get(&cid("c1")), Some(&muted));
    assert!(s.local_notify_prefs(&ws("b")).await.unwrap().is_empty());
    // A wipe clears them.
    s.wipe(&w).await.unwrap();
    assert!(s.local_notify_prefs(&w).await.unwrap().is_empty());
}

pub async fn local_notify_extras_round_trip_and_default_deletes(s: &dyn Store) {
    let w = ws("a");
    let timed = domain::NotifyExtras {
        threads_off: false,
        sound_off: true,
        mute_until: Some(1_700_003_600),
    };
    let quiet = domain::NotifyExtras {
        threads_off: true,
        ..Default::default()
    };
    for (c, extras) in [("c1", timed), ("c2", quiet)] {
        s.apply(
            &w,
            PersistOp::LocalNotifyExtrasPut {
                conversation: cid(c),
                extras,
            },
        )
        .await
        .unwrap();
    }
    let got = s.local_notify_extras(&w).await.unwrap();
    assert_eq!(got.len(), 2);
    assert_eq!(got.get(&cid("c1")), Some(&timed));
    assert_eq!(got.get(&cid("c2")), Some(&quiet));
    // Put replaces; the default row deletes.
    s.apply(
        &w,
        PersistOp::LocalNotifyExtrasPut {
            conversation: cid("c1"),
            extras: quiet,
        },
    )
    .await
    .unwrap();
    s.apply(
        &w,
        PersistOp::LocalNotifyExtrasPut {
            conversation: cid("c2"),
            extras: domain::NotifyExtras::default(),
        },
    )
    .await
    .unwrap();
    let got = s.local_notify_extras(&w).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(got.get(&cid("c1")), Some(&quiet));
    assert!(s.local_notify_extras(&ws("b")).await.unwrap().is_empty());
    s.wipe(&w).await.unwrap();
    assert!(s.local_notify_extras(&w).await.unwrap().is_empty());
}

pub async fn local_starred_round_trip_and_unstar_deletes(s: &dyn Store) {
    let w = ws("a");
    for (c, starred) in [("c1", true), ("c2", true), ("c1", true)] {
        s.apply(
            &w,
            PersistOp::LocalStarredPut {
                conversation: cid(c),
                starred,
            },
        )
        .await
        .unwrap();
    }
    let got = s.local_starred(&w).await.unwrap();
    assert_eq!(got.len(), 2);
    assert!(got.contains(&cid("c1")) && got.contains(&cid("c2")));
    s.apply(
        &w,
        PersistOp::LocalStarredPut {
            conversation: cid("c1"),
            starred: false,
        },
    )
    .await
    .unwrap();
    let got = s.local_starred(&w).await.unwrap();
    assert_eq!(got.len(), 1);
    assert!(got.contains(&cid("c2")));
    // Another workspace has its own stars.
    assert!(s.local_starred(&ws("b")).await.unwrap().is_empty());
    // A wipe clears them.
    s.wipe(&w).await.unwrap();
    assert!(s.local_starred(&w).await.unwrap().is_empty());
}

pub async fn messages_after_ascending_excludes_replies(s: &dyn Store) {
    let w = ws("a");
    put(
        s,
        &w,
        "C1",
        vec![
            msg("C1", "1.000000", "root"),
            reply("C1", "2.000000", "1.000000", "quiet", false),
            reply("C1", "3.000000", "1.000000", "loud", true),
            msg("C1", "4.000000", "four"),
            msg("C1", "5.000000", "five"),
        ],
    )
    .await;
    let got = s
        .messages_after(&w, &cid("C1"), &ts("1.000000"), 10)
        .await
        .unwrap();
    assert_eq!(tss(&got), ["3.000000", "4.000000", "5.000000"]);
    let got = s
        .messages_after(&w, &cid("C1"), &ts("1.000000"), 2)
        .await
        .unwrap();
    assert_eq!(tss(&got), ["3.000000", "4.000000"]);
    assert!(
        s.messages_after(&w, &cid("C1"), &ts("5.000000"), 10)
            .await
            .unwrap()
            .is_empty()
    );
}

fn emoji(code: &str) -> EmojiRef {
    EmojiRef {
        shortcode: Arc::from(code),
        unicode: Some(Arc::from("x")),
        skin_tone: Some(3),
    }
}

pub async fn recent_emoji_most_recent_first_capped(s: &dyn Store) {
    let w = ws("a");
    assert!(s.recent_emoji(&w, 10).await.unwrap().is_empty());
    for i in 0..40 {
        s.emoji_used(&w, emoji(&format!("e{i}")), 1000 + i)
            .await
            .unwrap();
    }
    let got = s.recent_emoji(&w, 100).await.unwrap();
    assert_eq!(got.len(), 36);
    assert_eq!(&*got[0].shortcode, "e39");
    assert_eq!(&*got[35].shortcode, "e4");
    assert_eq!(got[0].skin_tone, Some(3));
    assert_eq!(got[0].unicode.as_deref(), Some("x"));
    // Reusing an old one moves it to the front without growing the list.
    s.emoji_used(&w, emoji("e10"), 5000).await.unwrap();
    let got = s.recent_emoji(&w, 3).await.unwrap();
    assert_eq!(
        got.iter().map(|e| &*e.shortcode).collect::<Vec<_>>(),
        ["e10", "e39", "e38"]
    );
    assert_eq!(s.recent_emoji(&w, 100).await.unwrap().len(), 36);
    assert!(s.recent_emoji(&ws("b"), 10).await.unwrap().is_empty());
}

fn by(author: &str, mut m: Message) -> Message {
    m.author = Some(uid(author));
    m
}

fn q() -> SearchQuery {
    SearchQuery::default()
}

fn date(y: i16, m: i8, d: i8) -> jiff::civil::Date {
    jiff::civil::date(y, m, d)
}

// 2024-01-01 00:00:00 UTC is 1704067200.
const D1: &str = "1704100000.000000"; // 2024-01-01
const D2: &str = "1704200000.000000"; // 2024-01-02
const D3: &str = "1704300000.000000"; // 2024-01-03

pub async fn search_filtered_by_author_conversation_and_dates(s: &dyn Store) {
    let w = ws("a");
    put(
        s,
        &w,
        "C1",
        vec![
            by("u1", msg("C1", D1, "deploy one")),
            by("u2", msg("C1", D2, "deploy two")),
            by("u1", msg("C1", D3, "deploy three")),
        ],
    )
    .await;
    put(
        s,
        &w,
        "C2",
        vec![by("u1", msg("C2", D2, "deploy elsewhere"))],
    )
    .await;
    let ids = |hits: Vec<domain::SearchHit>| {
        hits.iter()
            .map(|h| format!("{}/{}", h.id.conversation.0, &h.id.ts.0[..10]))
            .collect::<Vec<_>>()
    };
    let mut f = q();
    f.text = Arc::from("deploy");
    f.authors = vec![uid("u1")];
    f.conversations = vec![cid("C1")];
    let got = ids(s.search_filtered(&w, &f, 10).await.unwrap());
    assert_eq!(got.len(), 2);
    assert!(got.contains(&"C1/1704100000".to_string()));
    assert!(got.contains(&"C1/1704300000".to_string()));
    // `before` and `after` are exclusive whole UTC days.
    let mut f = q();
    f.text = Arc::from("deploy");
    f.conversations = vec![cid("C1")];
    f.after = Some(date(2024, 1, 1));
    f.before = Some(date(2024, 1, 3));
    let got = ids(s.search_filtered(&w, &f, 10).await.unwrap());
    assert_eq!(got, ["C1/1704200000"]);
    let mut f = q();
    f.text = Arc::from("deploy");
    f.conversations = vec![cid("C1")];
    f.before = Some(date(2024, 1, 2));
    assert_eq!(
        ids(s.search_filtered(&w, &f, 10).await.unwrap()),
        ["C1/1704100000"]
    );
    // Other workspaces are not searched.
    assert!(
        s.search_filtered(&ws("b"), &f, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

pub async fn search_filtered_empty_text_lists_newest(s: &dyn Store) {
    let w = ws("a");
    put(
        s,
        &w,
        "C1",
        vec![
            by("u1", msg("C1", D1, "one")),
            by("u1", msg("C1", D3, "three")),
            by("u1", msg("C1", D2, "two")),
            by("u2", msg("C1", "1704400000.000000", "other")),
        ],
    )
    .await;
    let mut f = q();
    f.authors = vec![uid("u1")];
    let got = s.search_filtered(&w, &f, 2).await.unwrap();
    assert_eq!(
        got.iter().map(|h| &h.id.ts.0[..10]).collect::<Vec<_>>(),
        ["1704300000", "1704200000"]
    );
    assert!(!got[0].snippet.is_empty());
}

pub async fn search_filtered_has_file(s: &dyn Store) {
    let w = ws("a");
    let mut with_file = by("u1", msg("C1", D1, "see attached"));
    with_file.files = Arc::from([FileRef {
        id: FileId(Arc::from("F1")),
        name: Arc::from("a.png"),
        title: None,
        mime: Arc::from("image/png"),
        size: 1,
        kind: FileKind::Image,
        thumbs: Arc::from([]),
        download: None,
        preview: None,
        video: None,
        tombstone: false,
        lines: None,
        ext: ProviderExt::none(),
    }]);
    let mut with_reaction = by("u1", msg("C1", D2, "see reaction"));
    with_reaction.reactions = Arc::from([Reaction {
        emoji: emoji("tada"),
        count: 1,
        users: Arc::from([uid("u1")]),
        me: false,
    }]);
    put(
        s,
        &w,
        "C1",
        vec![
            with_file,
            with_reaction,
            by("u1", msg("C1", D3, "see nothing")),
        ],
    )
    .await;
    let mut f = q();
    f.text = Arc::from("see");
    f.has = vec![Has::File];
    let got = s.search_filtered(&w, &f, 10).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(&got[0].id.ts.0[..10], "1704100000");
    f.has = vec![Has::Reaction];
    let got = s.search_filtered(&w, &f, 10).await.unwrap();
    assert_eq!(got.len(), 1);
    assert_eq!(&got[0].id.ts.0[..10], "1704200000");
    f.has = vec![Has::File, Has::Reaction];
    assert!(s.search_filtered(&w, &f, 10).await.unwrap().is_empty());
}

pub async fn wipe_clears_local_schedules_and_recent_emoji(s: &dyn Store) {
    let (a, b) = (ws("a"), ws("b"));
    for w in [&a, &b] {
        s.apply(
            w,
            PersistOp::LocalSchedulePut(local_schedule("s1", 1, None)),
        )
        .await
        .unwrap();
        s.emoji_used(w, emoji("tada"), 1).await.unwrap();
    }
    s.wipe(&a).await.unwrap();
    assert!(s.local_schedules(&a).await.unwrap().is_empty());
    assert!(s.recent_emoji(&a, 10).await.unwrap().is_empty());
    assert_eq!(s.local_schedules(&b).await.unwrap().len(), 1);
    assert_eq!(s.recent_emoji(&b, 10).await.unwrap().len(), 1);
}

pub async fn local_reminders_round_trip_and_wipe(s: &dyn Store) {
    let w = ws("a");
    let mk = |id: &str, due: Option<i64>, recurring: bool, completed: bool| domain::Reminder {
        id: domain::ReminderId(Arc::from(id)),
        text: Arc::from(format!("text {id}")),
        due,
        recurring,
        completed,
        fired: false,
        interval_secs: if recurring { 86_400 } else { 0 },
        for_user: domain::UserId(Arc::from("me")),
        link: None,
        snoozed_until: None,
    };
    let linked = domain::ReminderLink {
        message: domain::MessageId {
            conversation: domain::ConversationId(Arc::from("c1")),
            ts: domain::MessageTs(Arc::from("7.000000")),
        },
        author: Some(domain::UserId(Arc::from("u2"))),
        excerpt: Arc::from("check the contrast"),
    };
    for r in [
        domain::Reminder {
            link: Some(linked.clone()),
            ..mk("r2", Some(200), false, false)
        },
        domain::Reminder {
            snoozed_until: Some(150),
            ..mk("r1", Some(100), true, false)
        },
        mk("r3", None, false, true),
    ] {
        s.apply(&w, PersistOp::LocalReminderPut(r)).await.unwrap();
    }
    // Ordered by due (none last), then id.
    let got = s.local_reminders(&w).await.unwrap();
    let ids: Vec<&str> = got.iter().map(|r| &*r.id.0).collect();
    assert_eq!(ids, ["r1", "r2", "r3"]);
    assert_eq!(got[1].link.as_ref(), Some(&linked));
    assert!(got[0].link.is_none());
    assert_eq!(got[0].snoozed_until, Some(150));
    assert_eq!(got[1].snoozed_until, None);
    assert!(
        got[0].recurring
            && got[0].interval_secs == 86_400
            && got[2].completed
            && got[2].due.is_none()
    );
    // Put replaces; remove deletes; other workspaces are separate.
    s.apply(
        &w,
        PersistOp::LocalReminderPut(domain::Reminder {
            fired: true,
            ..mk("r2", Some(200), false, true)
        }),
    )
    .await
    .unwrap();
    s.apply(
        &w,
        PersistOp::LocalReminderRemove(domain::ReminderId(Arc::from("r1"))),
    )
    .await
    .unwrap();
    let got = s.local_reminders(&w).await.unwrap();
    assert_eq!(got.len(), 2);
    assert!(got[0].completed && got[0].fired && !got[1].fired);
    assert!(s.local_reminders(&ws("b")).await.unwrap().is_empty());
    s.wipe(&w).await.unwrap();
    assert!(s.local_reminders(&w).await.unwrap().is_empty());
}

pub async fn user_groups_replace_whole_set_per_workspace_and_wipe(s: &dyn Store) {
    let g = |id: &str, handle: &str, n: u32, mine: bool| domain::UserGroup {
        id: id.into(),
        handle: handle.into(),
        name: format!("{handle} team").into(),
        member_count: n,
        is_member: mine,
    };
    let (a, b) = (ws("a"), ws("b"));
    s.apply(
        &a,
        PersistOp::UserGroups(vec![g("S2", "eng", 3, true), g("S1", "design", 9, false)]),
    )
    .await
    .unwrap();
    s.apply(&b, PersistOp::UserGroups(vec![g("S9", "ops", 1, false)]))
        .await
        .unwrap();
    let got = s.user_groups(&a).await.unwrap();
    assert_eq!(
        got,
        vec![g("S1", "design", 9, false), g("S2", "eng", 3, true)],
        "ordered by handle"
    );
    s.apply(&a, PersistOp::UserGroups(vec![g("S3", "qa", 2, false)]))
        .await
        .unwrap();
    assert_eq!(
        s.user_groups(&a).await.unwrap(),
        vec![g("S3", "qa", 2, false)]
    );
    s.wipe(&a).await.unwrap();
    assert!(s.user_groups(&a).await.unwrap().is_empty());
    assert_eq!(s.user_groups(&b).await.unwrap().len(), 1);
}

fn shared(id: &str, name: &str, tombstone: bool) -> FileRef {
    FileRef {
        id: FileId(Arc::from(id)),
        name: Arc::from(name),
        title: None,
        mime: Arc::from("application/octet-stream"),
        size: 10,
        kind: FileKind::Other,
        thumbs: Arc::from([]),
        download: None,
        preview: None,
        video: None,
        tombstone,
        lines: None,
        ext: ProviderExt::none(),
    }
}

fn with_files(mut m: Message, files: Vec<FileRef>) -> Message {
    m.files = Arc::from(files);
    m
}

fn file_ids(p: &domain::Page<domain::FileHit>) -> Vec<String> {
    p.items.iter().map(|h| h.file.id.0.to_string()).collect()
}

/// The Files tab's query: newest first, thread replies included, deleted files
/// and other conversations left out, each hit naming its sharer and message.
pub async fn conversation_files_newest_first(s: &dyn Store) {
    let w = ws("a");
    put(
        s,
        &w,
        "C1",
        vec![
            with_files(
                by("u1", msg("C1", "1.000001", "two files")),
                vec![shared("A", "a.txt", false), shared("B", "b.txt", false)],
            ),
            by("u1", msg("C1", "2.000001", "no file")),
            with_files(
                by("u2", msg("C1", "3.000001", "gone")),
                vec![shared("X", "x.txt", true)],
            ),
            with_files(
                by(
                    "u3",
                    reply("C1", "4.000001", "1.000001", "in a thread", false),
                ),
                vec![shared("C", "c.txt", false)],
            ),
        ],
    )
    .await;
    put(
        s,
        &w,
        "C2",
        vec![with_files(
            msg("C2", "5.000001", "elsewhere"),
            vec![shared("D", "d.txt", false)],
        )],
    )
    .await;
    let p = s
        .conversation_files(&w, &cid("C1"), None, 10)
        .await
        .unwrap();
    assert_eq!(file_ids(&p), ["C", "A", "B"]);
    assert_eq!(p.next, None, "everything fit in one page");
    let c = &p.items[0];
    assert_eq!(c.author, Some(uid("u3")));
    assert_eq!(c.ts, Some(ts("4.000001")));
    assert_eq!(c.conversation, Some(cid("C1")));
    assert!(
        s.conversation_files(&ws("b"), &cid("C1"), None, 10)
            .await
            .unwrap()
            .items
            .is_empty()
    );
}

/// Pages follow each other without gaps or repeats, and a page never holds
/// more than `limit` messages' files nor more than [`crate::FILES_PAGE_MAX`].
pub async fn conversation_files_pages_are_bounded(s: &dyn Store) {
    let w = ws("a");
    let messages: Vec<Message> = (1..=7)
        .map(|i| {
            with_files(
                msg("C1", &format!("{i}.000001"), "f"),
                vec![shared(&format!("F{i}"), "f.txt", false)],
            )
        })
        .collect();
    put(s, &w, "C1", messages).await;
    let c = cid("C1");
    let first = s.conversation_files(&w, &c, None, 3).await.unwrap();
    assert_eq!(file_ids(&first), ["F7", "F6", "F5"]);
    let next = first.next.clone().expect("more pages");
    let second = s
        .conversation_files(&w, &c, Some(domain::MessageTs(next)), 3)
        .await
        .unwrap();
    assert_eq!(file_ids(&second), ["F4", "F3", "F2"]);
    let third = s
        .conversation_files(&w, &c, second.next.clone().map(domain::MessageTs), 3)
        .await
        .unwrap();
    assert_eq!(file_ids(&third), ["F1"]);
    assert_eq!(third.next, None);
    // A message keeps all its files together; the page stops after it.
    put(
        s,
        &w,
        "C1",
        vec![with_files(
            msg("C1", "8.000001", "three"),
            vec![
                shared("G1", "g.txt", false),
                shared("G2", "g.txt", false),
                shared("G3", "g.txt", false),
            ],
        )],
    )
    .await;
    let p = s.conversation_files(&w, &c, None, 2).await.unwrap();
    assert_eq!(file_ids(&p), ["G1", "G2", "G3"]);
    assert!(p.next.is_some());
    // An oversized limit is clamped.
    let many: Vec<Message> = (0..crate::FILES_PAGE_MAX + 5)
        .map(|i| {
            with_files(
                msg("C9", &format!("{}.000001", 100 + i), "f"),
                vec![shared(&format!("M{i}"), "f.txt", false)],
            )
        })
        .collect();
    put(s, &w, "C9", many).await;
    let p = s
        .conversation_files(&w, &cid("C9"), None, usize::MAX)
        .await
        .unwrap();
    assert_eq!(p.items.len(), crate::FILES_PAGE_MAX);
    assert!(p.next.is_some());
}

/// Two messages whose timestamps share the same sort key (unparsable ones,
/// such as a pending local id, all sort first) straddling a page boundary:
/// the next page starts with the other one.
pub async fn conversation_files_ties_never_skip(s: &dyn Store) {
    let w = ws("a");
    put(
        s,
        &w,
        "C1",
        vec![
            with_files(
                msg("C1", "pending-a", "a"),
                vec![shared("A", "a.txt", false)],
            ),
            with_files(
                msg("C1", "pending-b", "b"),
                vec![shared("B", "b.txt", false)],
            ),
            with_files(
                msg("C1", "6.000001", "c"),
                vec![shared("C", "c.txt", false)],
            ),
        ],
    )
    .await;
    let c = cid("C1");
    let mut seen = Vec::new();
    let mut cursor = None;
    for _ in 0..5 {
        let p = s.conversation_files(&w, &c, cursor, 1).await.unwrap();
        seen.extend(file_ids(&p));
        match p.next {
            Some(n) => cursor = Some(domain::MessageTs(n)),
            None => break,
        }
    }
    assert_eq!(seen, ["C", "B", "A"]);
}
