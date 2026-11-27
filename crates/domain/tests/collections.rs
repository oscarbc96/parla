//! pins, bookmarks, saved items and the Threads inbox.

use std::sync::Arc;

use domain::test_support::{cid, conv, mid, msg, ts, uid};
use domain::{
    Bookmark, BookmarkId, Bootstrap, CallOutput, Capabilities, Changed, Effect, HistoryPage,
    InboxThread, Input, Load, Loaded, MessageId, Page, Pin, ProviderCall, ProviderError,
    ProviderEvent, SavedItem, WorkspaceState, reduce,
};
use settings::WorkspaceKey;

fn state() -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    s.now_unix = 1_700_000_000;
    reduce(
        &mut s,
        Input::Bootstrapped(Bootstrap {
            self_user: uid("me"),
            workspace_name: Arc::from("W"),
            conversations: vec![conv("c1", "general")],
            members: vec![],
        }),
    );
    s
}

fn hold(s: &mut WorkspaceState, ids: &[&str]) {
    let messages = ids.iter().map(|t| msg("c1", t, "hi")).collect();
    reduce(
        s,
        Input::HistoryLoaded {
            conversation: cid("c1"),
            page: HistoryPage {
                messages,
                has_more: false,
            },
        },
    );
}

fn held(s: &WorkspaceState, t: &str) -> domain::Message {
    s.windows[&cid("c1")]
        .messages()
        .iter()
        .find(|m| m.id == mid("c1", t))
        .cloned()
        .expect("held")
}

fn pin(t: &str) -> Pin {
    Pin {
        message: mid("c1", t),
        by: None,
        at: 5,
    }
}

fn item(t: &str) -> SavedItem {
    SavedItem {
        message: mid("c1", t),
        saved_at: 5,
        due: None,
        done: false,
    }
}

fn thread(t: &str) -> InboxThread {
    InboxThread {
        root: mid("c1", t),
        unread_replies: 1,
        mentions: 0,
        latest: ts(t),
    }
}

fn bookmark(id: &str) -> Bookmark {
    Bookmark {
        id: BookmarkId(Arc::from(id)),
        conversation: cid("c1"),
        title: Arc::from(id),
        link: Arc::from("https://example.com/x"),
        emoji: None,
    }
}

fn loads(fx: &[Effect]) -> Vec<Load> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Load(l) => Some(l.clone()),
            _ => None,
        })
        .collect()
}

fn calls(fx: &[Effect]) -> Vec<ProviderCall> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Call(c) => Some(c.clone()),
            _ => None,
        })
        .collect()
}

fn has(fx: &[Effect], c: Changed) -> bool {
    fx.iter()
        .any(|e| matches!(e, Effect::Changed(x) if *x == c))
}

fn finish(s: &mut WorkspaceState, call: ProviderCall, ok: bool) -> Vec<Effect> {
    reduce(
        s,
        Input::CallFinished {
            call,
            result: if ok {
                Ok(CallOutput::None)
            } else {
                Err(ProviderError::Network("x".into()))
            },
        },
    )
}

fn set_pinned(id: MessageId, pinned: bool) -> Input {
    Input::SetPinned { id, pinned }
}

#[test]
fn open_pins_loads_and_fetches_missing_messages() {
    let mut s = state();
    hold(&mut s, &["1.1"]);
    let fx = reduce(&mut s, Input::OpenPins(cid("c1")));
    assert!(matches!(loads(&fx)[..], [Load::Pins(_)]));
    let fx = reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::Pins(cid("c1"), vec![pin("1.1"), pin("2.2")]))),
    );
    assert!(has(&fx, Changed::Pins(cid("c1"))));
    // The held message is not fetched again; the missing one is, once.
    let l = loads(&fx);
    assert_eq!(l.len(), 1);
    assert!(matches!(&l[0], Load::Message(m) if *m == mid("c1", "2.2")));
    assert_eq!(s.pins[&cid("c1")].len(), 2);
    // Open again: already loaded, nothing to do.
    assert!(loads(&reduce(&mut s, Input::OpenPins(cid("c1")))).is_empty());
    // The answer lands in `referenced`.
    reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::Message(msg("c1", "2.2", "old")))),
    );
    assert!(s.referenced.contains_key(&mid("c1", "2.2")));
    // An identical snapshot is not a change.
    let fx = reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::Pins(cid("c1"), vec![pin("1.1"), pin("2.2")]))),
    );
    assert!(!has(&fx, Changed::Pins(cid("c1"))));
    assert!(loads(&fx).is_empty());
}

#[test]
fn missing_message_fetches_are_capped_per_page() {
    let mut s = state();
    let pins: Vec<Pin> = (0..80).map(|i| pin(&format!("{}.1", i + 10))).collect();
    let fx = reduce(&mut s, Input::Loaded(Ok(Loaded::Pins(cid("c1"), pins))));
    assert_eq!(loads(&fx).len(), 50);
}

#[test]
fn pin_toggle_optimistic_and_rollback() {
    let mut s = state();
    hold(&mut s, &["1.1"]);
    reduce(&mut s, Input::Loaded(Ok(Loaded::Pins(cid("c1"), vec![]))));
    let id = mid("c1", "1.1");
    let fx = reduce(&mut s, set_pinned(id.clone(), true));
    assert!(held(&s, "1.1").pinned);
    assert_eq!(s.pins[&cid("c1")].len(), 1);
    assert!(has(&fx, Changed::Pins(cid("c1"))));
    assert!(has(&fx, Changed::Timeline(cid("c1"))));
    assert!(matches!(
        calls(&fx)[..],
        [ProviderCall::SetPinned { pinned: true, .. }]
    ));
    // The provider refuses: the truth is refetched, the flag is re-read.
    let fx = finish(
        &mut s,
        ProviderCall::SetPinned {
            id: id.clone(),
            pinned: true,
        },
        false,
    );
    let l = loads(&fx);
    assert!(l.iter().any(|x| matches!(x, Load::Pins(_))));
    assert!(l.iter().any(|x| matches!(x, Load::Message(m) if *m == id)));
    // The refetched truth says: not pinned.
    reduce(&mut s, Input::Loaded(Ok(Loaded::Pins(cid("c1"), vec![]))));
    let mut truth = msg("c1", "1.1", "hi");
    truth.pinned = false;
    reduce(&mut s, Input::Loaded(Ok(Loaded::Message(truth))));
    assert!(!held(&s, "1.1").pinned);
    assert!(s.pins[&cid("c1")].is_empty());
}

#[test]
fn concurrent_pin_writes_refetch_once_when_last_answer_arrives() {
    let mut s = state();
    hold(&mut s, &["1.1"]);
    let id = mid("c1", "1.1");
    let on = ProviderCall::SetPinned {
        id: id.clone(),
        pinned: true,
    };
    let off = ProviderCall::SetPinned {
        id: id.clone(),
        pinned: false,
    };
    reduce(&mut s, Input::Loaded(Ok(Loaded::Pins(cid("c1"), vec![]))));
    reduce(&mut s, set_pinned(id.clone(), true));
    reduce(&mut s, set_pinned(id, false));
    // First answer fails, one still in flight: no refetch yet.
    assert!(loads(&finish(&mut s, on, false)).is_empty());
    // Last answer arrives: refetch once.
    let fx = finish(&mut s, off, true);
    assert!(loads(&fx).iter().any(|l| matches!(l, Load::Pins(_))));
}

#[test]
fn pin_echo_is_noop() {
    let mut s = state();
    hold(&mut s, &["1.1"]);
    reduce(&mut s, Input::Loaded(Ok(Loaded::Pins(cid("c1"), vec![]))));
    let id = mid("c1", "1.1");
    reduce(&mut s, set_pinned(id.clone(), true));
    finish(
        &mut s,
        ProviderCall::SetPinned {
            id: id.clone(),
            pinned: true,
        },
        true,
    );
    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::PinChanged {
            id: id.clone(),
            pinned: true,
        }),
    );
    assert!(fx.is_empty(), "{fx:?}");
    // A real change from elsewhere applies.
    let fx = reduce(
        &mut s,
        Input::Provider(ProviderEvent::PinChanged { id, pinned: false }),
    );
    assert!(!held(&s, "1.1").pinned);
    assert!(has(&fx, Changed::Pins(cid("c1"))));
    assert!(has(&fx, Changed::Timeline(cid("c1"))));
}

#[test]
fn save_toggle_optimistic_and_rollback() {
    let mut s = state();
    hold(&mut s, &["1.1"]);
    reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::Saved(Page {
            items: vec![],
            next: None,
        }))),
    );
    let id = mid("c1", "1.1");
    let fx = reduce(
        &mut s,
        Input::SetSaved {
            id: id.clone(),
            saved: true,
        },
    );
    assert!(held(&s, "1.1").saved);
    assert_eq!(s.saved.as_ref().unwrap().items.len(), 1);
    assert!(has(&fx, Changed::Saved));
    let fx = finish(&mut s, ProviderCall::SetSaved { id, saved: true }, false);
    assert!(
        loads(&fx)
            .iter()
            .any(|l| matches!(l, Load::Saved { cursor: None }))
    );
}

#[test]
fn saved_paging_appends() {
    let mut s = state();
    let fx = reduce(&mut s, Input::OpenSaved);
    assert!(matches!(loads(&fx)[..], [Load::Saved { cursor: None }]));
    reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::Saved(Page {
            items: vec![item("1.1")],
            next: Some(Arc::from("c2")),
        }))),
    );
    let fx = reduce(&mut s, Input::MoreSaved);
    match &loads(&fx)[..] {
        [Load::Saved { cursor: Some(c) }] => assert_eq!(&**c, "c2"),
        other => panic!("{other:?}"),
    }
    // A second request while one is in flight is ignored.
    assert!(loads(&reduce(&mut s, Input::MoreSaved)).is_empty());
    let fx = reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::Saved(Page {
            items: vec![item("1.1"), item("2.2")],
            next: None,
        }))),
    );
    assert!(has(&fx, Changed::Saved));
    let p = s.saved.as_ref().unwrap();
    assert_eq!(p.items.len(), 2, "duplicates are not appended twice");
    assert!(p.next.is_none());
    // No cursor left: nothing to load.
    assert!(loads(&reduce(&mut s, Input::MoreSaved)).is_empty());
}

#[test]
fn bookmarks_load_on_open() {
    let mut s = state();
    s.caps = Capabilities {
        bookmarks: true,
        ..Default::default()
    };
    let fx = reduce(&mut s, Input::Open(cid("c1")));
    assert!(loads(&fx).iter().any(|l| matches!(l, Load::Bookmarks(_))));
    reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::Bookmarks(cid("c1"), vec![bookmark("b1")]))),
    );
    // Loaded: opening again does not reload.
    let fx = reduce(&mut s, Input::Open(cid("c1")));
    assert!(!loads(&fx).iter().any(|l| matches!(l, Load::Bookmarks(_))));
    // Add: the provider's answer is inserted.
    let fx = reduce(
        &mut s,
        Input::AddBookmark {
            conversation: cid("c1"),
            title: Arc::from("t"),
            link: Arc::from("https://example.com/y"),
        },
    );
    let call = calls(&fx).remove(0);
    let fx = reduce(
        &mut s,
        Input::CallFinished {
            call,
            result: Ok(CallOutput::Bookmark(bookmark("b2"))),
        },
    );
    assert!(has(&fx, Changed::Bookmarks(cid("c1"))));
    assert_eq!(s.bookmarks[&cid("c1")].len(), 2);
    // Remove is optimistic; a failure refetches.
    let fx = reduce(
        &mut s,
        Input::RemoveBookmark {
            conversation: cid("c1"),
            id: BookmarkId(Arc::from("b1")),
        },
    );
    assert_eq!(s.bookmarks[&cid("c1")].len(), 1);
    assert!(has(&fx, Changed::Bookmarks(cid("c1"))));
    let call = calls(&fx).remove(0);
    let fx = finish(&mut s, call, false);
    assert!(loads(&fx).iter().any(|l| matches!(l, Load::Bookmarks(_))));
}

#[test]
fn inbox_refreshes_on_thread_read_only_when_loaded() {
    let mut s = state();
    let marker = ProviderEvent::ThreadReadMarker {
        root: mid("c1", "1.1"),
        ts: ts("1.2"),
    };
    // Not loaded: nothing.
    assert!(loads(&reduce(&mut s, Input::Provider(marker.clone()))).is_empty());
    assert!(
        loads(&reduce(
            &mut s,
            Input::Provider(ProviderEvent::ThreadInboxChanged)
        ))
        .is_empty()
    );
    let fx = reduce(&mut s, Input::OpenThreadInbox);
    assert!(matches!(
        loads(&fx)[..],
        [Load::ThreadInbox { cursor: None }]
    ));
    reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::ThreadInbox(Page {
            items: vec![thread("1.1")],
            next: None,
        }))),
    );
    let fx = reduce(&mut s, Input::Provider(marker.clone()));
    assert!(matches!(
        loads(&fx)[..],
        [Load::ThreadInbox { cursor: None }]
    ));
    // A burst while one refresh is in flight coalesces into one more.
    assert!(loads(&reduce(&mut s, Input::Provider(marker))).is_empty());
    assert!(
        loads(&reduce(
            &mut s,
            Input::Provider(ProviderEvent::ThreadInboxChanged)
        ))
        .is_empty()
    );
    let fx = reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::ThreadInbox(Page {
            items: vec![thread("1.1"), thread("2.2")],
            next: None,
        }))),
    );
    assert!(has(&fx, Changed::ThreadInbox));
    assert!(matches!(
        loads(&fx)
            .iter()
            .filter(|l| matches!(l, Load::ThreadInbox { .. }))
            .count(),
        1
    ));
}

#[test]
fn referenced_is_bounded_and_cleared_when_lean() {
    // Pins whose messages arrive one by one.
    let mut s = state();
    for i in 0..520u32 {
        let t = format!("{}.1", i + 10);
        reduce(
            &mut s,
            Input::Loaded(Ok(Loaded::Pins(cid("c1"), vec![pin(&t)]))),
        );
        reduce(
            &mut s,
            Input::Loaded(Ok(Loaded::Message(msg("c1", &t, "x")))),
        );
    }
    assert_eq!(s.referenced.len(), 500);
    // The oldest were dropped first.
    assert!(!s.referenced.contains_key(&mid("c1", "10.1")));
    assert!(s.referenced.contains_key(&mid("c1", "529.1")));
    reduce(&mut s, Input::SetLean(true));
    assert!(s.referenced.is_empty());
    // An answer that lands while lean is not kept.
    reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::Message(msg("c1", "9.1", "x")))),
    );
    assert!(s.referenced.is_empty());
}

// ---- review fixes ---------------------------------------------------------

fn pins_loaded(s: &mut WorkspaceState, ts: &[&str]) -> Vec<Effect> {
    let list = ts.iter().map(|t| pin(t)).collect();
    reduce(s, Input::Loaded(Ok(Loaded::Pins(cid("c1"), list))))
}

fn saved_loaded(s: &mut WorkspaceState, ts: &[&str], next: Option<&str>) -> Vec<Effect> {
    reduce(
        s,
        Input::Loaded(Ok(Loaded::Saved(Page {
            items: ts.iter().map(|t| item(t)).collect(),
            next: next.map(Arc::from),
        }))),
    )
}

fn bookmarks_loaded(s: &mut WorkspaceState, ids: &[&str]) -> Vec<Effect> {
    let list = ids.iter().map(|i| bookmark(i)).collect();
    reduce(s, Input::Loaded(Ok(Loaded::Bookmarks(cid("c1"), list))))
}

fn msg_load_count(fx: &[Effect]) -> usize {
    loads(fx)
        .iter()
        .filter(|l| matches!(l, Load::Message(_)))
        .count()
}

#[test]
fn lean_sends_no_message_loads_and_wake_fetches_missing() {
    let mut s = state();
    pins_loaded(&mut s, &["1.1"]);
    saved_loaded(&mut s, &["2.2"], None);
    reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::ThreadInbox(Page {
            items: vec![thread("3.3")],
            next: None,
        }))),
    );
    // Answers land, then the workspace goes lean.
    reduce(&mut s, Input::SetLean(true));
    assert!(s.referenced.is_empty());
    // Lists keep loading while lean, but no message is asked for.
    let fx = pins_loaded(&mut s, &["1.1", "4.4"]);
    assert_eq!(msg_load_count(&fx), 0);
    let fx = saved_loaded(&mut s, &["2.2", "5.5"], None);
    assert_eq!(msg_load_count(&fx), 0);
    // Leaving lean asks for what the loaded lists show and lack.
    let fx = reduce(&mut s, Input::SetLean(false));
    let ids: Vec<_> = loads(&fx)
        .into_iter()
        .filter_map(|l| match l {
            Load::Message(m) => Some(m),
            _ => None,
        })
        .collect();
    for t in ["1.1", "4.4", "2.2", "5.5", "3.3"] {
        assert!(ids.contains(&mid("c1", t)), "{t} in {ids:?}");
    }
    assert_eq!(ids.len(), 5);
}

#[test]
fn add_bookmark_without_answer_reloads_only_that_conversation() {
    let mut s = state();
    // c1 is loaded, c2 is not; only the call's conversation reloads.
    bookmarks_loaded(&mut s, &["b1"]);
    let call = |c: &str| ProviderCall::AddBookmark {
        conversation: cid(c),
        title: Arc::from("t"),
        link: Arc::from("https://example.com/z"),
    };
    let fx = reduce(
        &mut s,
        Input::CallFinished {
            call: call("c1"),
            result: Ok(CallOutput::None),
        },
    );
    assert!(matches!(&loads(&fx)[..], [Load::Bookmarks(c)] if *c == cid("c1")));
    // An unloaded conversation is not opened as a side effect.
    let fx = reduce(
        &mut s,
        Input::CallFinished {
            call: call("c2"),
            result: Ok(CallOutput::None),
        },
    );
    assert!(loads(&fx).is_empty());
}

#[test]
fn message_load_that_changes_nothing_emits_nothing() {
    let mut s = state();
    pins_loaded(&mut s, &["1.1"]);
    saved_loaded(&mut s, &["1.1"], None);
    let answer = |s: &mut WorkspaceState| {
        // Ask again by reloading the list after clearing the copy.
        reduce(s, Input::Loaded(Ok(Loaded::Message(msg("c1", "1.1", "x")))))
    };
    // The first answer is a change (the copy appears).
    let fx = answer(&mut s);
    assert!(has(&fx, Changed::Pins(cid("c1"))));
    assert!(has(&fx, Changed::Saved));
    // An unasked or identical answer repaints nothing.
    let fx = answer(&mut s);
    assert!(fx.is_empty(), "{fx:?}");
    // A refetch of an identical copy repaints nothing either.
    let mut id_fx = reduce(&mut s, Input::Provider(ProviderEvent::SavedChanged));
    id_fx.extend(saved_loaded(&mut s, &["1.1"], None));
    assert!(msg_load_count(&id_fx) == 0);
    assert!(!has(&id_fx, Changed::Saved));
}

#[test]
fn new_copy_keeps_the_optimistic_flag_while_a_write_is_in_flight() {
    let mut s = state();
    pins_loaded(&mut s, &["1.1"]);
    // Ask for the message, then toggle before the answer.
    // (The pin list asked for 1.1 above.)
    reduce(&mut s, set_pinned(mid("c1", "1.1"), false));
    let mut truth = msg("c1", "1.1", "x");
    truth.pinned = true;
    reduce(&mut s, Input::Loaded(Ok(Loaded::Message(truth))));
    // The list no longer shows it, so nothing is kept for it.
    assert!(s.referenced.is_empty());
    // Saved: the optimistic value wins over the provider's old copy.
    saved_loaded(&mut s, &["2.2"], None);
    reduce(
        &mut s,
        Input::SetSaved {
            id: mid("c1", "2.2"),
            saved: true,
        },
    );
    let mut truth = msg("c1", "2.2", "x");
    truth.saved = false;
    reduce(&mut s, Input::Loaded(Ok(Loaded::Message(truth))));
    assert!(s.referenced[&mid("c1", "2.2")].saved);
}

#[test]
fn referenced_copy_is_dropped_when_the_message_loads_into_a_window() {
    let mut s = state();
    pins_loaded(&mut s, &["1.1"]);
    reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::Message(msg("c1", "1.1", "x")))),
    );
    assert!(s.referenced.contains_key(&mid("c1", "1.1")));
    reduce(
        &mut s,
        Input::HistoryLoaded {
            conversation: cid("c1"),
            page: HistoryPage {
                messages: vec![msg("c1", "1.1", "x")],
                has_more: false,
            },
        },
    );
    assert!(s.referenced.is_empty());
    // And a message answer for a held message is not kept either.
    pins_loaded(&mut s, &["1.1", "1.1"]);
    reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::Message(msg("c1", "1.1", "x")))),
    );
    assert!(s.referenced.is_empty());
}

#[test]
fn failed_writes_refetch_only_loaded_lists() {
    let mut s = state();
    let id = mid("c1", "1.1");
    hold(&mut s, &["1.1"]);
    reduce(&mut s, set_pinned(id.clone(), true));
    reduce(
        &mut s,
        Input::SetSaved {
            id: id.clone(),
            saved: true,
        },
    );
    let fx = finish(
        &mut s,
        ProviderCall::SetPinned {
            id: id.clone(),
            pinned: true,
        },
        false,
    );
    assert!(!loads(&fx).iter().any(|l| matches!(l, Load::Pins(_))));
    let fx = finish(&mut s, ProviderCall::SetSaved { id, saved: true }, false);
    assert!(!loads(&fx).iter().any(|l| matches!(l, Load::Saved { .. })));
    let fx = finish(
        &mut s,
        ProviderCall::RemoveBookmark {
            conversation: cid("c1"),
            id: BookmarkId(Arc::from("b1")),
        },
        false,
    );
    assert!(loads(&fx).is_empty());
}

#[test]
fn saved_answers_during_writes_are_dropped_and_read_again() {
    let mut s = state();
    // First page: dropped while a save is in flight, re-read after.
    let fx = reduce(&mut s, Input::OpenSaved);
    assert_eq!(loads(&fx).len(), 1);
    let id = mid("c1", "1.1");
    reduce(
        &mut s,
        Input::SetSaved {
            id: id.clone(),
            saved: true,
        },
    );
    let fx = saved_loaded(&mut s, &["9.9"], None);
    assert!(s.saved.is_none(), "dropped");
    assert!(loads(&fx).is_empty(), "blocked by the write");
    let fx = finish(&mut s, ProviderCall::SetSaved { id, saved: true }, true);
    assert!(matches!(
        loads(&fx).iter().find(|l| matches!(l, Load::Saved { .. })),
        Some(Load::Saved { cursor: None })
    ));
    saved_loaded(&mut s, &["1.1"], Some("c2"));
    // Next page: dropped while saves are in flight, then the first page again.
    let fx = reduce(&mut s, Input::MoreSaved);
    assert_eq!(loads(&fx).len(), 1);
    let id2 = mid("c1", "2.2");
    reduce(
        &mut s,
        Input::SetSaved {
            id: id2.clone(),
            saved: true,
        },
    );
    saved_loaded(&mut s, &["7.7"], None);
    assert!(
        !s.saved
            .as_ref()
            .unwrap()
            .items
            .iter()
            .any(|i| i.message == mid("c1", "7.7"))
    );
    let fx = finish(
        &mut s,
        ProviderCall::SetSaved {
            id: id2,
            saved: true,
        },
        true,
    );
    assert!(
        loads(&fx)
            .iter()
            .any(|l| matches!(l, Load::Saved { cursor: None }))
    );
}

#[test]
fn pins_answer_during_write_is_dropped_and_read_again() {
    let mut s = state();
    reduce(&mut s, Input::OpenPins(cid("c1")));
    let id = mid("c1", "1.1");
    reduce(&mut s, set_pinned(id.clone(), true));
    let fx = pins_loaded(&mut s, &[]);
    assert!(!s.pins.contains_key(&cid("c1")), "dropped");
    assert!(loads(&fx).is_empty());
    let fx = finish(&mut s, ProviderCall::SetPinned { id, pinned: true }, true);
    assert!(matches!(&loads(&fx)[..], [Load::Pins(_)]));
    let fx = pins_loaded(&mut s, &["1.1"]);
    assert!(has(&fx, Changed::Pins(cid("c1"))));
}

#[test]
fn pins_and_bookmarks_loads_coalesce() {
    let mut s = state();
    pins_loaded(&mut s, &[]);
    bookmarks_loaded(&mut s, &[]);
    s.caps.bookmarks = true;
    // Bookmarks: three events while one load is out give one more load.
    let ev = || Input::Provider(ProviderEvent::BookmarksChanged(cid("c1")));
    assert_eq!(loads(&reduce(&mut s, ev())).len(), 1);
    assert!(loads(&reduce(&mut s, ev())).is_empty());
    assert!(loads(&reduce(&mut s, ev())).is_empty());
    let fx = bookmarks_loaded(&mut s, &["b1"]);
    assert_eq!(
        loads(&fx)
            .iter()
            .filter(|l| matches!(l, Load::Bookmarks(_)))
            .count(),
        1
    );
    let fx = bookmarks_loaded(&mut s, &["b1"]);
    assert!(loads(&fx).is_empty());
    // Pins: opening twice before the answer sends one load.
    let mut s = state();
    assert_eq!(loads(&reduce(&mut s, Input::OpenPins(cid("c1")))).len(), 1);
    assert!(loads(&reduce(&mut s, Input::OpenPins(cid("c1")))).is_empty());
}

#[test]
fn bookmark_remove_flight_drops_snapshot_and_rereads() {
    let mut s = state();
    bookmarks_loaded(&mut s, &["b1", "b2"]);
    let rm = |id: &str| Input::RemoveBookmark {
        conversation: cid("c1"),
        id: BookmarkId(Arc::from(id)),
    };
    let rm_call = |id: &str| ProviderCall::RemoveBookmark {
        conversation: cid("c1"),
        id: BookmarkId(Arc::from(id)),
    };
    reduce(&mut s, rm("b1"));
    reduce(&mut s, rm("b2"));
    assert!(s.bookmarks[&cid("c1")].is_empty());
    // A snapshot that predates the removals is dropped.
    reduce(
        &mut s,
        Input::Provider(ProviderEvent::BookmarksChanged(cid("c1"))),
    );
    bookmarks_loaded(&mut s, &["b1", "b2"]);
    assert!(s.bookmarks[&cid("c1")].is_empty());
    // First answer fails with one still in flight: no refetch yet.
    assert!(loads(&finish(&mut s, rm_call("b1"), false)).is_empty());
    // The last answer settles: one re-read.
    let fx = finish(&mut s, rm_call("b2"), true);
    assert!(matches!(&loads(&fx)[..], [Load::Bookmarks(_)]));
    bookmarks_loaded(&mut s, &["b1"]);
    assert_eq!(s.bookmarks[&cid("c1")].len(), 1);
}
