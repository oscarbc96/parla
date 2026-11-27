//! M3 model part 2: huddles, call cards, saved items, accounts, composed text.

use std::sync::Arc;

use domain::test_support::{Call, FakeFactory, FakeProvider, cid, conv, mid, msg, ts, uid};
use domain::{
    AccountInfo, Block, BookmarkId, CallCard, CallKind, Capabilities, ChatProvider, ComposedSpan,
    ComposedText, ContentEncoder, EmojiRef, Huddle, HuddleId, InboxThread, Inline, Page,
    PlainEncoder, ProviderError, ProviderFactory, RichText, SavedItem,
};
use futures::executor::block_on;
use settings::WorkspaceKey;

fn card(kind: CallKind) -> RichText {
    RichText {
        blocks: Arc::from(vec![Block::Call(CallCard {
            kind,
            title: None,
            join_url: None,
            started: None,
            ended: None,
            participants: Arc::from([]),
            created_by: None,
        })]),
    }
}

#[test]
fn call_card_plain_text() {
    assert_eq!(card(CallKind::Huddle).plain_text(), "Huddle");
    let ext = CallKind::External {
        provider: Arc::from("Meet"),
    };
    assert_eq!(card(ext).plain_text(), "Call: Meet");
}

#[test]
fn message_new_not_pinned_or_saved() {
    let m = msg("C1", "1.000001", "hi");
    assert!(!m.pinned && !m.saved);
}

fn user_span(range: std::ops::Range<usize>) -> ComposedSpan {
    ComposedSpan {
        range,
        inline: Inline::User {
            id: uid("U1"),
            label: None,
        },
    }
}

#[test]
fn plain_encoder_uses_labels() {
    let t = ComposedText::new("hi @ana", vec![user_span(3..7)]);
    assert_eq!(PlainEncoder.encode(&t), "hi @ana");
    assert_eq!(PlainEncoder.encode(&ComposedText::plain("x")), "x");
}

#[test]
#[should_panic]
#[cfg(debug_assertions)]
fn composed_span_ranges_are_char_boundaries() {
    // "é" is two bytes; 1..2 splits it.
    let _ = ComposedText::new("é", vec![user_span(1..2)]);
}

#[test]
fn default_account_info_from_accounts() {
    let p = Arc::new(FakeProvider::new("W1", uid("U1")));
    let f = FakeFactory::default().with_provider(p);
    let info = f.account_info();
    assert_eq!(info.len(), 1);
    assert_eq!(info[0].key, WorkspaceKey::new("fake", "W1"));
    assert_eq!(info[0].name.as_ref(), "W1");
    assert!(info[0].org.is_none() && info[0].icon.is_none());
}

#[test]
fn fake_factory_scripts_accounts_and_remove() {
    let f = FakeFactory::default();
    let key = WorkspaceKey::new("fake", "W9");
    f.set_accounts(vec![AccountInfo {
        key: key.clone(),
        name: Arc::from("Nine"),
        org: None,
        icon: None,
    }]);
    assert_eq!(f.accounts(), vec![key.clone()]);
    assert_eq!(f.account_info()[0].name.as_ref(), "Nine");
    block_on(f.remove(&key)).unwrap();
    assert!(f.accounts().is_empty());
    assert_eq!(f.removed(), vec![key]);
    let _ = f.encoder().encode(&ComposedText::plain("ok"));
}

fn all_caps() -> Capabilities {
    Capabilities {
        huddles: true,
        pins: true,
        bookmarks: true,
        saved: true,
        thread_inbox: true,
        ..Default::default()
    }
}

#[test]
fn fake_scripts_huddles_and_pins() {
    block_on(fake_scripts_huddles_and_pins_async());
}

async fn fake_scripts_huddles_and_pins_async() {
    let p = FakeProvider::new("W1", uid("U1")).with_capabilities(all_caps());
    p.add_conversation(conv("C1", "general"));
    p.add_message(msg("C1", "1.000001", "a"));
    let h = Huddle {
        id: HuddleId(Arc::from("H1")),
        conversation: cid("C1"),
        participants: Arc::from([uid("U2")]),
        started: 100,
        created_by: Some(uid("U2")),
        is_dm: false,
    };
    p.set_huddles(vec![h.clone()]);
    assert_eq!(p.huddles().await.unwrap().len(), 1);
    p.decline_huddle(&cid("C1"), &h.id).await.unwrap();

    let id = mid("C1", "1.000001");
    p.set_pinned(&id, true).await.unwrap();
    assert_eq!(p.pins(&cid("C1")).await.unwrap()[0].message, id);
    assert!(p.message(&id).await.unwrap().pinned);
    p.set_pinned(&id, false).await.unwrap();
    assert!(p.pins(&cid("C1")).await.unwrap().is_empty());

    let b = p
        .add_bookmark(&cid("C1"), "Docs", "https://example.com/d")
        .await
        .unwrap();
    assert_eq!(p.bookmarks(&cid("C1")).await.unwrap().len(), 1);
    p.remove_bookmark(&cid("C1"), &b.id).await.unwrap();
    assert!(p.bookmarks(&cid("C1")).await.unwrap().is_empty());

    p.set_saved(&id, true).await.unwrap();
    let page = p.saved(None).await.unwrap();
    assert_eq!(page.items.len(), 1);
    p.set_saved(&id, false).await.unwrap();
    assert!(p.saved(None).await.unwrap().items.is_empty());

    p.set_thread_inbox(vec![InboxThread {
        root: id.clone(),
        unread_replies: 2,
        mentions: 1,
        latest: ts("2.000000"),
    }]);
    assert_eq!(
        p.thread_inbox(None).await.unwrap().items[0].unread_replies,
        2
    );

    let calls = p.calls();
    assert!(matches!(&calls[0], Call::DeclineHuddle { .. }));
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, Call::SetPinned { pinned: true, .. }))
    );
    assert!(calls.iter().any(|c| matches!(c, Call::AddBookmark { .. })));
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, Call::RemoveBookmark { .. }))
    );
    assert!(
        calls
            .iter()
            .any(|c| matches!(c, Call::SetSaved { saved: true, .. }))
    );
}

#[test]
fn m3b_methods_unsupported_without_caps() {
    block_on(m3b_unsupported_async());
}

async fn m3b_unsupported_async() {
    let p = FakeProvider::new("W1", uid("U1"));
    assert_eq!(p.huddles().await.unwrap_err(), ProviderError::Unsupported);
    assert_eq!(p.saved(None).await.unwrap_err(), ProviderError::Unsupported);
    assert!(p.huddle_url(&cid("C1")).is_none());
}

#[test]
fn urls_default_none_and_fake_gives_one_with_huddles() {
    let p = FakeProvider::new("W1", uid("U1")).with_capabilities(all_caps());
    assert!(p.huddle_url(&cid("C1")).unwrap().contains("C1"));
    assert!(p.app_url(&cid("C1")).is_some());
}

#[test]
fn factory_stays_object_safe() {
    fn _f(_: Arc<dyn ProviderFactory>, _: Arc<dyn ContentEncoder>) {}
    let _ = (
        BookmarkId(Arc::from("B")),
        SavedItem {
            message: mid("C", "1.0"),
            saved_at: 0,
            due: None,
            done: false,
        },
        Page::<SavedItem> {
            items: vec![],
            next: None,
        },
        EmojiRef {
            shortcode: Arc::from("x"),
            unicode: None,
            skin_tone: None,
        },
    );
}
