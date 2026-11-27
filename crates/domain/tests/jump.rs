use domain::test_support::{cid, conv, mid, msg, ts, uid};
use domain::{
    AroundPage, Changed, Effect, HistoryPage, Input, Message, ProviderEvent, WorkspaceState, reduce,
};
use settings::WorkspaceKey;

fn t(n: u32) -> String {
    format!("1.{n:06}")
}
fn m(n: u32) -> Message {
    msg("c1", &t(n), "x")
}
fn state(cap: usize, newest: u32) -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), cap);
    s.self_user = Some(uid("me"));
    s.conversations.insert(cid("c1"), conv("c1", "general"));
    reduce(
        &mut s,
        Input::HistoryLoaded {
            conversation: cid("c1"),
            page: HistoryPage {
                messages: ((newest - 4)..=newest).map(m).collect(),
                has_more: true,
            },
        },
    );
    s
}
fn around(from: u32, to: u32, more_before: bool, more_after: bool) -> AroundPage {
    AroundPage {
        messages: (from..=to).map(m).collect(),
        has_more_before: more_before,
        has_more_after: more_after,
    }
}
fn detach(s: &mut WorkspaceState, target: u32, from: u32, to: u32) {
    reduce(
        s,
        Input::AroundLoaded {
            target: mid("c1", &t(target)),
            page: around(from, to, true, true),
        },
    );
}

#[test]
fn jump_inside_window_only_marks() {
    let mut s = state(200, 100);
    let fx = reduce(&mut s, Input::JumpTo(mid("c1", &t(98))));
    assert_eq!(s.jump, Some(mid("c1", &t(98))));
    assert!(!fx.iter().any(|e| matches!(e, Effect::FetchAround { .. })));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Jump(c)) if *c == cid("c1")))
    );
    assert!(s.windows[&cid("c1")].is_live());
}

#[test]
fn jump_outside_fetches_around_and_detaches() {
    let mut s = state(200, 100);
    let fx = reduce(&mut s, Input::JumpTo(mid("c1", &t(10))));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::FetchAround { target } if *target == mid("c1", &t(10))))
    );
    assert!(s.jump.is_none());
    detach(&mut s, 10, 5, 15);
    let w = &s.windows[&cid("c1")];
    assert_eq!(w.messages().len(), 11);
    assert!(!w.is_live() && w.has_more_after() && w.has_more_before());
    assert_eq!(s.jump, Some(mid("c1", &t(10))));
}

#[test]
fn live_post_while_detached_not_inserted() {
    let mut s = state(200, 100);
    detach(&mut s, 10, 5, 15);
    reduce(
        &mut s,
        Input::Provider(ProviderEvent::MessagePosted(m(101))),
    );
    let w = &s.windows[&cid("c1")];
    assert_eq!(w.messages().len(), 11);
    assert!(w.has_more_after());
    assert_eq!(s.conversations[&cid("c1")].latest, Some(ts(&t(101))));
}

#[test]
fn scrolling_down_fetches_newer_until_live() {
    let mut s = state(200, 100);
    detach(&mut s, 10, 5, 15);
    let fx = reduce(
        &mut s,
        Input::Focus {
            conversation: Some(cid("c1")),
            at_bottom: true,
            window_active: true,
        },
    );
    assert!(fx.iter().any(
        |e| matches!(e, Effect::FetchNewer { conversation, after } if *conversation == cid("c1") && *after == ts(&t(15)))
    ));
    assert!(!fx.iter().any(|e| matches!(e, Effect::MarkRead { .. })));
    reduce(
        &mut s,
        Input::NewerLoaded {
            conversation: cid("c1"),
            page: HistoryPage {
                messages: (16..=25).map(m).collect(),
                has_more: true,
            },
        },
    );
    assert!(!s.windows[&cid("c1")].is_live());
    assert_eq!(s.windows[&cid("c1")].newest_ts(), Some(&ts(&t(25))));
    reduce(
        &mut s,
        Input::NewerLoaded {
            conversation: cid("c1"),
            page: HistoryPage {
                messages: (26..=30).map(m).collect(),
                has_more: false,
            },
        },
    );
    assert!(s.windows[&cid("c1")].is_live());
}

#[test]
fn jump_to_reply_opens_root_and_thread() {
    let mut s = state(200, 100);
    let mut r = m(98);
    r.thread_root = Some(ts(&t(97)));
    reduce(
        &mut s,
        Input::HistoryLoaded {
            conversation: cid("c1"),
            page: HistoryPage {
                messages: vec![r.clone()],
                has_more: true,
            },
        },
    );
    let fx = reduce(&mut s, Input::JumpTo(r.id.clone()));
    assert_eq!(s.jump, Some(mid("c1", &t(97))));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::FetchReplies { root, .. } if *root == mid("c1", &t(97))))
    );
    assert_eq!(s.threads[&mid("c1", &t(97))].jump, Some(r.id));
}

#[test]
fn jumping_to_a_reply_of_an_open_thread_adds_no_viewer() {
    let mut s = state(200, 100);
    let root = mid("c1", &t(97));
    reduce(&mut s, Input::OpenThread(root.clone()));
    let mut r = m(101);
    r.thread_root = Some(ts(&t(97)));
    reduce(
        &mut s,
        Input::RepliesLoaded {
            root: root.clone(),
            page: HistoryPage {
                messages: vec![r.clone()],
                has_more: false,
            },
        },
    );
    reduce(&mut s, Input::JumpTo(r.id));
    assert_eq!(s.threads[&root].viewers, 1);
    reduce(&mut s, Input::CloseThread(root.clone()));
    assert!(!s.threads.contains_key(&root));
}

#[test]
fn jump_to_unheld_reply_loads_root_around() {
    let mut s = state(200, 100);
    let mut r = m(11);
    r.thread_root = Some(ts(&t(10)));
    let fx = reduce(
        &mut s,
        Input::AroundLoaded {
            target: r.id.clone(),
            page: AroundPage {
                messages: vec![r.clone()],
                has_more_before: false,
                has_more_after: false,
            },
        },
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::FetchAround { target } if *target == mid("c1", &t(10))))
    );
    assert_eq!(s.threads[&mid("c1", &t(10))].jump, Some(r.id));
    // The channel window was not replaced by the reply.
    assert_eq!(s.windows[&cid("c1")].messages().len(), 5);
}

#[test]
fn jump_to_latest_reloads() {
    let mut s = state(200, 100);
    detach(&mut s, 10, 5, 15);
    let fx = reduce(&mut s, Input::JumpToLatest(cid("c1")));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Reload(c) if *c == cid("c1")))
    );
    assert!(s.windows.get(&cid("c1")).is_none_or(|w| w.is_empty()));
    assert!(s.jump.is_none());
}

#[test]
fn detached_window_respects_capacity() {
    let mut s = state(10, 100);
    detach(&mut s, 50, 20, 80);
    let w = &s.windows[&cid("c1")];
    assert_eq!(w.messages().len(), 10);
    assert!(w.messages().iter().any(|x| x.id == mid("c1", &t(50))));
    assert!(w.has_more_before() && w.has_more_after());
    reduce(
        &mut s,
        Input::NewerLoaded {
            conversation: cid("c1"),
            page: HistoryPage {
                messages: (81..=95).map(m).collect(),
                has_more: true,
            },
        },
    );
    assert_eq!(s.windows[&cid("c1")].messages().len(), 10);
}
