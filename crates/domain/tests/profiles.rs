//! profiles, the bounded profile cache, own status and presence.

use std::sync::Arc;

use domain::test_support::{member, uid};
use domain::{
    CallOutput, Changed, Effect, Input, Load, Loaded, PROFILE_CACHE, PresenceSetting, Profile,
    ProviderCall, ProviderError, ProviderEvent, Status, WorkspaceState, local_time, reduce,
    status_visible,
};
use settings::WorkspaceKey;

const NOW: i64 = 1_700_000_000;

fn state() -> WorkspaceState {
    let mut st = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 100);
    st.self_user = Some(uid("u1"));
    st.members.insert(uid("u1"), member("u1", "Me"));
    st
}

fn profile(u: &str) -> Profile {
    Profile {
        user: uid(u),
        display_name: Arc::from(u),
        real_name: Arc::from(u),
        title: None,
        pronouns: None,
        tz: None,
        status: None,
        email: None,
        phone: None,
        fields: vec![],
        image: None,
        is_bot: false,
        deleted: false,
    }
}

fn status(text: &str, expires: Option<i64>) -> Status {
    Status {
        emoji: None,
        text: Arc::from(text),
        expires,
    }
}

fn loads_profile(fx: &[Effect], u: &str) -> bool {
    fx.iter()
        .any(|e| matches!(e, Effect::Load(Load::Profile(x)) if *x == uid(u)))
}
fn loaded(st: &mut WorkspaceState, p: Profile) -> Vec<Effect> {
    reduce(st, Input::Loaded(Ok(Loaded::Profile(p))))
}
fn fail(call: ProviderCall) -> Input {
    Input::CallFinished {
        call,
        result: Err(ProviderError::Other("offline".into())),
    }
}
fn ok(call: ProviderCall) -> Input {
    Input::CallFinished {
        call,
        result: Ok(CallOutput::None),
    }
}

#[test]
fn open_profile_loads_once() {
    let mut st = state();
    let fx = reduce(&mut st, Input::OpenProfile(uid("u2")));
    assert!(loads_profile(&fx, "u2"));
    // No DND read without the capability.
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::Load(Load::DndUsers(_))))
    );
    loaded(&mut st, profile("u2"));
    let fx = reduce(&mut st, Input::OpenProfile(uid("u2")));
    assert!(!loads_profile(&fx, "u2"));
    // With the capability, DND for that user is read too.
    st.caps.dnd = true;
    let fx = reduce(&mut st, Input::OpenProfile(uid("u2")));
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Load(Load::DndUsers(v)) if v.as_slice() == [uid("u2")]
    )));
}

#[test]
fn cache_is_bounded() {
    let mut st = state();
    for i in 0..PROFILE_CACHE + 5 {
        loaded(&mut st, profile(&format!("p{i}")));
    }
    assert_eq!(st.profiles.len(), PROFILE_CACHE);
    // The oldest loaded were dropped first.
    assert!(!st.profiles.contains_key(&uid("p0")));
    assert!(
        st.profiles
            .contains_key(&uid(&format!("p{}", PROFILE_CACHE + 4)))
    );
    // Reloading refreshes, it does not grow the cache.
    loaded(&mut st, profile("p10"));
    assert_eq!(st.profiles.len(), PROFILE_CACHE);
}

#[test]
fn set_status_optimistic_and_refetch() {
    let mut st = state();
    loaded(&mut st, profile("u1"));
    let s = status("lunch", None);
    let fx = reduce(&mut st, Input::SetStatus(Some(s.clone())));
    assert_eq!(st.members[&uid("u1")].status, Some(s.clone()));
    assert_eq!(st.profiles[&uid("u1")].status, Some(s.clone()));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Members)))
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Profile(u)) if *u == uid("u1")))
    );
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Call(ProviderCall::SetStatus(Some(x))) if *x == s
    )));
    // The same status again changes nothing visible but is still sent.
    let fx = reduce(&mut st, Input::SetStatus(Some(s.clone())));
    assert!(!fx.iter().any(|e| matches!(e, Effect::Changed(_))));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Call(ProviderCall::SetStatus(_))))
    );
    // One answer in, one failure: the last answer refetches the truth.
    let fx = reduce(&mut st, ok(ProviderCall::SetStatus(Some(s.clone()))));
    assert!(!loads_profile(&fx, "u1"));
    let fx = reduce(&mut st, fail(ProviderCall::SetStatus(Some(s))));
    assert!(loads_profile(&fx, "u1"));
    // The refetched truth replaces the optimistic status everywhere.
    let fx = loaded(&mut st, profile("u1"));
    assert_eq!(st.members[&uid("u1")].status, None);
    assert_eq!(st.profiles[&uid("u1")].status, None);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Members)))
    );
}

#[test]
fn status_success_needs_no_refetch_and_clear_works() {
    let mut st = state();
    st.members.get_mut(&uid("u1")).unwrap().status = Some(status("old", None));
    let fx = reduce(&mut st, Input::SetStatus(None));
    assert_eq!(st.members[&uid("u1")].status, None);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Call(ProviderCall::SetStatus(None))))
    );
    let fx = reduce(&mut st, ok(ProviderCall::SetStatus(None)));
    assert!(fx.is_empty());
}

#[test]
fn a_member_update_without_a_picture_keeps_the_known_one() {
    let mut st = state();
    let pic = domain::ImageRef::Web("https://x.test/a.png".into());
    let mut with = member("u2", "Bot");
    with.avatar = domain::Avatar::new(Some(pic.clone()), None);
    reduce(&mut st, Input::Provider(ProviderEvent::MemberUpdated(with)));
    reduce(
        &mut st,
        Input::Provider(ProviderEvent::MemberUpdated(member("u2", "Bot renamed"))),
    );
    let m = &st.members[&uid("u2")];
    assert_eq!(&*m.display_name, "Bot renamed");
    assert_eq!(m.avatar_for(36.0), Some(&pic));
    // A new picture replaces it.
    let mut newer = member("u2", "Bot renamed");
    let n = domain::ImageRef::Web("https://x.test/b.png".into());
    newer.avatar = domain::Avatar::new(Some(n.clone()), None);
    reduce(
        &mut st,
        Input::Provider(ProviderEvent::MemberUpdated(newer)),
    );
    assert_eq!(st.members[&uid("u2")].avatar_for(36.0), Some(&n));
}

#[test]
fn avatar_for_picks_the_large_picture_from_56_px() {
    let mut m = member("u1", "A");
    assert_eq!(m.avatar_for(36.0), None);
    let (s, l) = (
        domain::ImageRef::Web("s".into()),
        domain::ImageRef::Web("l".into()),
    );
    m.avatar = domain::Avatar::new(Some(s.clone()), None);
    assert_eq!(m.avatar_for(72.0), Some(&s), "one size serves both");
    m.avatar = domain::Avatar::new(Some(s.clone()), Some(l.clone()));
    assert_eq!(m.avatar_for(36.0), Some(&s));
    assert_eq!(m.avatar_for(55.9), Some(&s));
    assert_eq!(m.avatar_for(56.0), Some(&l));
    assert_eq!(m.avatar_for(72.0), Some(&l));
    assert!(domain::Avatar::new(None, None).is_none());
}

#[test]
fn status_in_flight_survives_stale_loads_and_member_updates() {
    let mut st = state();
    let s = status("focus", None);
    reduce(&mut st, Input::SetStatus(Some(s.clone())));
    // A snapshot that predates the write must not undo it.
    loaded(&mut st, profile("u1"));
    assert_eq!(st.members[&uid("u1")].status, Some(s.clone()));
    reduce(
        &mut st,
        Input::Provider(ProviderEvent::MemberUpdated(member("u1", "Me"))),
    );
    assert_eq!(st.members[&uid("u1")].status, Some(s));
}

#[test]
fn set_presence_optimistic_and_unknown_after_failure() {
    let mut st = state();
    let fx = reduce(&mut st, Input::SetPresence(PresenceSetting::Away));
    assert_eq!(st.presence_setting, Some(PresenceSetting::Away));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Profile(u)) if *u == uid("u1")))
    );
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Call(ProviderCall::SetPresence(PresenceSetting::Away))
    )));
    // Same value again: no change notification.
    let fx = reduce(&mut st, Input::SetPresence(PresenceSetting::Away));
    assert!(!fx.iter().any(|e| matches!(e, Effect::Changed(_))));
    reduce(
        &mut st,
        ok(ProviderCall::SetPresence(PresenceSetting::Away)),
    );
    assert_eq!(st.presence_setting, Some(PresenceSetting::Away));
    // The last answer fails: there is nothing to read the truth from, so
    // the setting is unknown again.
    let fx = reduce(
        &mut st,
        fail(ProviderCall::SetPresence(PresenceSetting::Away)),
    );
    assert_eq!(st.presence_setting, None);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Profile(_))))
    );
}

#[test]
fn expired_status_hidden() {
    assert!(status_visible(&status("a", None), NOW));
    assert!(status_visible(&status("a", Some(NOW + 1)), NOW));
    assert!(!status_visible(&status("a", Some(NOW)), NOW));
    assert!(!status_visible(&status("a", Some(NOW - 5)), NOW));
}

#[test]
fn profile_changed_reloads_only_open() {
    let mut st = state();
    loaded(&mut st, profile("u2"));
    loaded(&mut st, profile("u3"));
    reduce(&mut st, Input::OpenProfile(uid("u2")));
    // Not open: the cached copy is dropped, nothing is loaded.
    let fx = reduce(
        &mut st,
        Input::Provider(ProviderEvent::ProfileChanged(uid("u3"))),
    );
    assert!(!st.profiles.contains_key(&uid("u3")));
    assert!(!loads_profile(&fx, "u3"));
    // Open: dropped and reloaded.
    let fx = reduce(
        &mut st,
        Input::Provider(ProviderEvent::ProfileChanged(uid("u2"))),
    );
    assert!(!st.profiles.contains_key(&uid("u2")));
    assert!(loads_profile(&fx, "u2"));
    // Closed again: no reload.
    reduce(&mut st, Input::CloseProfile);
    loaded(&mut st, profile("u2"));
    let fx = reduce(
        &mut st,
        Input::Provider(ProviderEvent::ProfileChanged(uid("u2"))),
    );
    assert!(!loads_profile(&fx, "u2"));
}

#[test]
fn member_updated_copies_status_into_cached_profile() {
    let mut st = state();
    loaded(&mut st, profile("u2"));
    let mut m = member("u2", "Ann");
    m.status = Some(status("away", None));
    let fx = reduce(&mut st, Input::Provider(ProviderEvent::MemberUpdated(m)));
    assert_eq!(st.profiles[&uid("u2")].status, Some(status("away", None)));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Profile(u)) if *u == uid("u2")))
    );
}

#[test]
fn local_time_uses_iana_zone() {
    let now = jiff::Timestamp::from_second(NOW).unwrap();
    let mut p = profile("u2");
    assert!(local_time(&p, now).is_none());
    p.tz = Some(Arc::from("Not/AZone"));
    assert!(local_time(&p, now).is_none());
    p.tz = Some(Arc::from("Asia/Tokyo"));
    let z = local_time(&p, now).unwrap();
    // 2023-11-14T22:13:20Z is 07:13 the next day in Tokyo (UTC+9).
    assert_eq!((z.day(), z.hour(), z.minute()), (15, 7, 13));
}

#[test]
fn open_profile_survives_eviction() {
    let mut st = state();
    reduce(&mut st, Input::OpenProfile(uid("p0")));
    for i in 0..PROFILE_CACHE + 5 {
        loaded(&mut st, profile(&format!("p{i}")));
    }
    assert_eq!(st.profiles.len(), PROFILE_CACHE);
    assert!(st.profiles.contains_key(&uid("p0")));
    // The next oldest went instead.
    assert!(!st.profiles.contains_key(&uid("p1")));
}

#[test]
fn stale_load_keeps_in_flight_status_without_a_self_member() {
    let mut st = state();
    st.members.remove(&uid("u1"));
    loaded(&mut st, profile("u1"));
    let s = status("focus", None);
    reduce(&mut st, Input::SetStatus(Some(s.clone())));
    assert_eq!(st.profiles[&uid("u1")].status, Some(s.clone()));
    loaded(&mut st, profile("u1"));
    assert_eq!(st.profiles[&uid("u1")].status, Some(s));
}

#[test]
fn status_answers_out_of_order_refetch_once_at_the_end() {
    let mut st = state();
    let a = status("a", None);
    let b = status("b", None);
    reduce(&mut st, Input::SetStatus(Some(a.clone())));
    reduce(&mut st, Input::SetStatus(Some(b.clone())));
    // The newer write fails first: not the last answer, no refetch yet.
    let fx = reduce(&mut st, fail(ProviderCall::SetStatus(Some(b.clone()))));
    assert!(!loads_profile(&fx, "u1"));
    // The older write succeeds last: the failure still forces a refetch.
    let fx = reduce(&mut st, ok(ProviderCall::SetStatus(Some(a))));
    assert!(loads_profile(&fx, "u1"));
    assert_eq!(st.members[&uid("u1")].status, Some(b));
}

/// Opening a direct message loads the peer's profile once (for the header's
/// local time); a cached profile, a channel or a self DM loads nothing.
#[test]
fn opening_a_direct_message_loads_the_peers_profile_once() {
    let mut st = state();
    let mut d = domain::test_support::conv("D1", "ana");
    d.kind = domain::ConversationKind::Direct;
    d.peer = Some(uid("u2"));
    st.conversations.insert(d.id.clone(), d.clone());
    let fx = reduce(&mut st, Input::Open(d.id.clone()));
    assert!(loads_profile(&fx, "u2"), "{fx:?}");
    loaded(&mut st, profile("u2"));
    let fx = reduce(&mut st, Input::Open(d.id.clone()));
    assert!(!loads_profile(&fx, "u2"), "cached: {fx:?}");
    let c = domain::test_support::conv("C1", "general");
    st.conversations.insert(c.id.clone(), c.clone());
    let fx = reduce(&mut st, Input::Open(c.id));
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::Load(Load::Profile(_))))
    );
    let mut own = domain::test_support::conv("D2", "me");
    own.kind = domain::ConversationKind::Direct;
    own.peer = Some(uid("u1"));
    st.conversations.insert(own.id.clone(), own.clone());
    let fx = reduce(&mut st, Input::Open(own.id));
    assert!(!loads_profile(&fx, "u1"), "self: {fx:?}");
}
