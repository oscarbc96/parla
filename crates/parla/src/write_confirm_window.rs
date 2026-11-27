//! The write confirmation window.
//!
//! [`WindowConfirmer`] is the app's `Confirmer` for MCP and for plugins (both
//! `mcp::confirm::Confirmer` and `write_confirm::Confirmer`): MCP requests
//! and plugin requests share one queue, and each opens one small prompt window ([`components::ConfirmWriteView`]) on the main thread, one at
//! a time and oldest first. Closing the window, the app quitting or the
//! server giving up (it drops the future at its timeout) resolves the request
//! as declined and closes the window. Nothing is armed while no request is
//! pending: no timers, no polling. It works without the main window, so a
//! headless app opens only the prompt.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};

use components::{ConfirmWriteEvent, ConfirmWriteView};
use futures::channel::oneshot;
use futures::future::BoxFuture;
use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, Global, Subscription, TitlebarOptions,
    WindowBounds, WindowKind, WindowOptions, size,
};
use write_confirm::{Confirmation, Confirmer, WriteRequest};

use crate::mcp_backend::Hop;

struct Pending {
    id: u64,
    req: WriteRequest,
    tx: oneshot::Sender<Confirmation>,
}

struct Active {
    id: u64,
    window: AnyWindowHandle,
    tx: oneshot::Sender<Confirmation>,
    _subs: Vec<Subscription>,
}

/// The prompts: the one on screen and those waiting behind it.
#[derive(Default)]
pub struct ConfirmQueue {
    active: Option<Active>,
    pending: VecDeque<Pending>,
}

impl Global for ConfirmQueue {}

impl ConfirmQueue {
    /// How many requests wait behind the open prompt (tests).
    pub fn waiting(cx: &App) -> usize {
        cx.try_global::<ConfirmQueue>()
            .map_or(0, |q| q.pending.len())
    }

    /// The open prompt window (tests).
    pub fn window(cx: &App) -> Option<AnyWindowHandle> {
        cx.try_global::<ConfirmQueue>()
            .and_then(|q| q.active.as_ref().map(|a| a.window))
    }
}

/// The app's `Confirmer`: prompt windows opened through the main-thread hop.
pub struct WindowConfirmer {
    hop: Hop,
    next_id: AtomicU64,
}

impl WindowConfirmer {
    pub fn new(hop: Hop) -> Self {
        Self {
            hop,
            next_id: AtomicU64::new(1),
        }
    }
}

/// Withdraws a request when the server drops the wait (timeout, shutdown).
struct Withdraw {
    hop: Hop,
    id: u64,
}

impl Drop for Withdraw {
    fn drop(&mut self) {
        let id = self.id;
        self.hop
            .run(move |cx| finish(cx, id, Confirmation::TimedOut));
    }
}

impl mcp::confirm::Confirmer for WindowConfirmer {
    fn confirm(
        &self,
        req: mcp::confirm::WriteRequest,
    ) -> BoxFuture<'static, mcp::confirm::Confirmation> {
        let answer = Confirmer::confirm(self, req.into());
        Box::pin(async move { answer.await.into() })
    }
}

impl Confirmer for WindowConfirmer {
    fn confirm(&self, req: WriteRequest) -> BoxFuture<'static, Confirmation> {
        let (tx, rx) = oneshot::channel();
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.hop.run(move |cx| {
            cx.default_global::<ConfirmQueue>()
                .pending
                .push_back(Pending { id, req, tx });
            pump(cx);
        });
        let guard = Withdraw {
            hop: self.hop.clone(),
            id,
        };
        Box::pin(async move {
            let answer = rx.await.unwrap_or(Confirmation::Declined);
            drop(guard);
            answer
        })
    }
}

/// Opens the next prompt when none is showing.
fn pump(cx: &mut App) {
    loop {
        if !cx.has_global::<ConfirmQueue>() {
            return;
        }
        if crate::idle_lock::is_locked(cx) {
            // Nobody can read a prompt behind the lock screen.
            decline_all(cx);
            return;
        }
        let q = cx.global_mut::<ConfirmQueue>();
        if q.active.is_some() {
            return;
        }
        let Some(p) = q.pending.pop_front() else {
            return;
        };
        if p.tx.is_canceled() {
            continue;
        }
        open(cx, p);
    }
}

fn open(cx: &mut App, p: Pending) {
    let Pending { id, req, tx } = p;
    let l = i18n::gpui::localizer(cx);
    let title = components::confirm_write::request_title(&l, &req);
    let bounds = Bounds::centered(
        None,
        size(
            themes::metrics::LAYOUT_WINDOW_CONFIRM_MIN_WIDTH + themes::metrics::SPACE_10,
            themes::metrics::LAYOUT_WINDOW_CONFIRM_HEIGHT,
        ),
        cx,
    );
    let opened = cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            window_min_size: Some(size(
                themes::metrics::LAYOUT_WINDOW_CONFIRM_MIN_WIDTH,
                themes::metrics::LAYOUT_WINDOW_CONFIRM_MIN_HEIGHT,
            )),
            titlebar: Some(TitlebarOptions {
                title: Some(title.into()),
                ..Default::default()
            }),
            kind: WindowKind::PopUp,
            is_minimizable: false,
            ..Default::default()
        },
        |_window, cx| cx.new(|cx| ConfirmWriteView::new(req, cx)),
    );
    let Ok(handle) = opened else {
        let _ = tx.send(Confirmation::Declined);
        return;
    };
    let view = handle.entity(cx).ok();
    let mut subs = Vec::new();
    if let Some(view) = view {
        subs.push(cx.subscribe(&view, move |_, e: &ConfirmWriteEvent, cx| {
            finish(
                cx,
                id,
                match e {
                    ConfirmWriteEvent::Approved => Confirmation::Approved { remember: false },
                    // For MCP, remembering is "for this session".
                    ConfirmWriteEvent::ApprovedForSession
                    | ConfirmWriteEvent::ApprovedAndRemember => {
                        Confirmation::Approved { remember: true }
                    }
                    ConfirmWriteEvent::Declined => Confirmation::Declined,
                },
            );
        }));
        subs.push(cx.observe_release(&view, move |_, cx| {
            finish(cx, id, Confirmation::Declined);
        }));
    }
    cx.global_mut::<ConfirmQueue>().active = Some(Active {
        id,
        window: handle.into(),
        tx,
        _subs: subs,
    });
    crate::tray::bring_to_front(cx);
}

/// Resolves request `id` (shown or still waiting), closes its window and
/// shows the next prompt.
fn finish(cx: &mut App, id: u64, answer: Confirmation) {
    if !cx.has_global::<ConfirmQueue>() {
        return;
    }
    let q = cx.global_mut::<ConfirmQueue>();
    if q.active.as_ref().is_some_and(|a| a.id == id) {
        let a = q.active.take().expect("checked");
        let _ = a.tx.send(answer);
        let _ = a.window.update(cx, |_, window, _| window.remove_window());
        drop(a._subs);
    } else if let Some(i) = q.pending.iter().position(|p| p.id == id) {
        let p = q.pending.remove(i).expect("found");
        let _ = p.tx.send(answer);
    }
    pump(cx);
}

/// Declines the open prompt and every queued one (the idle lock).
pub fn decline_all(cx: &mut App) {
    if !cx.has_global::<ConfirmQueue>() {
        return;
    }
    let q = cx.global_mut::<ConfirmQueue>();
    let active = q.active.take();
    let pending: Vec<Pending> = q.pending.drain(..).collect();
    for p in pending {
        let _ = p.tx.send(Confirmation::Declined);
    }
    if let Some(a) = active {
        let _ = a.tx.send(Confirmation::Declined);
        let _ = a.window.update(cx, |_, window, _| window.remove_window());
        drop(a._subs);
    }
}
