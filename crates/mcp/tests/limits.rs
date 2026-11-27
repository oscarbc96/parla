use std::time::Duration;

use mcp::limits::{
    ClientId, Limited, Limits, MAX_PENDING_CONFIRMATIONS, PROVIDER_READS_PER_MIN_PER_WORKSPACE,
    READ_BURST, WRITE_BURST,
};
use settings::WorkspaceKey;

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
