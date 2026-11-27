//! Epoch ticker: one thread that bumps the engine's
//! epoch every [`TICK`], alive only while at least one guest call is in
//! flight. An idle app has no plugin timer at all.

use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use wasmtime::Engine;

/// Epoch tick. Deadlines are whole ticks, so a 50 ms deadline fires at about
/// 50 to 60 ms (M8 A1 spike: 52 ms median).
pub const TICK: Duration = Duration::from_millis(10);

/// Starts the ticker thread when the in-flight count goes from 0 to 1 and
/// stops it when the count returns to 0. Cheap to clone.
#[derive(Clone)]
pub struct EpochTicker {
    inner: Arc<Inner>,
}

struct Inner {
    engine: Engine,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    in_flight: usize,
    running: Option<Running>,
}

struct Running {
    stop: Arc<Stop>,
    handle: JoinHandle<()>,
}

/// Stop flag the ticker waits on, so stopping wakes it at once.
#[derive(Default)]
struct Stop {
    stopped: Mutex<bool>,
    wake: Condvar,
}

impl EpochTicker {
    pub fn new(engine: Engine) -> Self {
        EpochTicker {
            inner: Arc::new(Inner {
                engine,
                state: Mutex::new(State::default()),
            }),
        }
    }

    /// Marks a guest call in flight until the guard drops.
    pub fn enter(&self) -> InFlight {
        let mut state = self.inner.lock();
        state.in_flight += 1;
        if state.running.is_none() {
            state.running = Some(spawn(self.inner.engine.clone()));
        }
        InFlight {
            inner: self.inner.clone(),
        }
    }

    /// Guest calls in flight now.
    pub fn in_flight(&self) -> usize {
        self.inner.lock().in_flight
    }

    /// True while the ticker thread exists.
    pub fn is_running(&self) -> bool {
        self.inner.lock().running.is_some()
    }
}

impl Inner {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// RAII guard for one guest call; the last one to drop stops the ticker and
/// joins its thread.
#[must_use = "the call counts as in flight only while the guard lives"]
pub struct InFlight {
    inner: Arc<Inner>,
}

impl Drop for InFlight {
    fn drop(&mut self) {
        let running = {
            let mut state = self.inner.lock();
            state.in_flight -= 1;
            if state.in_flight == 0 {
                state.running.take()
            } else {
                None
            }
        };
        if let Some(running) = running {
            *running
                .stop
                .stopped
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = true;
            running.stop.wake.notify_one();
            let _ = running.handle.join();
        }
    }
}

fn spawn(engine: Engine) -> Running {
    let stop = Arc::new(Stop::default());
    let flag = stop.clone();
    let handle = std::thread::Builder::new()
        .name("parla-plugin-epoch".into())
        .spawn(move || {
            // An absolute schedule: oversleeping one tick does not delay the
            // next (a drifting ticker missed 50 ms deadlines by up to 100 ms
            // under load in the A1 spike).
            let mut next = Instant::now() + TICK;
            let mut stopped = flag.stopped.lock().unwrap_or_else(PoisonError::into_inner);
            while !*stopped {
                let wait = next.saturating_duration_since(Instant::now());
                stopped = flag
                    .wake
                    .wait_timeout(stopped, wait)
                    .unwrap_or_else(PoisonError::into_inner)
                    .0;
                if *stopped {
                    break;
                }
                if Instant::now() >= next {
                    engine.increment_epoch();
                    next += TICK;
                }
            }
        })
        .expect("spawn the plugin epoch ticker");
    Running { stop, handle }
}
