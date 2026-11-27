//! M3 model: ids, DND, capabilities, default provider methods, fake script.

use std::sync::Arc;

use domain::test_support::{Call, FakeProvider, Op, cid, conv, msg, ts, uid};
use domain::{
    Capabilities, ChatProvider, ConversationPrefs, DndStatus, MessageTs, NotifyLevel,
    ProviderError, SearchQuery,
};
use futures::executor::block_on;
use futures::future::BoxFuture;
use futures::stream::BoxStream;

#[test]
fn just_after_is_one_micro_more() {
    let t = MessageTs(Arc::from("1700000000.999999"));
    assert_eq!(t.just_after().0.as_ref(), "1700000001.000000");
    assert_eq!(t.just_after().just_before(), t);
}

#[test]
fn dnd_active_rules() {
    let snoozed = DndStatus {
        snooze_until: Some(160),
        ..Default::default()
    };
    assert!(snoozed.active(100));
    assert!(!snoozed.active(160));
    let sched = DndStatus {
        enabled: true,
        next_start: Some(100),
        next_end: Some(200),
        snooze_until: None,
    };
    assert!(sched.active(100));
    assert!(sched.active(199));
    assert!(!sched.active(200));
    assert!(!sched.active(99));
    let off = DndStatus {
        enabled: false,
        ..sched
    };
    assert!(!off.active(150));
}

#[test]
fn new_caps_default_false() {
    let c = Capabilities::default();
    assert!(
        !(c.search
            || c.reminders
            || c.dnd
            || c.status
            || c.pins
            || c.bookmarks
            || c.saved
            || c.thread_inbox
            || c.notify_prefs)
    );
}

/// Implements only the required methods.
struct Minimal(settings::WorkspaceKey, Capabilities);

impl ChatProvider for Minimal {
    fn workspace(&self) -> &settings::WorkspaceKey {
        &self.0
    }
    fn capabilities(&self) -> &Capabilities {
        &self.1
    }
    fn bootstrap(&self) -> BoxFuture<'_, Result<domain::Bootstrap, ProviderError>> {
        unimplemented!()
    }
    fn history(
        &self,
        _: &domain::ConversationId,
        _: Option<MessageTs>,
        _: u32,
    ) -> BoxFuture<'_, Result<domain::HistoryPage, ProviderError>> {
        unimplemented!()
    }
    fn send(
        &self,
        _: &domain::ConversationId,
        _: &str,
        _: Option<MessageTs>,
        _: domain::ClientMsgId,
    ) -> BoxFuture<'_, Result<domain::Message, ProviderError>> {
        unimplemented!()
    }
    fn edit(&self, _: &domain::MessageId, _: &str) -> BoxFuture<'_, Result<(), ProviderError>> {
        unimplemented!()
    }
    fn delete(&self, _: &domain::MessageId) -> BoxFuture<'_, Result<(), ProviderError>> {
        unimplemented!()
    }
    fn mark_read(
        &self,
        _: &domain::ConversationId,
        _: &MessageTs,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        unimplemented!()
    }
    fn events(&self) -> BoxStream<'static, domain::ProviderEvent> {
        unimplemented!()
    }
    fn history_between(
        &self,
        _: &domain::ConversationId,
        _: MessageTs,
        _: Option<MessageTs>,
        _: u32,
    ) -> BoxFuture<'_, Result<domain::HistoryPage, ProviderError>> {
        unimplemented!()
    }
    fn replies(
        &self,
        _: &domain::ConversationId,
        _: &MessageTs,
        _: Option<MessageTs>,
        _: u32,
    ) -> BoxFuture<'_, Result<domain::HistoryPage, ProviderError>> {
        unimplemented!()
    }
    fn reply(
        &self,
        _: &domain::ConversationId,
        _: &MessageTs,
        _: &str,
        _: bool,
        _: domain::ClientMsgId,
    ) -> BoxFuture<'_, Result<domain::Message, ProviderError>> {
        unimplemented!()
    }
    fn react(
        &self,
        _: &domain::MessageId,
        _: &domain::EmojiRef,
        _: bool,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        unimplemented!()
    }
    fn mark_thread_read(
        &self,
        _: &domain::ConversationId,
        _: &MessageTs,
        _: &MessageTs,
    ) -> BoxFuture<'_, Result<(), ProviderError>> {
        unimplemented!()
    }
    fn counts(&self) -> BoxFuture<'_, Result<Vec<domain::ConversationCounts>, ProviderError>> {
        unimplemented!()
    }
    fn custom_emoji(&self) -> BoxFuture<'_, Result<Vec<domain::CustomEmoji>, ProviderError>> {
        unimplemented!()
    }
    fn fetch(
        &self,
        _: &domain::RemoteUrl,
        _: u64,
    ) -> BoxFuture<'_, Result<Vec<u8>, ProviderError>> {
        unimplemented!()
    }
    fn download(
        &self,
        _: &domain::RemoteUrl,
        _: &std::path::Path,
    ) -> BoxFuture<'_, Result<u64, ProviderError>> {
        unimplemented!()
    }
    fn upload(
        &self,
        _: &domain::ConversationId,
        _: Option<MessageTs>,
        _: Vec<domain::Upload>,
        _: Option<&str>,
        _: domain::ClientMsgId,
    ) -> BoxFuture<'_, Result<domain::Message, ProviderError>> {
        unimplemented!()
    }
    fn web_url(&self, _: &domain::MessageId) -> Option<String> {
        None
    }
}

#[test]
fn default_methods_are_unsupported() {
    let p = Minimal(
        settings::WorkspaceKey::new("min", "w"),
        Capabilities::default(),
    );
    let q = SearchQuery::default();
    assert_eq!(
        block_on(p.search_messages(&q, None, 10)).err(),
        Some(ProviderError::Unsupported)
    );
    assert_eq!(
        block_on(p.snooze(5)).err(),
        Some(ProviderError::Unsupported)
    );
    assert_eq!(p.conversation_web_url(&cid("c1")), None);
}

fn fake20(caps: Capabilities) -> FakeProvider {
    let f = FakeProvider::new("w1", uid("me")).with_capabilities(caps);
    f.add_conversation(conv("c1", "general"));
    for i in 1..=20 {
        f.add_message(msg("c1", &format!("{i}.000000"), &format!("m{i}")));
    }
    f
}

#[test]
fn default_history_around_combines_both_sides() {
    let f = fake20(Capabilities::default());
    let page = block_on(f.history_around(&cid("c1"), &ts("10.000000"), 6)).unwrap();
    let got: Vec<String> = page.messages.iter().map(|m| m.text.to_string()).collect();
    assert_eq!(got, ["m8", "m9", "m10", "m11", "m12", "m13"]);
    assert!(page.has_more_before);
    assert!(page.has_more_after);
    // Near the newest end nothing more follows.
    let page = block_on(f.history_around(&cid("c1"), &ts("19.000000"), 6)).unwrap();
    assert!(!page.has_more_after);
    assert_eq!(page.messages.last().unwrap().text.as_ref(), "m20");
}

#[test]
fn fake_records_m3_calls() {
    let f = fake20(Capabilities {
        scheduled_send: true,
        dnd: true,
        notify_prefs: true,
        ..Default::default()
    });
    let s = block_on(f.schedule(&cid("c1"), None, "later", 1_800_000_000)).unwrap();
    assert_eq!(block_on(f.scheduled()).unwrap(), vec![s]);
    let d = block_on(f.snooze(30)).unwrap();
    assert!(d.snooze_until.is_some());
    let prefs = ConversationPrefs {
        level: NotifyLevel::Mentions,
        muted: false,
    };
    block_on(f.set_notify_prefs(&cid("c1"), prefs)).unwrap();
    let calls = f.calls();
    assert!(matches!(
        calls[0],
        Call::Schedule {
            post_at: 1_800_000_000,
            ..
        }
    ));
    assert_eq!(calls[1], Call::Snooze(30));
    assert!(matches!(calls[2], Call::SetNotifyPrefs { .. }));
}

#[test]
fn fake_honours_capabilities_and_injected_errors() {
    let f = fake20(Capabilities::default());
    assert_eq!(
        block_on(f.snooze(5)).err(),
        Some(ProviderError::Unsupported)
    );
    let f = fake20(Capabilities {
        dnd: true,
        ..Default::default()
    });
    f.fail_next(Op::Snooze, ProviderError::RateLimited);
    assert_eq!(
        block_on(f.snooze(5)).err(),
        Some(ProviderError::RateLimited)
    );
    assert!(block_on(f.snooze(5)).is_ok());
}
