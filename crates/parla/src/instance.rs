//! Single instance, headless mode and login launches.
//!
//! The running app owns the local IPC endpoint. A second launch connects with
//! `op show` and exits; the running app shows its window. `parla --headless`
//! runs with no window, every workspace lean, and quits five minutes after the
//! last MCP client left if no window was ever shown. A login launch passes
//! `--hidden`: it also starts with no window and behind the menu-bar icon, but
//! it keeps syncing and notifying in the background and never arms the idle
//! quit, so it stays up until the user quits. With the menu-bar icon off there
//! is no way back, so such a launch starts with a window instead. Nothing here
//! polls: the one timer is armed only while headless with zero MCP connections.

use std::path::Path;
use std::rc::Rc;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use futures::StreamExt as _;
use gpui::{App, Context, Global, Task};
use mcp::ipc::{Accepted, IpcError, IpcHandle, IpcServer};

use crate::workspaces::{Workspaces, WorkspacesHandle};

/// How long a headless app with no MCP client and no window stays up.
pub const HEADLESS_IDLE: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    /// Opens a window at start.
    Window,
    /// No window, lean workspaces and an idle quit (the MCP shim).
    Headless,
    /// No window yet, but a normal hidden window behind the menu-bar icon:
    /// sync and notifications keep running and there is no idle quit (the
    /// login item). With the icon off this resolves to [`Self::Window`].
    Hidden,
}

impl LaunchMode {
    /// `--headless` or the login item's `--hidden` anywhere in `args` (the
    /// program name included or not). A shim launch wins if both are present.
    pub fn from_args<I, S>(args: I) -> LaunchMode
    where
        I: IntoIterator<Item = S>,
        S: AsRef<std::ffi::OsStr>,
    {
        let mut mode = LaunchMode::Window;
        for arg in args {
            if arg.as_ref() == "--headless" {
                return LaunchMode::Headless;
            }
            if arg.as_ref() == "--hidden" {
                mode = LaunchMode::Hidden;
            }
        }
        mode
    }

    /// Resolves a launch against the menu-bar icon setting. A `--hidden` start
    /// with no icon would have no way back (no icon and no Dock icon), so it
    /// starts like a normal Window launch. Other modes are unaffected.
    pub fn with_tray(self, tray: bool) -> LaunchMode {
        if self == Self::Hidden && !tray {
            Self::Window
        } else {
            self
        }
    }

    /// Starts workspaces to match the window shape at launch. Headless has no
    /// window at all and makes each workspace lean at once; hidden treats the
    /// missing window like a closed one, so sessions stay awake and arm their
    /// one-shot lean timers. A normal start waits for the window to start them.
    pub fn start_workspaces(self, w: &mut Workspaces, cx: &mut Context<Workspaces>) {
        match self {
            Self::Window => {}
            Self::Headless => {
                w.set_headless(true, cx);
                w.start_all(cx);
            }
            Self::Hidden => {
                w.set_window_visible(false, cx);
                w.start_all(cx);
            }
        }
    }
}

/// Asks the running instance, if any, to show its window. True when one
/// answered: the caller exits. Blocking and short: a missing endpoint fails at
/// once.
pub fn handoff(path: &Path) -> bool {
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    else {
        return false;
    };
    rt.block_on(mcp::ipc::connect(path, "show")).is_ok()
}

/// What the app does for the instance: show or focus its window, and quit.
pub struct Hooks {
    pub show: Rc<dyn Fn(&mut App)>,
    pub quit: Rc<dyn Fn(&mut App)>,
}

/// The IPC listener, the launch mode and the headless idle timer.
pub struct Instance {
    headless: bool,
    hooks: Rc<Hooks>,
    ipc: IpcHandle,
    mcp_gate: Arc<AtomicBool>,
    connections: usize,
    idle: Option<Task<()>>,
}

impl Global for Instance {}

/// The window opener, kept as a global when the endpoint could not be bound,
/// so the tray and the Dock still bring the window back without an `Instance`.
pub struct ShowFallback(pub Rc<dyn Fn(&mut App)>);

impl Global for ShowFallback {}

impl Instance {
    /// Binds the endpoint at `path` and starts serving `show` requests. `op mcp`
    /// answers `mcp_off` until [`Instance::set_mcp_gate`] opens the gate.
    /// `InUse` means another instance runs: the caller hands off and exits.
    pub fn install(
        cx: &mut App,
        rt: &tokio::runtime::Handle,
        path: &Path,
        mode: LaunchMode,
        hooks: Hooks,
    ) -> Result<(), IpcError> {
        let gate = Arc::new(AtomicBool::new(false));
        let g = gate.clone();
        let (tx, mut rx) = futures::channel::mpsc::unbounded::<Accepted>();
        let ipc = {
            let _enter = rt.enter();
            IpcServer::bind(path)?
                .with_mcp_gate(Arc::new(move || g.load(Ordering::Relaxed)))
                .run(move |a| {
                    let _ = tx.unbounded_send(a);
                })
        };
        cx.set_global(Instance {
            // Only a shim launch is MCP-headless. A hidden login launch shares
            // no window yet, but keeps the normal hidden-window lifecycle and
            // must never arm the idle quit.
            headless: mode == LaunchMode::Headless,
            hooks: Rc::new(hooks),
            ipc,
            mcp_gate: gate,
            connections: 0,
            idle: None,
        });
        Self::arm_idle(cx);
        cx.spawn(async move |cx| {
            while let Some(accepted) = rx.next().await {
                cx.update(|cx| Self::accepted(accepted, cx));
            }
        })
        .detach();
        Ok(())
    }

    fn accepted(accepted: Accepted, cx: &mut App) {
        match accepted {
            Accepted::Show => Self::show(cx),
            // Without an MCP host the stream is dropped: the client sees EOF.
            Accepted::Mcp { stream, peer_pid } => {
                crate::mcp_host::McpHost::accept(stream, peer_pid, cx)
            }
        }
    }

    /// A second launch asked for the window.
    pub fn show(cx: &mut App) {
        let Some(this) = cx.try_global::<Instance>() else {
            if let Some(f) = cx.try_global::<ShowFallback>().map(|f| f.0.clone()) {
                f(cx);
            }
            return;
        };
        let was_headless = this.headless;
        let hooks = this.hooks.clone();
        if was_headless {
            let this = cx.global_mut::<Instance>();
            this.headless = false;
            this.idle = None;
            if let Some(w) = cx.try_global::<WorkspacesHandle>().map(|h| h.0.clone()) {
                w.update(cx, |w, cx| w.set_headless(false, cx));
            }
        }
        (hooks.show)(cx);
    }

    /// The Dock icon was clicked (macOS): show the hidden window, or create it.
    pub fn reopen(cx: &mut App) {
        Self::show(cx);
    }

    pub fn is_headless(cx: &App) -> bool {
        cx.try_global::<Instance>().is_some_and(|i| i.headless)
    }

    /// Opens or closes the `op mcp` gate (the `mcp.enabled` setting).
    pub fn set_mcp_gate(cx: &App, on: bool) {
        if let Some(i) = cx.try_global::<Instance>() {
            i.mcp_gate.store(on, Ordering::Relaxed);
        }
    }

    /// Closes the listener and every live connection.
    pub fn shutdown(cx: &App) {
        if let Some(i) = cx.try_global::<Instance>() {
            i.ipc.shutdown();
        }
    }

    /// An MCP client connected: a headless app stays up while any is.
    pub fn mcp_connected(cx: &mut App) {
        if cx.try_global::<Instance>().is_some() {
            let this = cx.global_mut::<Instance>();
            this.connections += 1;
            this.idle = None;
        }
    }

    /// An MCP client left; the idle countdown restarts with the last one.
    pub fn mcp_disconnected(cx: &mut App) {
        if cx.try_global::<Instance>().is_none() {
            return;
        }
        let this = cx.global_mut::<Instance>();
        this.connections = this.connections.saturating_sub(1);
        Self::arm_idle(cx);
    }

    /// Arms the single quit timer: only headless, only with no connection.
    fn arm_idle(cx: &mut App) {
        let this = cx.global::<Instance>();
        if !this.headless || this.connections > 0 || this.idle.is_some() {
            return;
        }
        let task = cx.spawn(async move |cx| {
            cx.background_executor().timer(HEADLESS_IDLE).await;
            cx.update(|cx| {
                let Some(this) = cx.try_global::<Instance>() else {
                    return;
                };
                if this.headless && this.connections == 0 {
                    let quit = this.hooks.quit.clone();
                    quit(cx);
                }
            });
        });
        cx.global_mut::<Instance>().idle = Some(task);
    }
}
