//! The limiter moved from `mcp::limits` (its tests, unchanged), now keyed by
//! requester, plus the plugin budget.

use std::time::Duration;

use settings::WorkspaceKey;
use write_confirm::limits::{
    ClientId, Limited, Limits, MAX_PENDING_CONFIRMATIONS, PROVIDER_READS_PER_MIN_PER_WORKSPACE,
    READ_BURST, WRITE_BURST,
};

fn ws(id: &str) -> WorkspaceKey {
    WorkspaceKey::new("fake", id)
}

#[test]
fn reads_rate_limited_per_client() {
    let (l, _clock) = Limits::fake();
    let (a, b) = (ClientId(1), ClientId(2));
    for _ in 0..READ_BURST {
        l.read(a).unwrap();
    }
    let Limited { retry_after_ms } = l.read(a).unwrap_err();
    assert!(
        retry_after_ms > 0 && retry_after_ms <= 500,
        "{retry_after_ms}"
    );
    l.read(b).unwrap();
}

#[test]
fn writes_burst_three() {
    let (l, _clock) = Limits::fake();
    let a = ClientId(1);
    for _ in 0..WRITE_BURST {
        l.write(a).unwrap();
    }
    assert_eq!(l.write(a).unwrap_err().retry_after_ms, 6_000);
    // Reads have their own budget.
    l.read(a).unwrap();
}

#[test]
fn provider_reads_capped_per_workspace() {
    let (l, _clock) = Limits::fake();
    let w = ws("W1");
    l.provider_reads(&w, 12).unwrap();
    // A second client's reads draw on the same workspace budget.
    l.provider_reads(&w, 8).unwrap();
    assert!(l.provider_reads(&w, 1).is_err());
    l.provider_reads(&ws("W2"), 1).unwrap();
    l.provider_reads(&w, 0).unwrap();
    assert!(
        l.provider_reads(&ws("W3"), PROVIDER_READS_PER_MIN_PER_WORKSPACE + 1)
            .is_err()
    );
}

#[test]
fn refills_with_clock() {
    let (l, clock) = Limits::fake();
    let a = ClientId(1);
    for _ in 0..WRITE_BURST {
        l.write(a).unwrap();
    }
    assert!(l.write(a).is_err());
    clock.advance(Duration::from_secs(5));
    assert!(l.write(a).is_err());
    clock.advance(Duration::from_secs(1));
    l.write(a).unwrap();
    assert!(l.write(a).is_err());
    let w = ws("W1");
    l.provider_reads(&w, PROVIDER_READS_PER_MIN_PER_WORKSPACE)
        .unwrap();
    assert!(l.provider_reads(&w, 1).is_err());
    clock.advance(Duration::from_secs(3));
    l.provider_reads(&w, 1).unwrap();
}

#[test]
fn pending_confirmations_capped_and_freed_on_drop() {
    let (l, _clock) = Limits::fake();
    let a = ClientId(1);
    let guards: Vec<_> = (0..MAX_PENDING_CONFIRMATIONS)
        .map(|_| l.pending(a).unwrap())
        .collect();
    assert!(l.pending(a).is_err());
    // Another client has its own queue.
    let _other = l.pending(ClientId(2)).unwrap();
    drop(guards);
    let again = l.pending(a).unwrap();
    drop(again);
}

#[test]
fn forget_releases_client_state() {
    let (l, _clock) = Limits::fake();
    let a = ClientId(1);
    for _ in 0..READ_BURST {
        l.read(a).unwrap();
    }
    assert!(l.read(a).is_err());
    assert_eq!(l.client_count(), 1);
    l.forget(a);
    assert_eq!(l.client_count(), 0);
    l.read(a).unwrap();
}

#[test]
fn guard_outliving_forget_is_harmless() {
    let (l, _clock) = Limits::fake();
    let a = ClientId(1);
    let g = l.pending(a).unwrap();
    l.forget(a);
    drop(g);
    assert!(l.pending(a).is_ok());
}

#[test]
fn limiter_keys_plugins_separately() {
    use write_confirm::limits::{
        PLUGIN_WRITE_BURST, PLUGIN_WRITES_PER_MIN, RequesterKey, WRITES_PER_MIN,
    };
    let (l, clock) = Limits::fake();
    let a = RequesterKey::plugin("standup-bot");
    for _ in 0..PLUGIN_WRITE_BURST {
        l.write(a.clone()).unwrap();
    }
    // One write per 60 / PLUGIN_WRITES_PER_MIN seconds after the burst.
    let wait = 60_000 / u64::from(PLUGIN_WRITES_PER_MIN);
    assert_eq!(l.write(a.clone()).unwrap_err().retry_after_ms, wait);
    // Another plugin and an MCP client each keep their own write budget.
    l.write(RequesterKey::plugin("other")).unwrap();
    l.write(ClientId(1)).unwrap();
    l.pending(a.clone()).unwrap();
    assert_eq!(l.client_count(), 3);
    clock.advance(Duration::from_millis(wait));
    l.write(a.clone()).unwrap();
    l.forget(a);
    assert_eq!(l.client_count(), 2);
    // Design section 9.3: 20 write calls per minute per plugin; MCP keeps its 10.
    assert_eq!((PLUGIN_WRITES_PER_MIN, WRITES_PER_MIN), (20, 10));
}

#[test]
fn plugin_reads_sixty_per_minute() {
    use write_confirm::limits::{PLUGIN_READ_BURST, PLUGIN_READS_PER_MIN, RequesterKey};
    // Design section 9.2: 60 read calls per minute per plugin.
    assert_eq!(PLUGIN_READS_PER_MIN, 60);
    let (l, clock) = Limits::fake();
    let p = RequesterKey::plugin("standup-bot");
    for _ in 0..PLUGIN_READ_BURST {
        l.read(p.clone()).unwrap();
    }
    assert_eq!(l.read(p.clone()).unwrap_err().retry_after_ms, 1_000);
    clock.advance(Duration::from_secs(1));
    l.read(p).unwrap();
}
