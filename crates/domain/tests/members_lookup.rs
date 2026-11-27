//! Unknown authors and repliers are looked up on demand: one batched,
//! bounded request, no repeats, failures remembered.

use std::sync::Arc;

use domain::test_support::{cid, conv, member, mid, uid};
use domain::{
    Bootstrap, Capabilities, Changed, Effect, HistoryPage, Input, Load, Loaded,
    MEMBER_LOOKUP_BATCH, Message, ProviderError, UserId, WorkspaceState, reduce,
};
use settings::WorkspaceKey;

fn state(lookup: bool) -> WorkspaceState {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    s.caps = Capabilities {
        member_lookup: lookup,
        ..Capabilities::default()
    };
    reduce(
        &mut s,
        Input::Bootstrapped(Bootstrap {
            self_user: uid("me"),
            workspace_name: Arc::from("W"),
            conversations: vec![conv("c1", "general")],
            members: vec![member("known", "Known Person")],
        }),
    );
    s
}

fn by(t: &str, author: &str) -> Message {
    Message::new(mid("c1", t), Some(uid(author)), Arc::from("hi"))
}

fn history(s: &mut WorkspaceState, messages: Vec<Message>) -> Vec<Effect> {
    reduce(
        s,
        Input::HistoryLoaded {
            conversation: cid("c1"),
            page: HistoryPage {
                messages,
                has_more: false,
            },
        },
    )
}

fn loads(fx: &[Effect]) -> Vec<Vec<UserId>> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Load(Load::Members(ids)) => Some(ids.clone()),
            _ => None,
        })
        .collect()
}

fn answer(s: &mut WorkspaceState, asked: &[&str], found: &[&str]) -> Vec<Effect> {
    reduce(
        s,
        Input::Loaded(Ok(Loaded::Members {
            asked: asked.iter().map(|u| uid(u)).collect(),
            found: found
                .iter()
                .map(|u| member(u, &format!("Name {u}")))
                .collect(),
        })),
    )
}

#[test]
fn unknown_authors_and_repliers_emit_one_batched_load() {
    let mut s = state(true);
    let mut root = by("1.1", "u2");
    root.reply_users = Arc::from([uid("u3"), uid("known")]);
    let fx = history(
        &mut s,
        vec![root, by("1.2", "u2"), by("1.3", "u4"), by("1.4", "known")],
    );
    assert_eq!(loads(&fx), vec![vec![uid("u2"), uid("u3"), uid("u4")]]);
}

#[test]
fn nothing_is_asked_without_the_capability() {
    let mut s = state(false);
    let fx = history(&mut s, vec![by("1.1", "u2")]);
    assert!(loads(&fx).is_empty());
}

#[test]
fn resolved_member_is_stored_and_announced() {
    let mut s = state(true);
    history(&mut s, vec![by("1.1", "u2")]);
    let fx = answer(&mut s, &["u2"], &["u2"]);
    assert_eq!(&*s.members[&uid("u2")].display_name, "Name u2");
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Members)))
    );
    assert!(loads(&fx).is_empty());
}

#[test]
fn ids_in_flight_are_not_asked_again() {
    let mut s = state(true);
    assert_eq!(loads(&history(&mut s, vec![by("1.1", "u2")])).len(), 1);
    let fx = history(&mut s, vec![by("1.2", "u2")]);
    assert!(loads(&fx).is_empty());
}

#[test]
fn ids_not_found_are_remembered_as_failed() {
    let mut s = state(true);
    history(&mut s, vec![by("1.1", "u2")]);
    answer(&mut s, &["u2"], &[]);
    let fx = history(&mut s, vec![by("1.2", "u2")]);
    assert!(loads(&fx).is_empty());
}

#[test]
fn provider_errors_remember_failure_except_transient_ones() {
    let mut s = state(true);
    history(&mut s, vec![by("1.1", "u2"), by("1.2", "u3")]);
    reduce(
        &mut s,
        Input::Loaded(Err((
            Load::Members(vec![uid("u2"), uid("u3")]),
            ProviderError::Other("nope".into()),
        ))),
    );
    assert!(loads(&history(&mut s, vec![by("1.3", "u2")])).is_empty());

    let mut s = state(true);
    history(&mut s, vec![by("1.1", "u2")]);
    reduce(
        &mut s,
        Input::Loaded(Err((
            Load::Members(vec![uid("u2")]),
            ProviderError::RateLimited,
        ))),
    );
    assert_eq!(loads(&history(&mut s, vec![by("1.2", "u2")])).len(), 1);
}

#[test]
fn batches_are_bounded_and_continue_after_each_answer() {
    let mut s = state(true);
    let n = MEMBER_LOOKUP_BATCH * 2 + 5;
    let msgs = (0..n)
        .map(|i| by(&format!("1.{i:04}"), &format!("u{i:04}")))
        .collect();
    let first = loads(&history(&mut s, msgs));
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].len(), MEMBER_LOOKUP_BATCH);
    let ids: Vec<&str> = first[0].iter().map(|u| &*u.0).collect();
    let second = loads(&answer(&mut s, &ids, &ids));
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].len(), MEMBER_LOOKUP_BATCH);
    assert!(second[0].iter().all(|u| !first[0].contains(u)));
}

#[test]
fn a_member_that_arrives_meanwhile_is_not_overwritten() {
    let mut s = state(true);
    history(&mut s, vec![by("1.1", "u2")]);
    s.members.insert(uid("u2"), member("u2", "Streamed"));
    answer(&mut s, &["u2"], &["u2"]);
    assert_eq!(&*s.members[&uid("u2")].display_name, "Streamed");
}

fn dm(id: &str, peer: &str, open: bool) -> domain::Conversation {
    let mut c = conv(id, peer);
    c.kind = domain::ConversationKind::Direct;
    c.peer = Some(uid(peer));
    c.is_open = open;
    c
}

#[test]
fn peers_of_open_direct_conversations_are_looked_up_and_closed_ones_are_not() {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    s.caps = Capabilities {
        member_lookup: true,
        ..Capabilities::default()
    };
    let fx = reduce(
        &mut s,
        Input::Bootstrapped(Bootstrap {
            self_user: uid("me"),
            workspace_name: Arc::from("W"),
            conversations: vec![dm("d1", "p1", true), dm("d2", "p2", false)],
            members: vec![],
        }),
    );
    assert_eq!(loads(&fx), vec![vec![uid("p1")]]);
}

#[test]
fn a_new_message_reopens_a_closed_direct_conversation() {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    reduce(
        &mut s,
        Input::Bootstrapped(Bootstrap {
            self_user: uid("me"),
            workspace_name: Arc::from("W"),
            conversations: vec![dm("d2", "p2", false)],
            members: vec![],
        }),
    );
    assert!(!s.conversations[&cid("d2")].is_open);
    reduce(
        &mut s,
        Input::Provider(domain::ProviderEvent::MessagePosted(Message::new(
            mid("d2", "5.1"),
            Some(uid("p2")),
            Arc::from("hi"),
        ))),
    );
    assert!(s.conversations[&cid("d2")].is_open);
}

fn group(id: &str, ids: &[&str], open: bool) -> domain::Conversation {
    let mut c = conv(id, "");
    c.kind = domain::ConversationKind::Group;
    c.is_open = open;
    c.member_ids = ids.iter().map(|u| uid(u)).collect();
    c
}

#[test]
fn members_of_open_groups_are_looked_up_like_authors() {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    s.caps = Capabilities {
        member_lookup: true,
        ..Capabilities::default()
    };
    let fx = reduce(
        &mut s,
        Input::Bootstrapped(Bootstrap {
            self_user: uid("me"),
            workspace_name: Arc::from("W"),
            conversations: vec![
                group("g1", &["me", "known", "p1", "p2"], true),
                group("g2", &["p3"], false),
            ],
            members: vec![member("known", "Known Person"), member("me", "Me")],
        }),
    );
    assert_eq!(loads(&fx), vec![vec![uid("p1"), uid("p2")]]);
}
