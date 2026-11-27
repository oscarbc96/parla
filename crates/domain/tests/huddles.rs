use std::sync::Arc;
use std::time::{Duration, Instant};

use domain::test_support::{cid, conv, uid};
use domain::{
    Bootstrap, Changed, ConversationKind, DndStatus, Effect, Huddle, HuddleId, Input, Load, Loaded,
    ProviderCall, ProviderEvent, RING_TIMEOUT, RingEnd, WorkspaceState, reduce,
};
use settings::WorkspaceKey;

fn state() -> (WorkspaceState, Instant) {
    let mut s = WorkspaceState::new(WorkspaceKey::new("fake", "w1"), 200);
    let mut dm = conv("D1", "dm");
    dm.kind = ConversationKind::Direct;
    reduce(
        &mut s,
        Input::Bootstrapped(Bootstrap {
            self_user: uid("me"),
            workspace_name: Arc::from("W"),
            conversations: vec![conv("C1", "general"), dm],
            members: vec![],
        }),
    );
    s.caps.huddles = true;
    let t0 = Instant::now();
    reduce(
        &mut s,
        Input::Clock {
            unix: 1000,
            mono: t0,
        },
    );
    (s, t0)
}

fn hid(s: &str) -> HuddleId {
    HuddleId(Arc::from(s))
}
fn huddle(c: &str, id: &str, who: &[&str]) -> Huddle {
    Huddle {
        id: hid(id),
        conversation: cid(c),
        participants: who.iter().map(|u| uid(u)).collect(),
        started: 900,
        created_by: Some(uid("other")),
        is_dm: c.starts_with('D'),
    }
}
fn ev(s: &mut WorkspaceState, e: ProviderEvent) -> Vec<Effect> {
    reduce(s, Input::Provider(e))
}
fn ring_ev(c: &str) -> ProviderEvent {
    ProviderEvent::HuddleRing {
        conversation: cid(c),
        huddle: hid("H1"),
        from: uid("other"),
    }
}
fn starts(fx: &[Effect]) -> usize {
    fx.iter()
        .filter(|e| matches!(e, Effect::StartRing(_)))
        .count()
}
fn stops(fx: &[Effect]) -> Vec<RingEnd> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::StopRing { reason, .. } => Some(*reason),
            _ => None,
        })
        .collect()
}

#[test]
fn channel_huddle_upserts_and_ends() {
    let (mut s, _) = state();
    let fx = ev(
        &mut s,
        ProviderEvent::HuddleUpdated(huddle("C1", "H1", &["a"])),
    );
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Huddles)))
    );
    ev(
        &mut s,
        ProviderEvent::HuddleUpdated(huddle("C1", "H1", &["a", "b"])),
    );
    assert_eq!(s.huddles[&cid("C1")].participants.len(), 2);
    ev(
        &mut s,
        ProviderEvent::HuddleEnded {
            conversation: cid("C1"),
            id: hid("H1"),
            ended: 1000,
        },
    );
    assert!(s.huddles.is_empty());
}

#[test]
fn dm_ring_starts_once() {
    let (mut s, _) = state();
    assert_eq!(starts(&ev(&mut s, ring_ev("D1"))), 1);
    assert!(s.ring.is_some());
    assert_eq!(starts(&ev(&mut s, ring_ev("D1"))), 0);
}

#[test]
fn channel_invite_never_rings() {
    let (mut s, _) = state();
    assert_eq!(starts(&ev(&mut s, ring_ev("C1"))), 0);
    assert!(s.ring.is_none());
}

#[test]
fn ring_stops_on_every_exit() {
    type Act = Box<dyn Fn(&mut WorkspaceState, Instant) -> Vec<Effect>>;
    let cases: Vec<(RingEnd, Act)> = vec![
        (
            RingEnd::Answered,
            Box::new(|s, _| reduce(s, Input::JoinHuddle(cid("D1")))),
        ),
        (
            RingEnd::AnsweredElsewhere,
            Box::new(|s, _| {
                ev(
                    s,
                    ProviderEvent::HuddleUpdated(huddle("D1", "H1", &["other", "me"])),
                )
            }),
        ),
        (
            RingEnd::Declined,
            Box::new(|s, _| reduce(s, Input::DeclineRing)),
        ),
        (
            RingEnd::Cancelled,
            Box::new(|s, _| {
                ev(
                    s,
                    ProviderEvent::HuddleRingCancelled {
                        conversation: cid("D1"),
                    },
                )
            }),
        ),
        (
            RingEnd::Ended,
            Box::new(|s, _| {
                ev(
                    s,
                    ProviderEvent::HuddleEnded {
                        conversation: cid("D1"),
                        id: hid("H1"),
                        ended: 1001,
                    },
                )
            }),
        ),
        (
            RingEnd::TimedOut,
            Box::new(|s, t0| {
                reduce(
                    s,
                    Input::RingTimeout {
                        now: t0 + RING_TIMEOUT,
                    },
                )
            }),
        ),
        (
            RingEnd::Dnd,
            Box::new(|s, _| {
                ev(
                    s,
                    ProviderEvent::DndChanged {
                        user: uid("me"),
                        status: DndStatus {
                            enabled: false,
                            next_start: None,
                            next_end: None,
                            snooze_until: Some(5000),
                        },
                    },
                )
            }),
        ),
    ];
    for (want, act) in cases {
        let (mut s, t0) = state();
        ev(&mut s, ring_ev("D1"));
        let fx = act(&mut s, t0);
        assert_eq!(stops(&fx), vec![want], "{want:?}");
        assert!(s.ring.is_none(), "{want:?}");
    }
}

#[test]
fn early_timeout_keeps_ring() {
    let (mut s, t0) = state();
    ev(&mut s, ring_ev("D1"));
    let fx = reduce(
        &mut s,
        Input::RingTimeout {
            now: t0 + RING_TIMEOUT - Duration::from_secs(1),
        },
    );
    assert!(stops(&fx).is_empty() && s.ring.is_some());
}

#[test]
fn ring_respects_setting_and_dnd() {
    let (mut s, _) = state();
    reduce(&mut s, Input::SetRingEnabled(false));
    assert_eq!(starts(&ev(&mut s, ring_ev("D1"))), 0);
    reduce(&mut s, Input::SetRingEnabled(true));
    ev(
        &mut s,
        ProviderEvent::DndChanged {
            user: uid("me"),
            status: DndStatus {
                enabled: false,
                next_start: None,
                next_end: None,
                snooze_until: Some(5000),
            },
        },
    );
    assert_eq!(starts(&ev(&mut s, ring_ev("D1"))), 0);
}

#[test]
fn ring_skips_huddle_self_is_in() {
    let (mut s, _) = state();
    ev(
        &mut s,
        ProviderEvent::HuddleUpdated(huddle("D1", "H1", &["other", "me"])),
    );
    assert_eq!(starts(&ev(&mut s, ring_ev("D1"))), 0);
}

#[test]
fn reconnect_never_rings() {
    let (mut s, t0) = state();
    ev(&mut s, ProviderEvent::Connected { reconnect: true });
    assert_eq!(starts(&ev(&mut s, ring_ev("D1"))), 0);
    // Still inside the 5 s window.
    reduce(
        &mut s,
        Input::Clock {
            unix: 1004,
            mono: t0,
        },
    );
    assert_eq!(starts(&ev(&mut s, ring_ev("D1"))), 0);
    // Past it: armed.
    reduce(
        &mut s,
        Input::Clock {
            unix: 1005,
            mono: t0,
        },
    );
    assert_eq!(starts(&ev(&mut s, ring_ev("D1"))), 1);
}

#[test]
fn reconnect_rearms_on_huddle_reload() {
    let (mut s, _) = state();
    ev(&mut s, ProviderEvent::Connected { reconnect: true });
    reduce(&mut s, Input::Loaded(Ok(Loaded::Huddles(vec![]))));
    assert_eq!(starts(&ev(&mut s, ring_ev("D1"))), 1);
}

#[test]
fn reload_ends_ring_of_vanished_huddle() {
    let (mut s, _) = state();
    ev(&mut s, ring_ev("D1"));
    let fx = reduce(&mut s, Input::Loaded(Ok(Loaded::Huddles(vec![]))));
    assert_eq!(stops(&fx), vec![RingEnd::Ended]);
}

#[test]
fn connect_loads_huddles_when_supported() {
    let (mut s, _) = state();
    s.caps.huddles = false;
    let fx = ev(&mut s, ProviderEvent::Connected { reconnect: false });
    assert!(!fx.iter().any(|e| matches!(e, Effect::Load(Load::Huddles))));
    s.caps.huddles = true;
    let fx = ev(&mut s, ProviderEvent::Connected { reconnect: false });
    assert!(fx.iter().any(|e| matches!(e, Effect::Load(Load::Huddles))));
}

#[test]
fn decline_sends_call() {
    let (mut s, _) = state();
    ev(&mut s, ring_ev("D1"));
    let fx = reduce(&mut s, Input::DeclineRing);
    assert!(fx.iter().any(|e| matches!(
        e,
        Effect::Call(ProviderCall::DeclineHuddle { conversation, id })
            if *conversation == cid("D1") && *id == hid("H1")
    )));
}

#[test]
fn join_opens_huddle() {
    let (mut s, _) = state();
    ev(
        &mut s,
        ProviderEvent::HuddleUpdated(huddle("C1", "H1", &["a"])),
    );
    let fx = reduce(&mut s, Input::JoinHuddle(cid("C1")));
    assert!(
        fx.iter().any(
            |e| matches!(e, Effect::OpenHuddle { conversation } if *conversation == cid("C1"))
        )
    );
    let fx = reduce(&mut s, Input::JoinHuddle(cid("D1")));
    assert!(!fx.iter().any(|e| matches!(e, Effect::OpenHuddle { .. })));
}

#[test]
fn in_huddle_set_tracks_users() {
    let (mut s, _) = state();
    ev(
        &mut s,
        ProviderEvent::InHuddle {
            user: uid("a"),
            active: true,
        },
    );
    assert!(s.in_huddle.contains(&uid("a")));
    ev(
        &mut s,
        ProviderEvent::InHuddle {
            user: uid("a"),
            active: false,
        },
    );
    assert!(s.in_huddle.is_empty());
}

#[test]
fn stale_end_leaves_live_ring() {
    let (mut s, _) = state();
    ev(&mut s, ring_ev("D1"));
    let fx = ev(
        &mut s,
        ProviderEvent::HuddleEnded {
            conversation: cid("D1"),
            id: hid("OLD"),
            ended: 900,
        },
    );
    assert!(stops(&fx).is_empty() && s.ring.is_some());
}

#[test]
fn reload_drops_huddles_missing_from_the_list() {
    let (mut s, _) = state();
    reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::Huddles(vec![
            huddle("C1", "H1", &["a"]),
            huddle("C2", "H2", &["c"]),
        ]))),
    );
    assert!(s.in_huddle.contains(&uid("c")));
    let fx = reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::Huddles(vec![huddle("C1", "H1", &["a"])]))),
    );
    assert!(!s.huddles.contains_key(&cid("C2")));
    assert!(!s.in_huddle.contains(&uid("c")));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Huddles)))
    );
}

#[test]
fn live_update_during_reload_survives_the_stale_answer() {
    let (mut s, _) = state();
    let fx = ev(&mut s, ProviderEvent::Connected { reconnect: true });
    assert!(
        fx.iter().any(|e| matches!(e, Effect::Load(Load::Huddles))),
        "connected must reload the huddle list"
    );
    // The update lands after the provider took the snapshot the answer
    // carries, so the answer must not drop it.
    ev(
        &mut s,
        ProviderEvent::HuddleUpdated(huddle("C1", "H1", &["a", "b"])),
    );
    reduce(&mut s, Input::Loaded(Ok(Loaded::Huddles(vec![]))));
    assert_eq!(s.huddles[&cid("C1")].participants.len(), 2);
    assert!(s.in_huddle.contains(&uid("b")));

    // Nor may it overwrite a newer live update with the stale listing.
    ev(&mut s, ProviderEvent::Connected { reconnect: true });
    ev(
        &mut s,
        ProviderEvent::HuddleUpdated(huddle("C1", "H1", &["a", "b", "c"])),
    );
    reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::Huddles(vec![huddle("C1", "H1", &["a"])]))),
    );
    assert_eq!(s.huddles[&cid("C1")].participants.len(), 3);
    assert!(s.in_huddle.contains(&uid("b")) && s.in_huddle.contains(&uid("c")));

    // Once the answer is in, the live state is no longer protected: the
    // next full answer is the truth again.
    reduce(&mut s, Input::Loaded(Ok(Loaded::Huddles(vec![]))));
    assert!(s.huddles.is_empty());
    assert!(!s.in_huddle.contains(&uid("b")));

    // A failed load is an answer too: it must not leave the tracker waiting
    // forever, which would make every later answer stale.
    ev(&mut s, ProviderEvent::Connected { reconnect: true });
    reduce(
        &mut s,
        Input::Loaded(Err((
            Load::Huddles,
            domain::ProviderError::Other("boom".into()),
        ))),
    );
    ev(
        &mut s,
        ProviderEvent::HuddleUpdated(huddle("C1", "H1", &["a"])),
    );
    reduce(&mut s, Input::Loaded(Ok(Loaded::Huddles(vec![]))));
    assert!(s.huddles.is_empty());
}

#[test]
fn reload_rebuilds_in_huddle_from_participants() {
    let (mut s, _) = state();
    ev(
        &mut s,
        ProviderEvent::InHuddle {
            user: uid("x"),
            active: true,
        },
    );
    ev(
        &mut s,
        ProviderEvent::HuddleUpdated(huddle("C1", "H1", &["a", "b"])),
    );
    assert!(s.in_huddle.contains(&uid("a")) && s.in_huddle.contains(&uid("x")));
    ev(
        &mut s,
        ProviderEvent::HuddleUpdated(huddle("C1", "H1", &["a"])),
    );
    assert!(!s.in_huddle.contains(&uid("b")) && s.in_huddle.contains(&uid("a")));
    // A reload lists only one huddle: the other ended while the connection
    // was down, so its huddle and participants go. A user tracked on its own
    // (`x`) stays.
    ev(
        &mut s,
        ProviderEvent::HuddleUpdated(huddle("C2", "H2", &["c"])),
    );
    reduce(
        &mut s,
        Input::Loaded(Ok(Loaded::Huddles(vec![huddle("C1", "H1", &["a", "d"])]))),
    );
    assert!(s.in_huddle.contains(&uid("a")) && s.in_huddle.contains(&uid("d")));
    assert!(!s.huddles.contains_key(&cid("C2")));
    assert!(!s.in_huddle.contains(&uid("c")) && s.in_huddle.contains(&uid("x")));
    assert!(!s.in_huddle.contains(&uid("b")));
    ev(
        &mut s,
        ProviderEvent::HuddleEnded {
            conversation: cid("C1"),
            id: hid("H1"),
            ended: 1,
        },
    );
    assert!(!s.in_huddle.contains(&uid("a")) && !s.in_huddle.contains(&uid("c")));
    assert!(s.in_huddle.contains(&uid("x")));
}

#[test]
fn unchanged_reload_does_not_signal() {
    let (mut s, _) = state();
    let l = || Input::Loaded(Ok(Loaded::Huddles(vec![huddle("C1", "H1", &["a"])])));
    assert!(
        reduce(&mut s, l())
            .iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Huddles)))
    );
    assert!(
        !reduce(&mut s, l())
            .iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Huddles)))
    );
}

#[test]
fn disabling_ring_setting_ends_ring() {
    let (mut s, _) = state();
    ev(&mut s, ring_ev("D1"));
    let fx = reduce(&mut s, Input::SetRingEnabled(false));
    assert_eq!(stops(&fx), vec![RingEnd::Cancelled]);
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Ring)))
    );
    assert!(s.ring.is_none());
}

#[test]
fn clock_past_dnd_start_ends_ring() {
    let (mut s, t0) = state();
    ev(
        &mut s,
        ProviderEvent::DndChanged {
            user: uid("me"),
            status: DndStatus {
                enabled: true,
                next_start: Some(1100),
                next_end: Some(2000),
                snooze_until: None,
            },
        },
    );
    ev(&mut s, ring_ev("D1"));
    assert!(s.ring.is_some());
    let fx = reduce(
        &mut s,
        Input::Clock {
            unix: 1100,
            mono: t0,
        },
    );
    assert_eq!(stops(&fx), vec![RingEnd::Dnd]);
}

#[test]
fn stale_timeout_leaves_newer_ring() {
    let (mut s, t0) = state();
    ev(&mut s, ring_ev("D1"));
    reduce(&mut s, Input::DeclineRing);
    let later = t0 + Duration::from_secs(100);
    reduce(
        &mut s,
        Input::Clock {
            unix: 1100,
            mono: later,
        },
    );
    // A new huddle (the declined one would stay quiet).
    ev(
        &mut s,
        ProviderEvent::HuddleRing {
            conversation: cid("D1"),
            huddle: hid("H2"),
            from: uid("other"),
        },
    );
    // The first ring's timer fires at t0 + RING_TIMEOUT: the new ring is 45 s from over.
    let fx = reduce(
        &mut s,
        Input::RingTimeout {
            now: t0 + RING_TIMEOUT,
        },
    );
    assert!(stops(&fx).is_empty() && s.ring.is_some());
}

#[test]
fn ring_from_self_is_ignored() {
    let (mut s, _) = state();
    let fx = ev(
        &mut s,
        ProviderEvent::HuddleRing {
            conversation: cid("D1"),
            huddle: hid("H1"),
            from: uid("me"),
        },
    );
    assert_eq!(starts(&fx), 0);
}

#[test]
fn declined_huddle_does_not_ring_again() {
    let (mut s, _) = state();
    ev(&mut s, ring_ev("D1"));
    reduce(&mut s, Input::DeclineRing);
    // A later update, then a repeated invite frame for the same huddle.
    ev(
        &mut s,
        ProviderEvent::HuddleUpdated(huddle("D1", "H1", &["other", "third"])),
    );
    let fx = ev(&mut s, ring_ev("D1"));
    assert_eq!(starts(&fx), 0);
    assert!(s.ring.is_none());
    // A different huddle in the same conversation rings.
    let fx = ev(
        &mut s,
        ProviderEvent::HuddleRing {
            conversation: cid("D1"),
            huddle: hid("H2"),
            from: uid("other"),
        },
    );
    assert_eq!(starts(&fx), 1);
}

#[test]
fn timed_out_huddle_does_not_ring_again() {
    let (mut s, t0) = state();
    ev(&mut s, ring_ev("D1"));
    reduce(
        &mut s,
        Input::RingTimeout {
            now: t0 + RING_TIMEOUT,
        },
    );
    assert!(s.ring.is_none());
    ev(
        &mut s,
        ProviderEvent::HuddleUpdated(huddle("D1", "H1", &["other", "third"])),
    );
    assert_eq!(starts(&ev(&mut s, ring_ev("D1"))), 0);
}

#[test]
fn dismissed_memory_is_bounded_and_cleared_by_the_end() {
    let (mut s, _) = state();
    for i in 0..40 {
        let id = format!("H{i}");
        ev(
            &mut s,
            ProviderEvent::HuddleRing {
                conversation: cid("D1"),
                huddle: hid(&id),
                from: uid("other"),
            },
        );
        reduce(&mut s, Input::DeclineRing);
    }
    assert!(s.dismissed_rings() <= 16);
    ev(
        &mut s,
        ProviderEvent::HuddleEnded {
            conversation: cid("D1"),
            id: hid("H39"),
            ended: 1001,
        },
    );
    let fx = ev(
        &mut s,
        ProviderEvent::HuddleRing {
            conversation: cid("D1"),
            huddle: hid("H39"),
            from: uid("other"),
        },
    );
    assert_eq!(starts(&fx), 1, "an ended huddle's memory is dropped");
}

#[test]
fn ring_needs_the_huddles_capability() {
    let (mut s, _) = state();
    s.caps.huddles = false;
    let fx = ev(&mut s, ring_ev("D1"));
    assert_eq!(starts(&fx), 0);
    assert!(s.ring.is_none());
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::Changed(Changed::Ring))),
        "no ring state is emitted"
    );
}

#[test]
fn in_huddle_event_is_advisory() {
    let (mut s, _) = state();
    ev(
        &mut s,
        ProviderEvent::HuddleUpdated(huddle("C1", "H1", &["a"])),
    );
    ev(&mut s, ring_ev("D1"));
    for active in [true, false, true] {
        let fx = ev(
            &mut s,
            ProviderEvent::InHuddle {
                user: uid("me"),
                active,
            },
        );
        assert_eq!(starts(&fx), 0);
        assert!(stops(&fx).is_empty());
    }
    assert!(s.ring.is_some(), "the ring is untouched");
    assert!(
        s.huddles.contains_key(&cid("C1")),
        "the huddle is untouched"
    );
    ev(
        &mut s,
        ProviderEvent::InHuddle {
            user: uid("zed"),
            active: true,
        },
    );
    assert_eq!(s.huddles.len(), 1, "no huddle is created");
}
