//! Driving a provider task and its progress stream together.

use futures::channel::mpsc::UnboundedReceiver;
use gpui::{Context, Task};

/// Runs `task` while reporting every step of `rx` to `step`, then `done` with
/// the task's result (after any steps still queued). Dropping the returned
/// task drops both, cancelling the work.
pub(crate) fn pump<V, T, R>(
    task: Task<R>,
    rx: UnboundedReceiver<T>,
    cx: &mut Context<V>,
    step: impl Fn(&mut V, T, &mut Context<V>) + 'static,
    done: impl FnOnce(&mut V, R, &mut Context<V>) + 'static,
) -> Task<()>
where
    V: 'static,
    T: 'static,
    R: Unpin + 'static,
{
    cx.spawn(async move |this, cx| {
        use futures::{FutureExt as _, StreamExt as _};
        let mut task = task.fuse();
        let mut rx = rx.fuse();
        let result = loop {
            futures::select! {
                value = rx.next() => match value {
                    Some(value) => {
                        this.update(cx, |this, cx| step(this, value, cx)).ok();
                    }
                    None => break task.await,
                },
                result = task => {
                    // Steps sent just before the task returned arrive first.
                    while let Some(value) = rx.next().now_or_never().flatten() {
                        this.update(cx, |this, cx| step(this, value, cx)).ok();
                    }
                    break result;
                }
            }
        };
        this.update(cx, |this, cx| done(this, result, cx)).ok();
    })
}
