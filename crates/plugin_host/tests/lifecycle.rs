//! Plugin lifecycle, quarantine, blocklist and the installed set. Time is a fake clock:
//! a fixed `Instant` plus offsets.

use std::sync::Arc;
use std::time::{Duration, Instant};

use plugin_host::lifecycle::{
    BlockEntry, Discovered, Effect, Event, FailureReason, Lifecycle, PluginState, discover,
    transition,
};
use plugin_host::manifest::{Manifest, ManifestSource, parse};
use plugin_host::registry::{ContributionKind, InstalledSet};
use store::PluginRow;

const MANIFEST: &str = r#"
id = "test-plugin"
name = "Test plugin"
version = "0.1.0"
api = "0.1"

[[commands]]
id = "hello"
title = "Say hello"

[[panels]]
id = "board"
title = "Board"
icon = "plugin"
"#;

fn manifest() -> Arc<Manifest> {
    Arc::new(parse(MANIFEST, ManifestSource::Package).unwrap_or_else(|e| panic!("{e}")))
}

struct Clock(Instant);

impl Clock {
    fn at(&self, minutes: u64) -> Instant {
        self.0 + Duration::from_secs(minutes * 60)
    }
}

fn clock() -> Clock {
    Clock(Instant::now())
}

/// Feeds `events` from `start`, returning the final lifecycle and every
/// effect in order.
fn run(start: Lifecycle, events: Vec<(Event, Instant)>) -> (Lifecycle, Vec<Effect>) {
    let mut l = start;
    let mut all = Vec::new();
    for (e, now) in events {
        let (next, effects) = transition(&l, e, now);
        l = next;
        all.extend(effects);
    }
    (l, all)
}

fn active(c: &Clock) -> Lifecycle {
    run(
        Lifecycle::new(PluginState::Verified, true),
        vec![(Event::Installed, c.at(0)), (Event::Activated, c.at(0))],
    )
    .0
}

fn crash(at: Instant) -> Vec<(Event, Instant)> {
    vec![
        (Event::Failed(FailureReason::Crashed), at),
        (Event::Activated, at),
    ]
}

fn block(malicious: bool) -> BlockEntry {
    BlockEntry {
        reason: "Sends messages without asking".into(),
        advisory: None,
        malicious,
    }
}

#[test]
fn three_crashes_quarantine() {
    let c = clock();
    let mut events = crash(c.at(0));
    events.extend(crash(c.at(4)));
    events.push((Event::Failed(FailureReason::MemoryLimit), c.at(9)));
    let (l, effects) = run(active(&c), events);
    assert_eq!(l.state, PluginState::Quarantined);
    assert!(effects.contains(&Effect::RemoveContributions));
    assert!(effects.contains(&Effect::CloseViews));
    assert!(effects.contains(&Effect::Persist(PluginState::Quarantined)));
    assert!(effects.contains(&Effect::ShowStopped {
        reason: FailureReason::MemoryLimit,
        quarantined: true,
    }));
    // The store already ended: no unload is asked for.
    assert!(!effects.contains(&Effect::Unload));
}

#[test]
fn two_crashes_in_eleven_minutes_do_not() {
    let c = clock();
    // Two crashes, and a third eleven minutes after the first: no ten
    // minute window holds three.
    let mut events = crash(c.at(0));
    events.extend(crash(c.at(5)));
    events.push((Event::Failed(FailureReason::Deadline), c.at(11)));
    let (l, effects) = run(active(&c), events);
    assert_eq!(l.state, PluginState::Unloaded);
    assert!(!effects.contains(&Effect::RemoveContributions));
    assert!(effects.contains(&Effect::ShowStopped {
        reason: FailureReason::Deadline,
        quarantined: false,
    }));
    // Two crashes alone never quarantine either.
    let (l, _) = run(active(&c), [crash(c.at(0)), crash(c.at(1))].concat());
    assert_eq!(l.state, PluginState::Active);
}

#[test]
fn restart_clears_quarantine() {
    let c = clock();
    let quarantined = run(
        active(&c),
        [crash(c.at(0)), crash(c.at(1)), crash(c.at(2))].concat(),
    )
    .0;
    assert_eq!(quarantined.state, PluginState::Quarantined);
    let (l, effects) = transition(&quarantined, Event::Restart, c.at(3));
    assert_eq!(l.state, PluginState::Installed);
    assert_eq!(
        effects,
        vec![
            Effect::RegisterContributions,
            Effect::Persist(PluginState::Installed)
        ]
    );
    // The window is cleared: one more crash does not re-quarantine.
    let (l, _) = run(
        l,
        [vec![(Event::Activated, c.at(4))], crash(c.at(4))].concat(),
    );
    assert_eq!(l.state, PluginState::Active);
    // Updating also clears it.
    let (l, _) = transition(&quarantined, Event::Updated, c.at(3));
    assert_eq!(l.state, PluginState::Installed);
}

#[test]
fn enabled_setting_cannot_revive_blocked() {
    let c = clock();
    let (blocked, effects) = transition(&active(&c), Event::Blocked(block(false)), c.at(0));
    assert_eq!(blocked.state, PluginState::Blocked);
    assert!(effects.contains(&Effect::Unload));
    assert!(effects.contains(&Effect::RemoveContributions));
    assert!(effects.contains(&Effect::ShowBlocked(block(false))));
    for e in [
        Event::Disabled,
        Event::Enabled,
        Event::Restart,
        Event::Updated,
    ] {
        let (l, effects) = transition(&blocked, e, c.at(1));
        assert_eq!(l.state, PluginState::Blocked);
        assert!(!effects.contains(&Effect::RegisterContributions));
    }
    // Quarantine too: the setting alone never revives it.
    let quarantined = run(
        active(&c),
        [crash(c.at(0)), crash(c.at(1)), crash(c.at(2))].concat(),
    )
    .0;
    let (l, effects) = transition(&quarantined, Event::Enabled, c.at(3));
    assert_eq!(l.state, PluginState::Quarantined);
    assert!(effects.is_empty());
    // Unblocking follows the setting the user left.
    let off = transition(&blocked, Event::Disabled, c.at(1)).0;
    assert_eq!(
        transition(&off, Event::Unblocked, c.at(2)).0.state,
        PluginState::Disabled
    );
    assert_eq!(
        transition(&blocked, Event::Unblocked, c.at(2)).0.state,
        PluginState::Installed
    );
}

#[test]
fn disable_removes_contributions() {
    let c = clock();
    let m = manifest();
    let mut set = InstalledSet::default();
    set.insert(m.clone());
    let (l, effects) = transition(
        &Lifecycle::new(PluginState::Verified, true),
        Event::Installed,
        c.at(0),
    );
    set.apply(&m.id, &effects);
    assert_eq!(l.state, PluginState::Installed);
    assert_eq!(set.contributions(ContributionKind::Command).count(), 1);
    assert_eq!(set.contributions(ContributionKind::Panel).count(), 1);

    let (l, _) = transition(&l, Event::Activated, c.at(1));
    let (l, effects) = transition(&l, Event::Disabled, c.at(2));
    assert_eq!(l.state, PluginState::Disabled);
    assert_eq!(
        effects,
        vec![
            Effect::CloseViews,
            Effect::Unload,
            Effect::RemoveContributions,
            Effect::Persist(PluginState::Disabled)
        ]
    );
    set.apply(&m.id, &effects);
    assert_eq!(set.contributions(ContributionKind::Command).count(), 0);
    assert_eq!(set.contributions(ContributionKind::Panel).count(), 0);
    // Still installed: data and grant stay.
    assert!(set.get(&m.id).is_some());

    let (l, effects) = transition(&l, Event::Enabled, c.at(3));
    assert_eq!(l.state, PluginState::Installed);
    set.apply(&m.id, &effects);
    let cmd: Vec<_> = set.contributions(ContributionKind::Command).collect();
    assert_eq!(cmd.len(), 1);
    assert_eq!(
        (cmd[0].plugin.as_str(), cmd[0].id),
        ("test-plugin", "hello")
    );

    set.remove(&m.id);
    assert!(set.get(&m.id).is_none());
    assert_eq!(set.contributions(ContributionKind::Command).count(), 0);
}

#[test]
fn malicious_block_has_no_override() {
    assert!(block(false).override_offered());
    assert!(!block(true).override_offered());
    let c = clock();
    let (l, _) = transition(&active(&c), Event::Blocked(block(true)), c.at(0));
    assert_eq!(l.state, PluginState::Blocked);
    assert_eq!(
        l.block.as_ref().map(BlockEntry::override_offered),
        Some(false)
    );
}

#[test]
fn folder_without_row_is_found_not_installed() {
    let row = |id: &str, state: &str| PluginRow {
        id: id.into(),
        version: "0.1.0".into(),
        source: "registry".into(),
        sha256: "00".into(),
        installed_at_ms: 0,
        state: state.into(),
        previous_version: None,
        files_digest: None,
        authors: None,
    };
    let rows = vec![row("kept", "disabled"), row("gone", "installed")];
    let found = discover(["dropped-in", "kept"], &rows);
    assert_eq!(
        found,
        vec![
            Discovered::FoundNotInstalled("dropped-in".into()),
            Discovered::MissingFolder("gone".into()),
            Discovered::Installed {
                id: "kept".into(),
                state: PluginState::Disabled
            },
        ]
    );
    // Row text round-trips; Active and Unloaded persist as Installed.
    for s in [
        PluginState::Installed,
        PluginState::Disabled,
        PluginState::Quarantined,
        PluginState::Blocked,
    ] {
        assert_eq!(PluginState::from_row(s.as_row()), Some(s));
    }
    assert_eq!(PluginState::Active.as_row(), "installed");
    assert_eq!(PluginState::from_row("bogus"), None);
}

fn quarantined(c: &Clock) -> Lifecycle {
    let l = run(
        active(c),
        [crash(c.at(0)), crash(c.at(1)), crash(c.at(2))].concat(),
    )
    .0;
    assert_eq!(l.state, PluginState::Quarantined);
    l
}

#[test]
fn unblock_returns_a_quarantined_plugin_to_quarantine() {
    let c = clock();
    let (blocked, _) = transition(&quarantined(&c), Event::Blocked(block(false)), c.at(3));
    assert_eq!(blocked.state, PluginState::Blocked);
    let (l, effects) = transition(&blocked, Event::Unblocked, c.at(4));
    assert_eq!(l.state, PluginState::Quarantined);
    assert!(!effects.contains(&Effect::RegisterContributions));
    // A plugin blocked while healthy goes back by its setting.
    let (blocked, _) = transition(&active(&c), Event::Blocked(block(false)), c.at(3));
    let (l, _) = transition(&blocked, Event::Unblocked, c.at(4));
    assert_eq!(l.state, PluginState::Installed);
}

#[test]
fn restart_while_not_quarantined_keeps_the_window() {
    let c = clock();
    let (l, _) = run(active(&c), [crash(c.at(0)), crash(c.at(1))].concat());
    let (l, effects) = transition(&l, Event::Restart, c.at(2));
    assert_eq!(l.state, PluginState::Unloaded);
    assert_eq!(effects, vec![Effect::Unload]);
    let (l, _) = transition(&l, Event::Updated, c.at(2));
    let (l, _) = run(
        l,
        vec![
            (Event::Activated, c.at(3)),
            (Event::Failed(FailureReason::Crashed), c.at(3)),
        ],
    );
    assert_eq!(l.state, PluginState::Quarantined);
}

#[test]
fn window_edges() {
    let c = clock();
    // Ten minutes is inside the window.
    let (l, _) = run(
        active(&c),
        [crash(c.at(0)), crash(c.at(5)), crash(c.at(10))].concat(),
    );
    assert_eq!(l.state, PluginState::Quarantined);
    // 0 and 5 then 11: no; 5, 11 and 12: yes.
    let (l, _) = run(
        active(&c),
        [crash(c.at(0)), crash(c.at(5)), crash(c.at(11))].concat(),
    );
    assert_eq!(l.state, PluginState::Active);
    let (l, _) = transition(&l, Event::Failed(FailureReason::Crashed), c.at(12));
    assert_eq!(l.state, PluginState::Quarantined);
}

#[test]
fn installed_only_from_verified() {
    let c = clock();
    for s in [PluginState::Available, PluginState::Fetched] {
        let (l, effects) = transition(&Lifecycle::new(s, true), Event::Installed, c.at(0));
        assert_eq!(l.state, s);
        assert!(effects.is_empty());
    }
    let (l, _) = transition(
        &Lifecycle::new(PluginState::Verified, false),
        Event::Installed,
        c.at(0),
    );
    assert_eq!(l.state, PluginState::Disabled);
}

#[test]
fn failure_before_activation_stays_installed() {
    let c = clock();
    let installed = Lifecycle::new(PluginState::Installed, true);
    let fail = |at| (Event::Failed(FailureReason::Crashed), c.at(at));
    let (l, effects) = run(installed.clone(), vec![fail(0)]);
    assert_eq!(l.state, PluginState::Installed);
    assert_eq!(
        effects,
        vec![Effect::ShowStopped {
            reason: FailureReason::Crashed,
            quarantined: false
        }]
    );
    let (l, _) = run(installed, vec![fail(0), fail(1), fail(2)]);
    assert_eq!(l.state, PluginState::Quarantined);
}
