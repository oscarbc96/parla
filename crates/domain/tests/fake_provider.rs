use std::sync::Arc;

use domain::test_support::{Call, FakeProvider, Op, cid, cmid, conv, member, msg, ts, uid};
use domain::{Capabilities, ChatProvider, ProviderError, ProviderEvent};
use futures::StreamExt;
use futures::executor::block_on;

fn fake() -> FakeProvider {
    let f = FakeProvider::new("w1", uid("me"));
    f.add_conversation(conv("c1", "general"));
    f.add_member(member("u2", "bo"));
    for i in 1..=5 {
        f.add_message(msg("c1", &format!("{i}.000000"), &format!("m{i}")));
    }
    f
}

fn texts(p: &domain::HistoryPage) -> Vec<String> {
    p.messages.iter().map(|m| m.text.to_string()).collect()
}

#[test]
fn bootstrap_returns_script() {
    let f = fake();
    let b = block_on(f.bootstrap()).unwrap();
    assert_eq!(b.self_user, uid("me"));
    assert_eq!(b.conversations.len(), 1);
    assert_eq!(b.members.len(), 1);
}

#[test]
fn history_pages_newest_first_then_older() {
    let f = fake();
    let p1 = block_on(f.history(&cid("c1"), None, 2)).unwrap();
    assert_eq!(texts(&p1), ["m4", "m5"]);
    assert!(p1.has_more);
    let p2 = block_on(f.history(&cid("c1"), Some(ts("4.000000")), 2)).unwrap();
    assert_eq!(texts(&p2), ["m2", "m3"]);
    assert!(p2.has_more);
    let p3 = block_on(f.history(&cid("c1"), Some(ts("2.000000")), 2)).unwrap();
    assert_eq!(texts(&p3), ["m1"]);
    assert!(!p3.has_more);
}

#[test]
fn send_is_recorded_and_stored() {
    let f = fake();
    let m = block_on(f.send(&cid("c1"), "hi", None, cmid("k1"))).unwrap();
    assert_eq!(&*m.text, "hi");
    assert_eq!(m.author, Some(uid("me")));
    assert_eq!(m.client_msg_id, Some(cmid("k1")));
    assert!(m.id.ts > ts("5.000000"));
    assert_eq!(
        f.calls(),
        [Call::Send {
            conversation: cid("c1"),
            text: "hi".into(),
            thread: None,
            id: cmid("k1")
        }]
    );
    let p = block_on(f.history(&cid("c1"), None, 1)).unwrap();
    assert_eq!(texts(&p), ["hi"]);
}

#[test]
fn edit_delete_mark_are_recorded_and_applied() {
    let f = fake();
    let id = domain::MessageId {
        conversation: cid("c1"),
        ts: ts("5.000000"),
    };
    block_on(f.edit(&id, "m5!")).unwrap();
    block_on(f.mark_read(&cid("c1"), &ts("5.000000"))).unwrap();
    let p = block_on(f.history(&cid("c1"), None, 1)).unwrap();
    assert_eq!(texts(&p), ["m5!"]);
    block_on(f.delete(&id)).unwrap();
    let p = block_on(f.history(&cid("c1"), None, 1)).unwrap();
    assert_eq!(texts(&p), ["m4"]);
    assert_eq!(
        f.calls(),
        [
            Call::Edit {
                id: id.clone(),
                text: "m5!".into()
            },
            Call::MarkRead {
                conversation: cid("c1"),
                ts: ts("5.000000")
            },
            Call::Delete(id),
        ]
    );
}

#[test]
fn events_replay_buffered_then_live_in_order() {
    let f = fake();
    f.push_event(ProviderEvent::Connected { reconnect: false });
    let mut s = f.events();
    f.push_event(ProviderEvent::MessagePosted(msg("c1", "9.000000", "late")));
    f.push_event(ProviderEvent::Disconnected);
    let got: Vec<_> = block_on(s.by_ref().take(3).collect());
    assert!(matches!(
        got[0],
        ProviderEvent::Connected { reconnect: false }
    ));
    assert!(matches!(&got[1], ProviderEvent::MessagePosted(m) if &*m.text == "late"));
    assert!(matches!(got[2], ProviderEvent::Disconnected));
}

#[test]
fn events_stream_ends_when_provider_dropped() {
    let f = fake();
    let mut s = f.events();
    f.push_event(ProviderEvent::Disconnected);
    drop(f);
    assert!(block_on(s.next()).is_some());
    assert!(block_on(s.next()).is_none());
}

#[test]
fn fail_next_injects_error_once_and_does_not_record() {
    let f = fake();
    f.fail_next(Op::Send, ProviderError::RateLimited);
    let e = block_on(f.send(&cid("c1"), "x", None, cmid("k"))).unwrap_err();
    assert_eq!(e, ProviderError::RateLimited);
    assert!(f.calls().is_empty());
    block_on(f.send(&cid("c1"), "x", None, cmid("k"))).unwrap();
    assert_eq!(f.calls().len(), 1);
    // other ops unaffected
    f.fail_next(Op::History, ProviderError::SessionExpired);
    assert!(block_on(f.bootstrap()).is_ok());
    assert_eq!(
        block_on(f.history(&cid("c1"), None, 1)).unwrap_err(),
        ProviderError::SessionExpired
    );
}

#[test]
fn capabilities_are_configurable() {
    let f = fake();
    assert_eq!(f.capabilities(), &Capabilities::default());
    let f = f.with_capabilities(Capabilities {
        threads: true,
        edit: true,
        ..Default::default()
    });
    assert!(f.capabilities().threads && f.capabilities().edit);
    assert_eq!(f.workspace().id.0, "w1");
    let _: Arc<dyn ChatProvider> = Arc::new(f);
}

#[test]
fn factory_parser_is_plain_unless_set() {
    use domain::test_support::FakeFactory;
    use domain::{ContentParser, Message, ProviderFactory, RichText};
    struct Upper;
    impl ContentParser for Upper {
        fn parse(&self, m: &Message) -> RichText {
            RichText::plain(&m.text.to_uppercase())
        }
    }
    let m = msg("c1", "1.000000", "hi");
    let plain = FakeFactory::default().content_parser().parse(&m);
    assert_eq!(plain.plain_text(), "hi");
    let set = FakeFactory::default()
        .with_parser(Arc::new(Upper))
        .content_parser()
        .parse(&m);
    assert_eq!(set.plain_text(), "HI");
}

#[test]
fn parse_text_defaults_to_plain() {
    use domain::{ContentParser, PlainParser};
    assert_eq!(PlainParser.parse_text("*a* <b>").plain_text(), "*a* <b>");
}

#[test]
fn lazy_history_is_built_once_on_first_read_and_stays_editable() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let built = Arc::new(AtomicUsize::new(0));
    let f = FakeProvider::new("w1", uid("me"));
    f.add_conversation(conv("c1", "general"));
    let counter = built.clone();
    f.add_lazy_messages(&cid("c1"), move || {
        counter.fetch_add(1, Ordering::SeqCst);
        (1..=3)
            .map(|i| msg("c1", &format!("{i}.000000"), &format!("m{i}")))
            .collect()
    });
    assert_eq!(built.load(Ordering::SeqCst), 0, "scripting reads nothing");
    block_on(f.bootstrap()).unwrap();
    assert_eq!(
        built.load(Ordering::SeqCst),
        0,
        "bootstrap does not read history"
    );

    let page = block_on(f.history(&cid("c1"), None, 10)).unwrap();
    assert_eq!(texts(&page), ["m1", "m2", "m3"]);
    // A new message lands after the generated ones and the generator is not run again.
    f.add_message(msg("c1", "4.000000", "m4"));
    let page = block_on(f.history(&cid("c1"), None, 10)).unwrap();
    assert_eq!(texts(&page), ["m1", "m2", "m3", "m4"]);
    assert_eq!(built.load(Ordering::SeqCst), 1);
}

#[test]
fn reconnect_now_is_recorded() {
    use domain::test_support::Call;
    let f = fake();
    f.reconnect_now();
    assert!(f.calls().iter().any(|c| matches!(c, Call::ReconnectNow)));
}
