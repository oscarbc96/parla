//! The call window: a GPUI window with a `wry` child webview, driven by
//! `HelperCore`. Everything is event driven: stdin lines and webview callbacks
//! arrive on one channel, so an idle helper holds no timer and does no polling.

use std::cell::RefCell;
use std::io::{BufRead, Read, Write};
use std::process::ExitCode;
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{
    App, Bounds, Context, IntoElement, Pixels, Render, Window, WindowBounds, WindowOptions, canvas,
    div, prelude::*, px, size,
};
use gpui_platform::application;
use url::Url;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{PageLoadEvent, PermissionKind, PermissionResponse, Rect, WebView, WebViewBuilder};

use super::core::{CallWebview, HelperCore, HelperOut};
use crate::platform;
use crate::protocol::{MAX_LINE, MediaKind, ParentMsg, WireCookie, decode_parent, encode_helper};

/// The webview, shared between the core, the view and the exit path. A
/// `WebView` is not `Send`; every access happens on the main thread (GPUI's
/// and wry's callbacks both run there), which is what makes the wrapper sound.
struct Handle(RefCell<Option<WebView>>);
// SAFETY: only ever touched on the main thread, see above.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}

#[derive(Clone)]
struct Wv(Arc<Handle>);

impl CallWebview for Wv {
    fn set_cookies(&self, cookies: &[WireCookie]) -> Result<(), String> {
        let b = self.0.0.borrow();
        let wv = b.as_ref().ok_or("no webview")?;
        for c in cookies {
            // The cookie borrows the value, so the one copy made here is wiped
            // when the cookie is set.
            let value = zeroize::Zeroizing::new(c.value.expose().to_owned());
            let cookie = cookie::Cookie::build((c.name.to_string(), value.as_str()))
                .domain(c.domain.to_string())
                .path(c.path.to_string())
                .secure(c.secure)
                .http_only(c.http_only)
                .build();
            wv.set_cookie(&cookie).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn load(&self, url: &str) -> Result<(), String> {
        let b = self.0.0.borrow();
        b.as_ref()
            .ok_or("no webview")?
            .load_url(url)
            .map_err(|e| e.to_string())
    }

    fn set_muted(&self, kind: MediaKind, muted: bool) -> bool {
        self.0
            .0
            .try_borrow()
            .ok()
            .and_then(|b| {
                b.as_ref()
                    .map(|wv| platform::set_capture_muted(wv, kind, muted))
            })
            .unwrap_or(false)
    }
}

type Shared = Arc<Mutex<HelperCore<Wv>>>;

struct View {
    handle: Arc<Handle>,
}

impl Render for View {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let me = cx.entity();
        div().size_full().child(
            canvas(
                move |bounds, _w, cx| me.update(cx, |v, _| v.set_bounds(bounds)),
                |_, _, _, _| {},
            )
            .size_full(),
        )
    }
}

impl View {
    fn set_bounds(&self, b: Bounds<Pixels>) {
        if let Ok(g) = self.handle.0.try_borrow()
            && let Some(w) = g.as_ref()
        {
            let _ = w.set_bounds(Rect {
                position: LogicalPosition::new(
                    f32::from(b.origin.x) as f64,
                    f32::from(b.origin.y) as f64,
                )
                .into(),
                size: LogicalSize::new(
                    f32::from(b.size.width) as f64,
                    f32::from(b.size.height) as f64,
                )
                .into(),
            });
        }
    }
}

fn send_all(tx: &UnboundedSender<HelperOut>, outs: Vec<HelperOut>) {
    for o in outs {
        let _ = tx.unbounded_send(o);
    }
}

/// Runs the helper. Returns only for a startup failure; a call ends by
/// exiting the process.
pub fn run_helper() -> std::process::ExitCode {
    // The first line is the surface. It is read before GPUI starts, so nothing
    // else consumes it and a bad parent never opens a window.
    let stdin = std::io::stdin();
    // Zeroizing, and sized for the protocol limit so `read_line` never leaves
    // the secret in a reallocated buffer: the line carries the session cookie.
    let mut first = zeroize::Zeroizing::new(String::with_capacity(MAX_LINE + 1));
    let read = stdin.lock().take(MAX_LINE as u64 + 1).read_line(&mut first);
    let surface = match (read, decode_parent(first.trim_end())) {
        (Ok(n), Ok(ParentMsg::Open(s))) if n > 0 => s,
        _ => return ExitCode::from(2),
    };
    let title: gpui::SharedString = surface.title.to_string().into();
    let user_agent = surface.user_agent.as_ref().map(|u| u.to_string());

    application().run(move |cx: &mut App| {
        platform::hide_from_dock();
        let (out_tx, mut out_rx) = unbounded::<HelperOut>();
        let (line_tx, mut line_rx) = unbounded::<Option<zeroize::Zeroizing<String>>>();
        std::thread::spawn(move || {
            let stdin = std::io::stdin();
            let mut stdin = stdin.lock();
            loop {
                // Every parent line can carry the session cookie: read into a
                // buffer sized for the protocol limit and wipe it on drop.
                let mut line = zeroize::Zeroizing::new(String::with_capacity(MAX_LINE + 1));
                match stdin.read_line(&mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        if line_tx.unbounded_send(Some(line)).is_err() {
                            return;
                        }
                    }
                }
            }
            let _ = line_tx.unbounded_send(None);
        });

        let handle = Arc::new(Handle(RefCell::new(None)));
        let core: Shared = Arc::new(Mutex::new(HelperCore::new(Wv(handle.clone()))));

        let bounds = Bounds::centered(None, size(px(1100.0), px(800.0)), cx);
        let (h2, core2, out2) = (handle.clone(), core.clone(), out_tx.clone());
        let window = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some(title),
                    ..Default::default()
                }),
                ..Default::default()
            },
            move |window, cx| {
                let (c_nav, o_nav) = (core2.clone(), out2.clone());
                let (c_load, o_load) = (core2.clone(), out2.clone());
                let (c_perm, h_perm) = (core2.clone(), h2.clone());
                let mut b = WebViewBuilder::new()
                    .with_url("about:blank")
                    .with_incognito(true)
                    .with_navigation_handler(move |raw| {
                        let Ok(url) = Url::parse(&raw) else {
                            return false;
                        };
                        // The core is busy only while `Open` sets cookies and
                        // pumps the run loop; only the blank page can load then.
                        let Ok(mut c) = c_nav.try_lock() else {
                            return url.as_str() == "about:blank";
                        };
                        let (allow, outs) = c.on_navigation(&url);
                        send_all(&o_nav, outs);
                        allow
                    })
                    .with_on_page_load_handler(move |ev, raw| {
                        if matches!(ev, PageLoadEvent::Finished)
                            && let Ok(url) = Url::parse(&raw)
                            && let Ok(mut c) = c_load.try_lock()
                        {
                            send_all(&o_load, c.on_page_loaded(&url));
                        }
                    })
                    // wry's handler carries no origin; the page we load is the
                    // allowlisted surface (navigation is already restricted), so
                    // the current page host stands in for the requesting origin.
                    .with_permission_handler(move |kind| {
                        if !matches!(kind, PermissionKind::Microphone | PermissionKind::Camera) {
                            return PermissionResponse::Default;
                        }
                        let Ok(c) = c_perm.try_lock() else {
                            return PermissionResponse::Deny;
                        };
                        let ok = current_origin(&h_perm).is_some_and(|o| c.on_permission(&o));
                        if ok {
                            PermissionResponse::Allow
                        } else {
                            PermissionResponse::Deny
                        }
                    })
                    .with_bounds(Rect {
                        position: LogicalPosition::new(0.0, 0.0).into(),
                        size: LogicalSize::new(1100.0, 800.0).into(),
                    });
                if let Some(ua) = &user_agent {
                    b = b.with_user_agent(ua);
                }
                match b.build_as_child(window) {
                    Ok(wv) => {
                        let (c_cap, o_cap) = (core2.clone(), out2.clone());
                        platform::observe_capture(&wv, move |kind, state| {
                            if let Ok(mut c) = c_cap.try_lock() {
                                send_all(&o_cap, c.on_capture(kind, state));
                            }
                        });
                        *h2.0.borrow_mut() = Some(wv);
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "call webview failed to build");
                        let _ = out2.unbounded_send(HelperOut::Send(
                            crate::protocol::HelperMsg::Error("webview failed to build".into()),
                        ));
                        let _ = out2.unbounded_send(HelperOut::Exit);
                    }
                }
                let view = cx.new(|_| View { handle: h2.clone() });
                let (c_close, o_close) = (core2.clone(), out2.clone());
                window.on_window_should_close(cx, move |_, _| {
                    if let Ok(mut c) = c_close.lock() {
                        send_all(&o_close, c.on_window_closed());
                    }
                    true
                });
                view
            },
        );
        let Ok(window) = window else {
            std::process::exit(1);
        };
        cx.activate(true);

        // The first `Open` goes through the same channel as every output.
        {
            let outs = core
                .lock()
                .map(|mut c| c.on_parent(ParentMsg::Open(surface)))
                .unwrap_or_default();
            send_all(&out_tx, outs);
        }

        let core_in = core.clone();
        let out_in = out_tx.clone();
        cx.spawn(async move |_| {
            while let Some(line) = line_rx.next().await {
                let outs = match line {
                    None => core_in.lock().map(|mut c| c.on_stdin_eof()),
                    Some(l) => match decode_parent(&l) {
                        Ok(m) => core_in.lock().map(|mut c| c.on_parent(m)),
                        Err(e) => {
                            tracing::debug!(error = %e, "ignored a malformed parent line");
                            continue;
                        }
                    },
                };
                send_all(&out_in, outs.unwrap_or_default());
            }
        })
        .detach();

        let handle_exit = handle.clone();
        cx.spawn(async move |cx| {
            let stdout = std::io::stdout();
            while let Some(out) = out_rx.next().await {
                match out {
                    HelperOut::Send(m) => {
                        let mut o = stdout.lock();
                        let _ = writeln!(o, "{}", encode_helper(&m));
                        let _ = o.flush();
                    }
                    HelperOut::OpenExternal(u) => platform::open_external(&u),
                    HelperOut::Activate => {
                        cx.update(|cx| {
                            cx.activate(true);
                            let _ = window.update(cx, |_, w, _| w.activate_window());
                        });
                    }
                    HelperOut::Exit => {
                        // Drop the webview first so WebKit shuts down cleanly.
                        if let Ok(mut g) = handle_exit.0.try_borrow_mut() {
                            g.take();
                        }
                        std::process::exit(0);
                    }
                }
            }
        })
        .detach();
    });
    ExitCode::SUCCESS
}

fn current_origin(h: &Arc<Handle>) -> Option<Url> {
    let g = h.0.try_borrow().ok()?;
    let raw = g.as_ref()?.url().ok()?;
    Url::parse(&raw).ok()
}
