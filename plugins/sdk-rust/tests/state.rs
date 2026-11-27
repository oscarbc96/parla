//! `plugin_state!`: per-instance plugin state that each test thread gets
//! fresh, so parallel `cargo test` runs never share it.

use std::sync::{Arc, Barrier};

parla_plugin_sdk::plugin_state!(static COUNT: u32 = 0);

fn bump() -> u32 {
    COUNT.with(|c| {
        *c += 1;
        *c
    })
}

#[test]
fn each_test_thread_starts_fresh() {
    assert_eq!(bump(), 1);
    assert_eq!(bump(), 2);
    assert_eq!(COUNT.get(), 2);
    COUNT.set(10);
    assert_eq!(COUNT.replace(0), 10);
}

#[test]
fn parallel_threads_do_not_share_state() {
    let barrier = Arc::new(Barrier::new(4));
    let handles: Vec<_> = (0..4)
        .map(|_| {
            let b = barrier.clone();
            std::thread::spawn(move || {
                b.wait();
                let mut last = 0;
                for _ in 0..100 {
                    last = bump();
                }
                last
            })
        })
        .collect();
    for h in handles {
        assert_eq!(h.join().unwrap(), 100);
    }
}
