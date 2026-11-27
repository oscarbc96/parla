//! Channel admin: create, join, leave, archive, rename, topic, description and
//! invite go out as one provider call each; their answers update the list.

use std::sync::Arc;

use domain::test_support::{FakeProvider, conv, member, uid};
use domain::{
    AdminCall, CallOutput, Capabilities, Changed, ChatProvider, ConversationId, ConversationKind,
    ConversationMeta, Effect, Input, InviteReport, Load, Loaded, NameProblem, PersistOp,
    ProviderCall, ProviderError, ProviderEvent, WorkspaceState, reduce,
};
use settings::WorkspaceKey;

fn cid(s: &str) -> ConversationId {
    ConversationId(Arc::from(s))
}

fn state() -> WorkspaceState {
    let mut st = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 100);
    st.self_user = Some(uid("u1"));
    st.members.insert(uid("u1"), member("u1", "Me"));
    st.caps = Capabilities {
        channel_admin: true,
        ..Default::default()
    };
    st.conversations.insert(cid("c1"), conv("c1", "general"));
    st
}

fn call_of(fx: &[Effect]) -> Option<AdminCall> {
    fx.iter().find_map(|e| match e {
        Effect::Call(ProviderCall::Admin(a)) => Some(a.clone()),
        _ => None,
    })
}

fn finished(st: &mut WorkspaceState, call: AdminCall, out: CallOutput) -> Vec<Effect> {
    reduce(
        st,
        Input::CallFinished {
            call: ProviderCall::Admin(call),
            result: Ok(out),
        },
    )
}

fn changed(fx: &[Effect]) -> bool {
    fx.iter()
        .any(|e| matches!(e, Effect::Changed(Changed::Conversations)))
}

#[test]
fn an_admin_input_becomes_one_provider_call() {
    let mut st = state();
    let op = AdminCall::Create {
        name: "launch-plan".into(),
        private: true,
    };
    assert_eq!(
        call_of(&reduce(&mut st, Input::Admin(op.clone()))),
        Some(op)
    );
    let leave = AdminCall::Leave(cid("c1"));
    assert_eq!(
        call_of(&reduce(&mut st, Input::Admin(leave.clone()))),
        Some(leave)
    );
    // Without the capability nothing goes out.
    let mut off = state();
    off.caps.channel_admin = false;
    assert!(
        call_of(&reduce(
            &mut off,
            Input::Admin(AdminCall::Archive(cid("c1")))
        ))
        .is_none()
    );
}

#[test]
fn a_created_channel_joins_the_list_as_a_member() {
    let mut st = state();
    let mut made = conv("c9", "launch-plan");
    made.kind = ConversationKind::PrivateChannel;
    made.is_member = false;
    let fx = finished(
        &mut st,
        AdminCall::Create {
            name: "launch-plan".into(),
            private: true,
        },
        CallOutput::Conversation(made),
    );
    let c = st.conversations.get(&cid("c9")).expect("inserted");
    assert!(c.is_member && c.is_open);
    assert_eq!(c.kind, ConversationKind::PrivateChannel);
    assert!(changed(&fx));
    assert!(fx.iter().any(
        |e| matches!(e, Effect::Persist(PersistOp::Conversations(v)) if v[0].id == cid("c9"))
    ));
}

#[test]
fn joining_marks_a_known_channel_as_a_member() {
    let mut st = state();
    let mut other = conv("c2", "random");
    other.is_member = false;
    st.conversations.insert(cid("c2"), other.clone());
    let fx = finished(
        &mut st,
        AdminCall::Join(cid("c2")),
        CallOutput::Conversation(other),
    );
    assert!(st.conversations[&cid("c2")].is_member);
    assert!(changed(&fx));
}

#[test]
fn leave_archive_rename_topic_and_description_apply_on_success() {
    let mut st = state();
    finished(
        &mut st,
        AdminCall::Rename {
            conversation: cid("c1"),
            name: "town-square".into(),
        },
        CallOutput::None,
    );
    finished(
        &mut st,
        AdminCall::SetTopic {
            conversation: cid("c1"),
            topic: "Weekly notes".into(),
        },
        CallOutput::None,
    );
    let fx = finished(
        &mut st,
        AdminCall::SetPurpose {
            conversation: cid("c1"),
            purpose: "Company wide news".into(),
        },
        CallOutput::None,
    );
    assert!(changed(&fx));
    let c = &st.conversations[&cid("c1")];
    assert_eq!(&*c.name, "town-square");
    assert_eq!(&*c.topic, "Weekly notes");
    assert_eq!(&*c.purpose, "Company wide news");
    finished(&mut st, AdminCall::Archive(cid("c1")), CallOutput::None);
    assert!(st.conversations[&cid("c1")].is_archived);
    finished(&mut st, AdminCall::Leave(cid("c1")), CallOutput::None);
    assert!(!st.conversations[&cid("c1")].is_member);
}

#[test]
fn a_failed_call_changes_nothing() {
    let mut st = state();
    let fx = reduce(
        &mut st,
        Input::CallFinished {
            call: ProviderCall::Admin(AdminCall::Leave(cid("c1"))),
            result: Err(ProviderError::NotAllowed("no".into())),
        },
    );
    assert!(st.conversations[&cid("c1")].is_member);
    assert!(!changed(&fx));
}

#[test]
fn a_live_description_change_updates_the_conversation() {
    let mut st = state();
    let fx = reduce(
        &mut st,
        Input::Provider(ProviderEvent::ConversationMetaChanged(ConversationMeta {
            id: cid("c1"),
            purpose: Some("About us".into()),
            ..ConversationMeta::new(cid("c1"))
        })),
    );
    assert_eq!(&*st.conversations[&cid("c1")].purpose, "About us");
    assert!(changed(&fx));
}

fn loads(fx: &[Effect]) -> Vec<ConversationId> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Load(Load::Conversation(c)) => Some(c.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn another_member_joining_or_leaving_updates_the_list_and_count() {
    let mut st = state();
    let mut group = conv("g1", "design");
    group.kind = ConversationKind::Group;
    group.member_ids = Arc::from([uid("u1"), uid("u2")]);
    group.member_count = Some(2);
    st.conversations.insert(cid("g1"), group);

    let fx = reduce(
        &mut st,
        Input::Provider(ProviderEvent::ConversationMemberChanged {
            conversation: cid("g1"),
            user: uid("u3"),
            joined: true,
        }),
    );
    let c = &st.conversations[&cid("g1")];
    assert_eq!(c.member_ids.to_vec(), [uid("u1"), uid("u2"), uid("u3")]);
    assert_eq!(c.member_count, Some(3));
    assert!(changed(&fx));
    assert!(fx.iter().any(
        |e| matches!(e, Effect::Persist(PersistOp::Conversations(v)) if v[0].id == cid("g1"))
    ));

    // A channel with no member list held keeps none; its count follows.
    st.conversations.get_mut(&cid("c1")).unwrap().member_count = Some(10);
    let fx = reduce(
        &mut st,
        Input::Provider(ProviderEvent::ConversationMemberChanged {
            conversation: cid("c1"),
            user: uid("u4"),
            joined: true,
        }),
    );
    let c = &st.conversations[&cid("c1")];
    assert!(c.member_ids.is_empty());
    assert_eq!(c.member_count, Some(11));
    assert!(changed(&fx));

    // Leaving is the reverse, and someone who was not in the held list
    // does not change it or the count.
    reduce(
        &mut st,
        Input::Provider(ProviderEvent::ConversationMemberChanged {
            conversation: cid("g1"),
            user: uid("u2"),
            joined: false,
        }),
    );
    let c = &st.conversations[&cid("g1")];
    assert_eq!(c.member_ids.to_vec(), [uid("u1"), uid("u3")]);
    assert_eq!(c.member_count, Some(2));
    let fx = reduce(
        &mut st,
        Input::Provider(ProviderEvent::ConversationMemberChanged {
            conversation: cid("g1"),
            user: uid("u9"),
            joined: false,
        }),
    );
    assert_eq!(st.conversations[&cid("g1")].member_count, Some(2));
    assert!(!changed(&fx));

    // An unknown conversation is not held, but the event is a reason to ask
    // for it when the provider can answer.
    let fx = reduce(
        &mut st,
        Input::Provider(ProviderEvent::ConversationMemberChanged {
            conversation: cid("c8"),
            user: uid("u3"),
            joined: true,
        }),
    );
    assert!(!st.conversations.contains_key(&cid("c8")));
    assert!(!changed(&fx));
    st.caps.conversation_lookup = true;
    let fx = reduce(
        &mut st,
        Input::Provider(ProviderEvent::ConversationMemberChanged {
            conversation: cid("c8"),
            user: uid("u3"),
            joined: true,
        }),
    );
    assert_eq!(loads(&fx), [cid("c8")]);
}

#[test]
fn a_change_of_an_unknown_conversation_asks_for_it_once() {
    let mut st = state();
    let change = |st: &mut WorkspaceState| {
        reduce(
            st,
            Input::Provider(ProviderEvent::ConversationMetaChanged(ConversationMeta {
                id: cid("c9"),
                name: Some("launch-day".into()),
                is_member: Some(true),
                ..ConversationMeta::new(cid("c9"))
            })),
        )
    };
    // Without the capability nothing is asked for (as before).
    assert!(loads(&change(&mut st)).is_empty());

    st.caps.conversation_lookup = true;
    let fx = change(&mut st);
    assert_eq!(loads(&fx), [cid("c9")]);
    assert!(!st.conversations.contains_key(&cid("c9")));
    // One request at a time per id; a second change only updates what it says.
    assert!(loads(&change(&mut st)).is_empty());
    // Leaving or archiving a channel not held needs no fetch.
    let left = |st: &mut WorkspaceState, member| {
        reduce(
            st,
            Input::Provider(ProviderEvent::ConversationMetaChanged(ConversationMeta {
                id: cid("c6"),
                is_member: member,
                ..ConversationMeta::new(cid("c6"))
            })),
        )
    };
    assert!(loads(&left(&mut st, Some(false))).is_empty());
    let archived = reduce(
        &mut st,
        Input::Provider(ProviderEvent::ConversationMetaChanged(ConversationMeta {
            id: cid("c5"),
            is_archived: Some(true),
            ..ConversationMeta::new(cid("c5"))
        })),
    );
    assert!(loads(&archived).is_empty());

    // The answer adds it, with the event's membership and the provider's details.
    let mut fetched = conv("c9", "launch-day");
    fetched.member_count = Some(12);
    fetched.is_member = false;
    let fx = reduce(&mut st, Input::Loaded(Ok(Loaded::Conversation(fetched))));
    let held = &st.conversations[&cid("c9")];
    assert!(held.is_member);
    assert_eq!(held.member_count, Some(12));
    assert!(changed(&fx));
    assert!(fx.iter().any(
        |e| matches!(e, Effect::Persist(PersistOp::Conversations(v)) if v[0].id == cid("c9"))
    ));

    // A failed request is forgotten, so a later change asks again.
    let other = ProviderEvent::ConversationMetaChanged(ConversationMeta::new(cid("c7")));
    assert_eq!(loads(&reduce(&mut st, Input::Provider(other))), [cid("c7")]);
    let fx = reduce(
        &mut st,
        Input::Loaded(Err((
            Load::Conversation(cid("c7")),
            ProviderError::Network("down".into()),
        ))),
    );
    assert!(loads(&fx).is_empty());
    let other = ProviderEvent::ConversationMetaChanged(ConversationMeta::new(cid("c7")));
    assert_eq!(loads(&reduce(&mut st, Input::Provider(other))), [cid("c7")]);
}

#[test]
fn a_fetched_conversation_merges_into_a_held_one() {
    let mut st = state();
    st.caps.conversation_lookup = true;
    let mut held = st.conversations[&cid("c1")].clone();
    held.is_muted = true;
    held.unread = true;
    held.mentions = 3;
    st.conversations.insert(cid("c1"), held);
    let mut fetched = conv("c1", "town-square");
    fetched.topic = "Fresh topic".into();
    fetched.member_count = Some(7);
    fetched.is_member = false;
    reduce(&mut st, Input::Loaded(Ok(Loaded::Conversation(fetched))));
    let c = &st.conversations[&cid("c1")];
    assert!(c.is_muted && c.unread && c.is_member);
    assert_eq!(c.mentions, 3);
    assert_eq!(&*c.name, "town-square");
    assert_eq!(&*c.topic, "Fresh topic");
    assert_eq!(c.member_count, Some(7));
}

#[test]
fn a_known_channel_keeps_its_mute_and_unread_state_on_join() {
    let mut st = state();
    let mut known = conv("c2", "random");
    known.is_member = false;
    known.is_muted = true;
    known.unread = true;
    known.mentions = 2;
    st.conversations.insert(cid("c2"), known);
    let mut answer = conv("c2", "random");
    answer.topic = "Fresh topic".into();
    answer.member_count = Some(9);
    finished(
        &mut st,
        AdminCall::Join(cid("c2")),
        CallOutput::Conversation(answer),
    );
    let c = &st.conversations[&cid("c2")];
    assert!(c.is_member && c.is_muted && c.unread);
    assert_eq!(c.mentions, 2);
    assert_eq!(&*c.topic, "Fresh topic");
    assert_eq!(c.member_count, Some(9));
}

#[test]
fn an_invite_counts_only_the_people_added() {
    let mut st = state();
    st.conversations.get_mut(&cid("c1")).unwrap().member_count = Some(3);
    finished(
        &mut st,
        AdminCall::Invite {
            conversation: cid("c1"),
            users: vec![uid("u2"), uid("u3")],
        },
        CallOutput::Invite(InviteReport {
            added: vec![uid("u2")],
            failed: vec![(
                uid("u3"),
                ProviderError::NotAllowed("already_in_channel".into()),
            )],
        }),
    );
    assert_eq!(st.conversations[&cid("c1")].member_count, Some(4));
}

#[test]
fn the_fake_provider_has_simple_name_rules() {
    let p = fake();
    let n = p.check_channel_name("  Launch Plan ");
    assert_eq!((n.name.as_str(), n.problem), ("launch-plan", None));
    assert_eq!(p.check_channel_name(" ").problem, Some(NameProblem::Empty));
    assert_eq!(
        p.check_channel_name("a.b").problem,
        Some(NameProblem::Characters)
    );
}

#[test]
fn the_fake_provider_lists_members_and_reports_invites_per_person() {
    let p = fake();
    p.set_channel_members(&cid("c1"), &[uid("u1"), uid("u2")]);
    let m = futures::executor::block_on(p.conversation_members(&cid("c1"))).unwrap();
    assert_eq!(m, [uid("u1"), uid("u2")]);
    let r = futures::executor::block_on(p.invite(&cid("c1"), &[uid("u2"), uid("u3")])).unwrap();
    assert_eq!(r.added, [uid("u3")]);
    assert_eq!(r.failed.len(), 1);
    assert_eq!(r.failed[0].0, uid("u2"));
}

fn fake() -> FakeProvider {
    let p = FakeProvider::new("w1", uid("u1")).with_capabilities(Capabilities {
        channel_admin: true,
        ..Default::default()
    });
    p.add_conversation(conv("c1", "general"));
    let mut other = conv("c2", "design-crit");
    other.is_member = false;
    other.purpose = "Show work early".into();
    p.add_conversation(other);
    p
}

#[test]
fn the_fake_provider_creates_browses_and_joins() {
    let p = fake();
    let made = futures::executor::block_on(p.create_channel("launch-plan", true)).unwrap();
    assert_eq!(made.kind, ConversationKind::PrivateChannel);
    assert!(made.is_member);
    let page = futures::executor::block_on(p.browse_channels("design", None, 20)).unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(&*page.items[0].purpose, "Show work early");
    let joined = futures::executor::block_on(p.join_conversation(&cid("c2"))).unwrap();
    assert!(joined.is_member);
    futures::executor::block_on(p.leave_conversation(&cid("c2"))).unwrap();
    futures::executor::block_on(p.rename_conversation(&cid("c1"), "town-square")).unwrap();
    futures::executor::block_on(p.invite(&cid("c1"), &[uid("u2")])).unwrap();
    // An empty query lists every channel, members and not.
    let all = futures::executor::block_on(p.browse_channels("", None, 20)).unwrap();
    assert_eq!(all.items.len(), 3);
    assert!(all.items.iter().any(|c| &*c.name == "town-square"));
    assert!(!all.items.iter().any(|c| c.id == cid("c2") && c.is_member));
}

#[test]
fn the_fake_provider_refuses_without_the_capability() {
    let p = FakeProvider::new("w1", uid("u1"));
    let r = futures::executor::block_on(p.create_channel("x", false));
    assert!(matches!(r, Err(ProviderError::Unsupported)));
}

#[test]
fn a_star_is_also_sent_to_a_provider_that_keeps_stars() {
    let mut st = state();
    let star = |st: &mut WorkspaceState, on| {
        reduce(
            st,
            Input::SetStarred {
                conversation: cid("c1"),
                starred: on,
            },
        )
    };
    let sent = |fx: &[Effect]| {
        fx.iter().find_map(|e| match e {
            Effect::Call(ProviderCall::SetConversationStarred {
                conversation,
                starred,
            }) => Some((conversation.clone(), *starred)),
            _ => None,
        })
    };
    // Parla's own list only, without the capability.
    assert_eq!(sent(&star(&mut st, true)), None);
    st.caps.conversation_stars = true;
    assert_eq!(sent(&star(&mut st, false)), Some((cid("c1"), false)));
    // No change, no call.
    assert_eq!(sent(&star(&mut st, false)), None);
}

#[test]
fn a_previewed_channel_is_held_as_a_non_member_and_opening_it_marks_nothing_read() {
    let mut st = state();
    let mut found = conv("c9", "design-crit");
    found.is_member = true;
    found.latest = Some(domain::MessageTs("9.000000".into()));
    let fx = reduce(&mut st, Input::Preview(found));
    assert!(changed(&fx));
    let held = &st.conversations[&cid("c9")];
    // The record came from a search or browse: never a membership.
    assert!(!held.is_member);
    let fx = reduce(&mut st, Input::Open(cid("c9")));
    assert!(fx.iter().any(|e| matches!(e, Effect::FetchHistory { .. })));
    assert!(
        !fx.iter().any(|e| matches!(e, Effect::MarkRead { .. })),
        "{fx:?}"
    );
    // A known conversation keeps its own record.
    let mut again = conv("c1", "renamed");
    again.is_member = false;
    reduce(&mut st, Input::Preview(again));
    assert!(st.conversations[&cid("c1")].is_member);
    assert_eq!(&*st.conversations[&cid("c1")].name, "general");
}

#[test]
fn the_fake_names_a_conversation_link() {
    let p = FakeProvider::new("w1", uid("u1"));
    assert_eq!(
        p.conversation_web_url(&cid("C1")).as_deref(),
        Some("https://fake.example.test/C1")
    );
}

#[test]
fn a_previewed_channel_stays_out_of_the_unread_totals() {
    let mut st = state();
    let before = st.totals();
    let mut found = conv("c9", "design-crit");
    found.unread = true;
    found.mentions = 2;
    reduce(&mut st, Input::Preview(found));
    assert_eq!(st.totals(), before);
    // A message in it while it is shown: still nothing counted.
    let mut m = domain::test_support::msg("c9", "10.000000", "hi");
    m.author = Some(uid("u2"));
    m.mentions = Arc::from([domain::Mention::User(uid("u1"))]);
    reduce(&mut st, Input::Provider(ProviderEvent::MessagePosted(m)));
    assert_eq!(st.totals(), before);
}
