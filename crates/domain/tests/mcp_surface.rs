//! The provider calls the M4 MCP tools need. M3 already added every role, so
//! this file pins the mapping (role to M3 name) and checks the fake serves it:
//!
//! - remote search: `ChatProvider::search_messages` (`Page<MessageHit>`)
//! - member lookup: `ChatProvider::members` (by id), `search_people` (text)
//! - profile fields: `ChatProvider::profile` -> `Profile { title, tz, status }`
//! - scheduled: `ScheduledMessage`, `ChatProvider::{scheduled, schedule}`
//! - status: `ChatProvider::set_status(Option<Status>)`

use std::sync::Arc;

use domain::test_support::{Call, FakeProvider, cid, member, msg, uid};
use domain::{
    Capabilities, ChatProvider, HitSource, MessageHit, Profile, ProviderError, ScheduledMessage,
    SearchQuery, Status,
};
use futures::executor::block_on;

fn fake(caps: Capabilities) -> FakeProvider {
    FakeProvider::new("w1", uid("me")).with_capabilities(caps)
}

fn hit(t: &str, text: &str) -> MessageHit {
    MessageHit {
        message: msg("c1", t, text),
        snippet: Arc::from(text),
        source: HitSource::Remote,
    }
}

#[test]
fn m3_provides_remote_search() {
    let f = fake(Capabilities {
        search: true,
        ..Capabilities::default()
    });
    f.set_search_hits(vec![
        hit("3.000000", "c"),
        hit("2.000000", "b"),
        hit("1.000000", "a"),
    ]);
    let q = SearchQuery {
        text: Arc::from("x"),
        ..SearchQuery::default()
    };
    let page = block_on(f.search_messages(&q, None, 2)).unwrap();
    let ts: Vec<_> = page.items.iter().map(|h| h.message.id.ts.clone()).collect();
    assert_eq!(ts.len(), 2);
    assert!(ts[0] > ts[1], "newest first as scripted");
}

#[test]
fn m3_provides_member_lookup_and_search() {
    let f = fake(Capabilities {
        search: true,
        member_lookup: true,
        ..Capabilities::default()
    });
    f.add_member(member("u2", "bo"));
    f.add_lookup_member(member("u3", "cy"));
    let found = block_on(f.members(&[uid("u3"), uid("nobody")])).unwrap();
    assert_eq!(found.len(), 1, "unknown ids are left out");
    assert_eq!(found[0].id, uid("u3"));
    let by_text = block_on(f.search_people("bo", 5)).unwrap();
    assert_eq!(by_text.len(), 1);
    assert_eq!(by_text[0].id, uid("u2"));
}

#[test]
fn m3_provides_profile_fields() {
    let f = fake(Capabilities {
        status: true,
        ..Capabilities::default()
    });
    let status = Status {
        emoji: None,
        text: Arc::from("out"),
        expires: Some(10),
    };
    f.set_profile(Profile {
        user: uid("u2"),
        display_name: Arc::from("bo"),
        real_name: Arc::from("Bo"),
        title: Some(Arc::from("Engineer")),
        pronouns: None,
        tz: Some(Arc::from("Europe/Madrid")),
        status: Some(status.clone()),
        email: None,
        phone: None,
        fields: vec![],
        image: None,
        is_bot: false,
        deleted: false,
    });
    let p = block_on(f.profile(&uid("u2"))).unwrap();
    assert_eq!(p.title.as_deref(), Some("Engineer"));
    assert_eq!(p.tz.as_deref(), Some("Europe/Madrid"));
    assert_eq!(p.status, Some(status));
}

#[test]
fn m3_provides_scheduled_listing_and_schedule() {
    let f = fake(Capabilities {
        scheduled_send: true,
        ..Capabilities::default()
    });
    let s: ScheduledMessage = block_on(f.schedule(&cid("c1"), None, "later", 1_000)).unwrap();
    assert_eq!(&*s.text, "later");
    assert_eq!(s.post_at, 1_000);
    let all = block_on(f.scheduled()).unwrap();
    assert_eq!(all, vec![s]);
    assert!(matches!(
        f.calls()[0],
        Call::Schedule { post_at: 1_000, .. }
    ));
}

#[test]
fn m3_provides_set_status_logged() {
    let f = fake(Capabilities {
        status: true,
        ..Capabilities::default()
    });
    let st = Status {
        emoji: None,
        text: Arc::from("focus"),
        expires: None,
    };
    block_on(f.set_status(Some(st.clone()))).unwrap();
    block_on(f.set_status(None)).unwrap();
    assert_eq!(
        f.calls(),
        vec![Call::SetStatus(Some(st)), Call::SetStatus(None)]
    );
}

#[test]
fn unsupported_without_capability() {
    let f = fake(Capabilities::default());
    assert!(matches!(
        block_on(f.scheduled()),
        Err(ProviderError::Unsupported)
    ));
    assert!(matches!(
        block_on(f.set_status(None)),
        Err(ProviderError::Unsupported)
    ));
}

#[test]
fn provider_stays_object_safe() {
    fn _f(_: Arc<dyn ChatProvider>) {}
}
