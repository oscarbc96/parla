//! `WorkspaceSession` against `FakeProvider` and `MemoryStore`; no network.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use components::{ComposerSource, FileNotice, TimelineSource};
use domain::test_support::{FakeProvider, Op, cid, conv, member, mid, msg, uid};
use domain::{ChatProvider, DeliveryState, ProviderError, ProviderEvent, Session};
use gpui::{AppContext as _, Entity, TestAppContext};
use parla::session::{SessionEvent, WorkspaceSession};
use store::{MemoryStore, Store};

type Events = Rc<RefCell<Vec<SessionEvent>>>;

struct Fixture {
    provider: Arc<FakeProvider>,
    store: Arc<MemoryStore>,
    session: Entity<WorkspaceSession>,
    events: Events,
}

fn setup(cx: &mut TestAppContext) -> Fixture {
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.add_conversation(conv("C1", "general"));
    provider.add_conversation(conv("C2", "random"));
    provider.add_member(member("u1", "Me"));
    provider.add_member(member("u2", "Other"));
    provider.add_message(msg("C1", "1.000000", "hi"));
    let store = Arc::new(MemoryStore::default());
    let p: Arc<dyn ChatProvider> = provider.clone();
    let st: Arc<dyn Store> = store.clone();
    let session = cx.new(|cx| WorkspaceSession::new(p, st, cx));
    let events: Events = Rc::default();
    let sink = events.clone();
    cx.update(|cx| {
        cx.subscribe(&session, move |_, ev: &SessionEvent, _| {
            sink.borrow_mut().push(ev.clone())
        })
        .detach()
    });
    cx.run_until_parked();
    Fixture {
        provider,
        store,
        session,
        events,
    }
}

#[gpui::test]
async fn bootstrap_then_open_loads_history(cx: &mut TestAppContext) {
    let f = setup(cx);
    f.session.read_with(cx, |s, _| {
        assert_eq!(s.state().conversations.len(), 2);
    });
    f.session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    f.session.read_with(cx, |s, _| {
        let w = s.state().windows.get(&cid("C1")).expect("window");
        assert_eq!(w.messages().len(), 1);
    });
}

#[gpui::test]
async fn send_shows_pending_then_delivered(cx: &mut TestAppContext) {
    let f = setup(cx);
    f.provider
        .push_event(ProviderEvent::Connected { reconnect: false });
    f.session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    f.session
        .update(cx, |s, cx| s.send(cid("C1"), "hello".into(), cx));
    f.session.read_with(cx, |s, _| {
        let m = s.state().windows[&cid("C1")]
            .messages()
            .last()
            .unwrap()
            .clone();
        assert_eq!(m.state, DeliveryState::Pending);
    });
    cx.run_until_parked();
    f.session.read_with(cx, |s, _| {
        let w = s.state().windows[&cid("C1")].messages();
        assert_eq!(w.len(), 2);
        assert_eq!(w.last().unwrap().state, DeliveryState::Delivered);
    });
    assert!(
        f.provider
            .calls()
            .iter()
            .any(|c| matches!(c, domain::test_support::Call::Send { text, .. } if text == "hello"))
    );
}

#[gpui::test]
async fn timeline_event_only_for_its_conversation(cx: &mut TestAppContext) {
    let f = setup(cx);
    f.session.update(cx, |s, cx| s.open(cid("C1"), cx));
    f.session.update(cx, |s, cx| s.open(cid("C2"), cx));
    cx.run_until_parked();
    f.events.borrow_mut().clear();
    f.provider
        .push_event(ProviderEvent::MessagePosted(msg("C2", "5.000000", "x")));
    cx.run_until_parked();
    let timelines: Vec<_> = f
        .events
        .borrow()
        .iter()
        .filter(|e| matches!(e, SessionEvent::Timeline(_)))
        .cloned()
        .collect();
    assert_eq!(timelines, vec![SessionEvent::Timeline(cid("C2"))]);
}

#[gpui::test]
async fn expired_emits_session_event(cx: &mut TestAppContext) {
    let f = setup(cx);
    f.provider.push_event(ProviderEvent::SessionExpired);
    cx.run_until_parked();
    assert!(f.events.borrow().contains(&SessionEvent::Session));
    f.session
        .read_with(cx, |s, _| assert_eq!(s.state().session, Session::Expired));
}

#[gpui::test]
async fn store_receives_writes(cx: &mut TestAppContext) {
    let f = setup(cx);
    f.session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    let ws = f.provider.workspace().clone();
    let convs = futures::executor::block_on(f.store.conversations(&ws)).unwrap();
    assert_eq!(convs.len(), 2);
    let msgs =
        futures::executor::block_on(f.store.messages_before(&ws, &cid("C1"), None, 10)).unwrap();
    assert_eq!(msgs.len(), 1);
}

#[gpui::test]
async fn react_is_optimistic_and_calls_provider(cx: &mut TestAppContext) {
    use domain::EmojiRef;
    use domain::test_support::{Call, mid};
    let f = setup(cx);
    f.provider
        .push_event(ProviderEvent::Connected { reconnect: false });
    f.session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    let emoji = EmojiRef {
        shortcode: "thumbsup".into(),
        unicode: Some("\u{1F44D}".into()),
        skin_tone: None,
    };
    f.session.update(cx, |s, cx| {
        s.react(mid("C1", "1.000000"), emoji.clone(), true, cx)
    });
    f.session.read_with(cx, |s, _| {
        let m = &s.state().windows[&cid("C1")].messages()[0];
        assert_eq!(m.reactions.len(), 1);
        assert!(m.reactions[0].me);
    });
    cx.run_until_parked();
    assert!(f.provider.calls().iter().any(|c| matches!(c,
        Call::React { emoji: e, add: true, .. } if e.shortcode == emoji.shortcode)));
}

#[gpui::test]
async fn thread_open_reply_close(cx: &mut TestAppContext) {
    use domain::test_support::{Call, mid};
    let f = setup(cx);
    f.provider
        .push_event(ProviderEvent::Connected { reconnect: false });
    f.session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    let root = mid("C1", "1.000000");
    f.session
        .update(cx, |s, cx| s.open_thread(root.clone(), cx));
    cx.run_until_parked();
    f.session
        .read_with(cx, |s, _| assert!(s.state().threads.contains_key(&root)));
    f.session
        .update(cx, |s, cx| s.reply(root.clone(), "re".into(), true, cx));
    f.session.read_with(cx, |s, _| {
        let t = &s.state().threads[&root];
        assert_eq!(
            t.replies.messages().last().unwrap().state,
            DeliveryState::Pending
        );
    });
    cx.run_until_parked();
    assert!(f.provider.calls().iter().any(|c| matches!(c,
        Call::Reply { text, broadcast: true, .. } if text == "re")));
    f.session
        .update(cx, |s, cx| s.close_thread(root.clone(), cx));
    f.session
        .read_with(cx, |s, _| assert!(!s.state().threads.contains_key(&root)));
}

#[gpui::test]
async fn jump_far_back_detaches_then_scrolling_down_reattaches(cx: &mut TestAppContext) {
    let f = setup(cx);
    for n in 1..=120u32 {
        f.provider.add_message(msg("C1", &format!("2.{n:06}"), "x"));
    }
    f.session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    let target = domain::test_support::mid("C1", "2.000010");
    f.session.update(cx, |s, cx| s.jump_to(target.clone(), cx));
    cx.run_until_parked();
    f.session.read_with(cx, |s, _| {
        let w = &s.state().windows[&cid("C1")];
        assert!(!w.is_live());
        assert!(w.messages().iter().any(|m| m.id == target));
        assert_eq!(s.state().jump, Some(target.clone()));
    });
    let mut live = false;
    for _ in 0..6 {
        f.session.update(cx, |s, cx| {
            s.focus(cid("C1"), true, true, cx);
        });
        cx.run_until_parked();
        live = f
            .session
            .read_with(cx, |s, _| s.state().windows[&cid("C1")].is_live());
        if live {
            break;
        }
    }
    assert!(live);
}

struct Bracket;

impl domain::ContentEncoder for Bracket {
    fn encode(&self, t: &domain::ComposedText) -> String {
        format!("[{}]", t.text)
    }
}

fn tada() -> domain::EmojiRef {
    domain::EmojiRef {
        shortcode: "tada".into(),
        unicode: Some("\u{1f389}".into()),
        skin_tone: None,
    }
}

#[gpui::test]
async fn recent_emoji_persisted_across_sessions(cx: &mut TestAppContext) {
    let f = setup(cx);
    f.session
        .update(cx, |s, cx| ComposerSource::emoji_used(s, tada(), cx));
    cx.run_until_parked();
    assert_eq!(
        f.session
            .read_with(cx, |s, _| ComposerSource::recent_emoji(s)),
        [tada()]
    );
    // A new session on the same store (a restart) starts with it.
    let p: Arc<dyn ChatProvider> = f.provider.clone();
    let st: Arc<dyn Store> = f.store.clone();
    let again = cx.new(|cx| WorkspaceSession::new(p, st, cx));
    cx.run_until_parked();
    assert_eq!(
        again.read_with(cx, |s, _| ComposerSource::recent_emoji(s)),
        [tada()]
    );
}

#[gpui::test]
async fn adding_a_reaction_counts_as_using_the_emoji(cx: &mut TestAppContext) {
    let f = setup(cx);
    f.session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    let id = f.session.read_with(cx, |s, _| {
        s.state().windows[&cid("C1")].messages()[0].id.clone()
    });
    f.session.update(cx, |s, cx| s.react(id, tada(), true, cx));
    cx.run_until_parked();
    assert_eq!(
        f.session
            .read_with(cx, |s, _| ComposerSource::recent_emoji(s)),
        [tada()]
    );
}

#[gpui::test]
async fn session_encodes_with_the_provider_encoder(cx: &mut TestAppContext) {
    let f = setup(cx);
    let body = domain::ComposedText::plain("hi");
    let plain = f
        .session
        .read_with(cx, |s, _| ComposerSource::encode(s, &body));
    assert_eq!(plain, "hi", "plain until the provider's encoder is set");
    f.session
        .update(cx, |s, _| s.set_encoder(Arc::new(Bracket)));
    let got = f
        .session
        .read_with(cx, |s, _| ComposerSource::encode(s, &body));
    assert_eq!(got, "[hi]");
}

#[gpui::test]
async fn session_completes_people_channels_and_emoji(cx: &mut TestAppContext) {
    use components::CompletionKind;
    let f = setup(cx);
    f.session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    let labels = |kind, q: &str| {
        f.session.read_with(cx, |s, _| {
            ComposerSource::completions(s, kind, q)
                .into_iter()
                .map(|c| c.label.to_string())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(labels(CompletionKind::Person, "oth"), ["@Other"]);
    assert_eq!(labels(CompletionKind::Person, "here"), ["@here"]);
    assert_eq!(labels(CompletionKind::Conversation, "ran"), ["#random"]);
    assert_eq!(labels(CompletionKind::Emoji, "tada")[0], "\u{1f389}");
}

#[gpui::test]
async fn slash_command_runs_through_the_reducer(cx: &mut TestAppContext) {
    let f = setup(cx);
    f.provider
        .push_event(ProviderEvent::Connected { reconnect: false });
    cx.run_until_parked();
    let r = f.session.update(cx, |s, cx| {
        ComposerSource::run_slash(s, domain::SlashCommand::Dnd(Some(30)), cx)
    });
    assert_eq!(r, Ok(()));
    cx.run_until_parked();
    f.session.read_with(cx, |s, _| {
        let me = uid("u1");
        let until = s.state().dnd.get(&me).and_then(|d| d.snooze_until);
        assert!(until.is_some(), "snooze set");
    });
    // An unknown person is an error and changes nothing.
    let r = f.session.update(cx, |s, cx| {
        ComposerSource::run_slash(
            s,
            domain::SlashCommand::Remind {
                target: domain::RemindTarget::Person("nobody".into()),
                what: "x".into(),
                when: "in 5 minutes".into(),
            },
            cx,
        )
    });
    assert_eq!(r, Err(domain::SlashError::UnknownName("nobody".into())));
}

#[gpui::test]
async fn expired_session_during_bootstrap_does_not_also_time_out(cx: &mut TestAppContext) {
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.hang_bootstrap();
    let store = Arc::new(MemoryStore::default());
    let p: Arc<dyn ChatProvider> = provider.clone();
    let st: Arc<dyn Store> = store;
    let session = cx.new(|cx| WorkspaceSession::new(p, st, cx));
    cx.run_until_parked();
    provider.push_event(ProviderEvent::SessionExpired);
    cx.run_until_parked();
    cx.executor()
        .advance_clock(std::time::Duration::from_secs(60));
    cx.run_until_parked();
    session.read_with(cx, |s, _| {
        assert_eq!(s.state().session, Session::Expired);
        assert!(
            s.bootstrap_error().is_none(),
            "expiry owns the banner, no timeout strip"
        );
    });
}

fn notice(f: &Fixture, cx: &mut TestAppContext) -> Option<FileNotice> {
    f.session.read_with(cx, |s, _| s.file_notice())
}

fn first_text(f: &Fixture, cx: &mut TestAppContext) -> String {
    f.session.read_with(cx, |s, _| {
        s.state().windows[&cid("C1")].messages()[0].text.to_string()
    })
}

#[gpui::test]
async fn edit_failure_shows_a_notice_and_keeps_the_message(cx: &mut TestAppContext) {
    let f = setup(cx);
    f.session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    f.provider
        .fail_next(Op::Edit, ProviderError::Network("down".into()));
    f.session
        .update(cx, |s, cx| s.edit(mid("C1", "1.000000"), "new".into(), cx));
    cx.run_until_parked();
    assert_eq!(
        notice(&f, cx),
        Some(FileNotice::Error(i18n::LocalizedText::new(
            "parla",
            "message-edit-failed"
        )))
    );
    assert_eq!(first_text(&f, cx), "hi");
}

#[gpui::test]
async fn delete_failure_shows_a_notice_and_keeps_the_message(cx: &mut TestAppContext) {
    let f = setup(cx);
    f.session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    f.provider
        .fail_next(Op::Delete, ProviderError::NotAllowed("no".into()));
    f.session
        .update(cx, |s, cx| s.delete(mid("C1", "1.000000"), cx));
    cx.run_until_parked();
    assert_eq!(
        notice(&f, cx),
        Some(FileNotice::Error(i18n::LocalizedText::new(
            "parla",
            "message-delete-failed"
        )))
    );
    assert_eq!(first_text(&f, cx), "hi");
}

#[gpui::test]
async fn failed_send_can_be_retried_or_discarded(cx: &mut TestAppContext) {
    let f = setup(cx);
    f.provider
        .push_event(ProviderEvent::Connected { reconnect: false });
    f.session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    f.provider
        .fail_next(Op::Send, ProviderError::Other("boom".into()));
    f.session
        .update(cx, |s, cx| s.send(cid("C1"), "hello".into(), cx));
    cx.run_until_parked();
    let failed = |f: &Fixture, cx: &mut TestAppContext| {
        f.session.read_with(cx, |s, _| {
            s.state().windows[&cid("C1")]
                .messages()
                .iter()
                .find(|m| matches!(m.state, DeliveryState::Failed(_)))
                .and_then(|m| m.client_msg_id.clone())
        })
    };
    let id = failed(&f, cx).expect("the send failed and stays visible");
    f.session
        .update(cx, |s, cx| TimelineSource::retry_send(s, id.clone(), cx));
    cx.run_until_parked();
    assert!(failed(&f, cx).is_none(), "retry delivered it");

    f.provider
        .fail_next(Op::Send, ProviderError::Other("boom".into()));
    f.session
        .update(cx, |s, cx| s.send(cid("C1"), "again".into(), cx));
    cx.run_until_parked();
    let id = failed(&f, cx).expect("failed again");
    f.session
        .update(cx, |s, cx| TimelineSource::discard_send(s, id.clone(), cx));
    cx.run_until_parked();
    assert!(failed(&f, cx).is_none());
    f.session.read_with(cx, |s, _| {
        assert!(
            s.state().windows[&cid("C1")]
                .messages()
                .iter()
                .all(|m| &*m.text != "again")
        );
    });
}

#[gpui::test]
async fn a_failed_first_page_is_flagged_and_retry_loads_it(cx: &mut TestAppContext) {
    use domain::ProviderError;
    use domain::test_support::Op;
    let f = setup(cx);
    f.provider
        .fail_next(Op::History, ProviderError::Other("boom".into()));
    f.session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    f.session.read_with(cx, |s, _| {
        assert!(s.history_failed(&cid("C1")));
        assert!(
            !s.state()
                .windows
                .get(&cid("C1"))
                .is_some_and(|w| w.is_loaded())
        );
    });
    f.session.update(cx, |s, cx| s.retry_history(cid("C1"), cx));
    cx.run_until_parked();
    f.session.read_with(cx, |s, _| {
        assert!(!s.history_failed(&cid("C1")));
        let w = s.state().windows.get(&cid("C1")).expect("window");
        assert!(w.is_loaded() && w.messages().len() == 1);
    });
}

#[gpui::test]
async fn failed_thread_replies_are_flagged_and_retry_loads_them(cx: &mut TestAppContext) {
    use domain::ProviderError;
    use domain::test_support::{Op, mid};
    let f = setup(cx);
    f.session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    let root = mid("C1", "1.000000");
    f.provider
        .fail_next(Op::Replies, ProviderError::Other("boom".into()));
    f.session
        .update(cx, |s, cx| s.open_thread(root.clone(), cx));
    cx.run_until_parked();
    f.session.read_with(cx, |s, _| {
        assert!(s.replies_failed(&root));
        assert!(!s.state().threads[&root].replies.is_loaded());
    });
    f.session
        .update(cx, |s, cx| s.retry_replies(root.clone(), cx));
    cx.run_until_parked();
    f.session.read_with(cx, |s, _| {
        assert!(!s.replies_failed(&root));
        assert!(s.state().threads[&root].replies.is_loaded());
    });
}

#[gpui::test]
async fn reopening_after_a_failure_clears_the_error_and_sends_one_request(cx: &mut TestAppContext) {
    use domain::ProviderError;
    use domain::test_support::Op;
    let f = setup(cx);
    f.provider
        .fail_next(Op::History, ProviderError::Other("boom".into()));
    f.session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    f.session
        .read_with(cx, |s, _| assert!(s.history_failed(&cid("C1"))));
    let before = f.provider.request_count();
    // Re-open and retry in the same turn: the error clears at once and only
    // one request goes out.
    f.session.update(cx, |s, cx| {
        s.open(cid("C1"), cx);
        assert!(!s.history_failed(&cid("C1")));
        s.retry_history(cid("C1"), cx);
    });
    cx.run_until_parked();
    let after = f.provider.request_count();
    assert_eq!(after - before, 1);
    f.session
        .read_with(cx, |s, _| assert!(!s.history_failed(&cid("C1"))));
}

#[gpui::test]
async fn reopening_a_thread_after_a_failure_clears_the_error(cx: &mut TestAppContext) {
    use domain::ProviderError;
    use domain::test_support::{Op, mid};
    let f = setup(cx);
    f.session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    let root = mid("C1", "1.000000");
    f.provider
        .fail_next(Op::Replies, ProviderError::Other("boom".into()));
    f.session
        .update(cx, |s, cx| s.open_thread(root.clone(), cx));
    cx.run_until_parked();
    f.session
        .read_with(cx, |s, _| assert!(s.replies_failed(&root)));
    f.session.update(cx, |s, cx| {
        s.open_thread(root.clone(), cx);
        assert!(!s.replies_failed(&root));
    });
    cx.run_until_parked();
    f.session.read_with(cx, |s, _| {
        assert!(s.state().threads[&root].replies.is_loaded())
    });
}

#[gpui::test]
async fn closing_one_viewer_keeps_the_replies_error(cx: &mut TestAppContext) {
    use domain::ProviderError;
    use domain::test_support::{Op, mid};
    let f = setup(cx);
    f.session.update(cx, |s, cx| s.open(cid("C1"), cx));
    cx.run_until_parked();
    let root = mid("C1", "1.000000");
    f.provider
        .fail_next(Op::Replies, ProviderError::Other("boom".into()));
    f.session.update(cx, |s, cx| {
        s.open_thread(root.clone(), cx);
        s.open_thread(root.clone(), cx);
    });
    cx.run_until_parked();
    f.session
        .update(cx, |s, cx| s.close_thread(root.clone(), cx));
    f.session
        .read_with(cx, |s, _| assert!(s.replies_failed(&root)));
    f.session
        .update(cx, |s, cx| s.close_thread(root.clone(), cx));
    f.session
        .read_with(cx, |s, _| assert!(!s.replies_failed(&root)));
}

#[gpui::test]
async fn disconnect_marks_offline_and_reconnect_clears_it(cx: &mut TestAppContext) {
    let f = setup(cx);
    f.provider
        .push_event(ProviderEvent::Connected { reconnect: false });
    cx.run_until_parked();
    f.session
        .read_with(cx, |s, _| assert!(!TimelineSource::offline(s)));
    f.provider.push_event(ProviderEvent::Disconnected);
    cx.run_until_parked();
    f.session
        .read_with(cx, |s, _| assert!(TimelineSource::offline(s)));
    f.provider
        .push_event(ProviderEvent::Connected { reconnect: true });
    cx.run_until_parked();
    f.session
        .read_with(cx, |s, _| assert!(!TimelineSource::offline(s)));
}

#[gpui::test]
async fn retry_connection_asks_the_provider_to_reconnect_now(cx: &mut TestAppContext) {
    let f = setup(cx);
    f.provider.push_event(ProviderEvent::Disconnected);
    cx.run_until_parked();
    f.session.update(cx, |s, _| s.retry_connection());
    assert!(
        f.provider
            .calls()
            .iter()
            .any(|c| matches!(c, domain::test_support::Call::ReconnectNow))
    );
}

#[gpui::test]
async fn expired_session_makes_the_composer_read_only(cx: &mut TestAppContext) {
    let f = setup(cx);
    f.session
        .read_with(cx, |s, _| assert!(!ComposerSource::read_only(s)));
    f.provider.push_event(ProviderEvent::SessionExpired);
    cx.run_until_parked();
    f.session
        .read_with(cx, |s, _| assert!(ComposerSource::read_only(s)));
}

#[gpui::test]
async fn message_window_follows_the_setting(cx: &mut TestAppContext) {
    let dir = tempfile::tempdir().unwrap();
    let paths = settings::ConfigPaths::at(dir.path());
    std::fs::write(
        &paths.settings,
        r#"{ "performance": { "message_window": 50 } }"#,
    )
    .unwrap();
    let p = paths.clone();
    cx.update(|cx| cx.set_global(settings::SettingsStore::load(p)));
    let f = setup(cx);
    f.session
        .read_with(cx, |s, _| assert_eq!(s.state().window_capacity, 50));
    std::fs::write(
        &paths.settings,
        r#"{ "performance": { "message_window": 120 } }"#,
    )
    .unwrap();
    cx.update(|cx| {
        cx.global_mut::<settings::SettingsStore>().reload();
    });
    cx.run_until_parked();
    f.session
        .read_with(cx, |s, _| assert_eq!(s.state().window_capacity, 120));
}

/// A failed bootstrap says nothing about the socket: a live connection stays
/// online, so queued sends are not held for a reconnect that never comes.
#[gpui::test]
async fn a_failed_bootstrap_leaves_the_connection_online(cx: &mut TestAppContext) {
    let provider = Arc::new(FakeProvider::new("w1", uid("u1")));
    provider.add_member(member("u1", "Me"));
    provider.push_event(ProviderEvent::Connected { reconnect: false });
    provider.fail_next(
        domain::test_support::Op::Bootstrap,
        domain::ProviderError::Network("down".into()),
    );
    let store = Arc::new(MemoryStore::default());
    let p: Arc<dyn ChatProvider> = provider.clone();
    let st: Arc<dyn Store> = store;
    let session = cx.new(|cx| WorkspaceSession::new(p, st, cx));
    cx.run_until_parked();
    session.read_with(cx, |s, _| {
        assert!(s.bootstrap_error().is_some(), "the failure is shown");
        assert_eq!(s.state().connection, domain::Connection::Online);
    });
}
