//! The Tokio runtime that provider I/O runs on, bridged to GPUI with a
//! oneshot-style join: the session awaits the `JoinHandle` on GPUI's foreground
//! executor, so results land back on the main thread.

use gpui::{App, Global};
use tokio::runtime::{Builder, Handle, Runtime};

/// Keeps the runtime alive for the life of the app.
struct TokioRuntime {
    _rt: Runtime,
    handle: Handle,
}

impl Global for TokioRuntime {}

/// Builds the runtime (2 workers, at most 4 blocking threads) and registers it
/// as a global. Panics only if the OS refuses to start threads.
pub fn init_runtime(cx: &mut App) -> Handle {
    let rt = Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(4)
        .thread_name("parla-io")
        .enable_all()
        .build()
        .expect("failed to start the Tokio runtime");
    let handle = rt.handle().clone();
    cx.set_global(TokioRuntime {
        _rt: rt,
        handle: handle.clone(),
    });
    handle
}

/// The runtime handle, if [`init_runtime`] ran. Tests do not start one and run
/// provider futures directly on GPUI's executor.
pub fn handle(cx: &App) -> Option<Handle> {
    cx.try_global::<TokioRuntime>().map(|r| r.handle.clone())
}
