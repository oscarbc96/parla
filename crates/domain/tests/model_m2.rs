use std::sync::Arc;

use domain::test_support::{Call, FakeProvider, cid, cmid, conv, msg, ts, uid};
use domain::*;
use futures::executor::block_on;

fn rend(w: u32) -> Rendition {
    Rendition {
        url: RemoteUrl(Arc::from(format!("https://f.example.test/{w}"))),
        width: w,
        height: w,
    }
}

fn file(thumbs: Vec<Rendition>) -> FileRef {
    FileRef {
        id: FileId(Arc::from("F1")),
        name: Arc::from("a.png"),
        title: None,
        mime: Arc::from("image/png"),
        size: 1,
        kind: FileKind::Image,
        thumbs: Arc::from(thumbs),
        download: None,
        preview: None,
        video: None,
        tombstone: false,
        lines: None,
        ext: ProviderExt::none(),
    }
}

#[test]
fn message_new_defaults() {
    let m = Message::new(
        MessageId {
            conversation: cid("c1"),
            ts: ts("1.000000"),
        },
        Some(uid("u1")),
        Arc::from("hi"),
    );
    assert_eq!(&*m.plain, "hi");
    assert!(m.mentions.is_empty() && m.reactions.is_empty() && m.files.is_empty());
    assert!(m.latest_reply.is_none() && m.reply_users.is_empty() && !m.broadcast);
}

#[test]
fn best_thumb_picks_smallest_wide_enough() {
    let f = file(vec![rend(160), rend(360), rend(720)]);
    assert_eq!(f.best_thumb(300).unwrap().width, 360);
    assert_eq!(f.best_thumb(1000).unwrap().width, 720);
    assert!(file(vec![]).best_thumb(10).is_none());
}

#[test]
fn just_before_is_one_micro_less() {
    assert_eq!(
        ts("1700000000.000100").just_before(),
        ts("1700000000.000099")
    );
    assert_eq!(
        ts("1700000001.000000").just_before(),
        ts("1700000000.999999")
    );
}

#[test]
fn no_ext_codec_round_trip_is_none() {
    let c = NoExtCodec;
    assert!(c.encode(ExtKind::Message, &ProviderExt::new(1u8)).is_none());
    assert!(c.decode(ExtKind::Message, b"x").get::<u8>().is_none());
}

#[test]
fn provider_and_factory_stay_object_safe() {
    fn _f(
        _: Arc<dyn ChatProvider>,
        _: Arc<dyn ProviderFactory>,
        _: Arc<dyn ExtCodec>,
        _: Arc<dyn ContentParser>,
    ) {
    }
}

#[test]
fn fake_factory_seams() {
    let f = domain::test_support::FakeFactory::default();
    assert_eq!(f.display_name(), "Fake");
    let m = msg("c1", "1.000000", "hello");
    assert_eq!(f.content_parser().parse(&m), RichText::plain("hello"));
}

#[test]
fn fake_history_between_pages_newest_first() {
    let f = FakeProvider::new("w1", uid("me"));
    f.add_conversation(conv("c1", "general"));
    for i in 1..=14 {
        f.add_message(msg("c1", &format!("{i}.000000"), "x"));
    }
    let c = cid("c1");
    let p = block_on(f.history_between(&c, ts("2.000000"), None, 5)).unwrap();
    let t: Vec<_> = p.messages.iter().map(|m| m.id.ts.0.to_string()).collect();
    assert_eq!(
        t,
        [
            "14.000000",
            "13.000000",
            "12.000000",
            "11.000000",
            "10.000000"
        ]
    );
    assert!(p.has_more);
    let oldest = p.messages.last().unwrap().id.ts.clone();
    let p = block_on(f.history_between(&c, ts("2.000000"), Some(oldest), 5)).unwrap();
    assert_eq!(p.messages.len(), 5);
    assert_eq!(&*p.messages[0].id.ts.0, "9.000000");
    assert!(p.has_more);
    let oldest = p.messages.last().unwrap().id.ts.clone();
    let p = block_on(f.history_between(&c, ts("2.000000"), Some(oldest), 5)).unwrap();
    assert_eq!(p.messages.len(), 2);
    assert!(!p.has_more);
}

#[test]
fn fake_replies_and_reactions_are_logged() {
    let f = FakeProvider::new("w1", uid("me"));
    f.add_conversation(conv("c1", "general"));
    let c = cid("c1");
    let root = ts("1.000000");
    let r = block_on(f.reply(&c, &root, "re", false, cmid("k1"))).unwrap();
    assert_eq!(r.thread_root, Some(root.clone()));
    let em = EmojiRef {
        shortcode: Arc::from("+1"),
        unicode: None,
        skin_tone: None,
    };
    block_on(f.react(&r.id, &em, true)).unwrap();
    assert_eq!(
        f.calls(),
        vec![
            Call::Reply {
                conversation: c,
                root,
                text: "re".into(),
                broadcast: false,
                id: cmid("k1")
            },
            Call::React {
                id: r.id,
                emoji: em,
                add: true
            }
        ]
    );
}
