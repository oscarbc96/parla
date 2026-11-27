//! spike: can a cookie-authenticated WKWebView reach a huddle (macOS).
//!
//! Usage: `cargo run -p slack_auth --example call_probe -- <TEAM_ID> <CHANNEL_ID>`
//!
//! The `d` cookie is read from the first stdin line (`xoxd-...`, `d=...` or a
//! `Cookie:` line), or from the OS keychain when `CALL_PROBE_KEYCHAIN=1`
//! (the entry saved by sign-in for `<TEAM_ID>`). It is never taken from argv or
//! env, and no cookie or token value is ever printed: only presence, lengths,
//! hosts and booleans.
//!
//! The webview uses a fresh non-persistent data store. The cookie is set natively
//! (`.slack.com`, HttpOnly, Secure, from a worker thread), then the page loads.
//! No script is injected.
//!
//! Commands (stdin, after the cookie line):
//!   w  load https://app.slack.com/client/<team>/<channel> (signed-in client?)
//!   h  load https://app.slack.com/huddle/<team>/<channel> (huddle surface?)
//!   u  print the current URL host and path (no query)
//!   m  print RSS of this process in KiB
//!   M  toggle microphone capture state (active <-> muted) on the WKWebView, print it
//!   C  toggle camera capture state (active <-> muted) on the WKWebView, print it
//!   s  print both capture states (none, active or muted)
//!   d  drop the webview, then `m` after 10 s to check memory is released
//!   q  quit
//!
//! `M` and `C` call `setMicrophoneCaptureState:` / `setCameraCaptureState:` through
//! `objc2-web-kit` on the `WKWebView` from `wry::WebViewExtMacOS::webview()`, then
//! print the state the web view reports; a state of `none` means no capture is running.
//!
//! Env: CALL_PROBE_AUTOCLOSE_SECS=N (exit after N seconds).
//! CALL_PROBE_PIPED=1: commands come from stdin without a TTY (helper-style launch by
//! another process; no prompt, no terminal echo handling; the cookie line, if not
//! `CALL_PROBE_KEYCHAIN=1`, is still the first line).

use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::channel::mpsc;
use gpui::{
    App, Bounds, Context, Entity, IntoElement, Pixels, Render, Window, WindowBounds, WindowOptions,
    canvas, div, prelude::*, px, size,
};
use gpui_platform::application;
use settings::WorkspaceKey;
use slack_api::SessionCookie;
use slack_auth::webview::{NavDecision, allow_navigation, huddle_url, safari_user_agent};
use slack_auth::{CredentialStore, KeychainStore};
use url::Url;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{PermissionKind, PermissionResponse, Rect, WebView, WebViewBuilder};

struct SendPtr(WebView);
// SAFETY: spike only; the worker thread only calls `set_cookie`, which wry implements
// by dispatching to the main run loop and waiting (same finding as the login probe).
unsafe impl Send for SendPtr {}
unsafe impl Sync for SendPtr {}

struct Probe {
    webview: Option<Arc<SendPtr>>,
    team: String,
    channel: String,
}

impl Probe {
    fn set_bounds(&mut self, b: Bounds<Pixels>) {
        if let Some(w) = &self.webview {
            let _ = w.0.set_bounds(Rect {
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

    /// Toggles (`toggle`) or just prints the capture state of the microphone or camera.
    #[cfg(target_os = "macos")]
    fn capture(&self, camera: bool, toggle: bool) {
        use block2::RcBlock;
        use objc2_web_kit::WKMediaCaptureState as S;
        use wry::WebViewExtMacOS;
        let Some(w) = &self.webview else {
            println!("no webview");
            return;
        };
        let wk = w.0.webview();
        let name = if camera { "camera" } else { "microphone" };
        // SAFETY: called on the main thread (the command loop runs there) on a live WKWebView.
        let read = |wk: &objc2_web_kit::WKWebView| unsafe {
            if camera {
                wk.cameraCaptureState()
            } else {
                wk.microphoneCaptureState()
            }
        };
        let label = |s: S| match s {
            S::Active => "active",
            S::Muted => "muted",
            S::None => "none",
            _ => "unknown",
        };
        let before = read(&wk);
        if !toggle {
            println!("{name}: {}", label(before));
            return;
        }
        let next = match before {
            S::Active => S::Muted,
            S::Muted => S::Active,
            _ => {
                println!("{name}: {} (nothing to toggle)", label(before));
                return;
            }
        };
        let wk2 = wk.clone();
        let done = RcBlock::new(move || {
            // SAFETY: WebKit invokes the handler on the main thread.
            let now = unsafe {
                if camera {
                    wk2.cameraCaptureState()
                } else {
                    wk2.microphoneCaptureState()
                }
            };
            println!("{name}: now {}", label(now));
        });
        // SAFETY: main thread, live WKWebView; the block is copied by WebKit.
        unsafe {
            if camera {
                wk.setCameraCaptureState_completionHandler(next, Some(&done));
            } else {
                wk.setMicrophoneCaptureState_completionHandler(next, Some(&done));
            }
        }
        println!("{name}: {} -> {} requested", label(before), label(next));
    }

    #[cfg(not(target_os = "macos"))]
    fn capture(&self, _camera: bool, _toggle: bool) {
        println!("capture state keys need macOS");
    }

    fn load(&self, url: &str) {
        match &self.webview {
            Some(w) => println!("load {}: {:?}", host_path(url), w.0.load_url(url).is_ok()),
            None => println!("no webview"),
        }
    }
}

fn host_path(u: &str) -> String {
    Url::parse(u)
        .map(|u| format!("{}{}", u.host_str().unwrap_or(""), u.path()))
        .unwrap_or_default()
}

fn rss_kib() -> String {
    Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// Builds the native `d` cookie. The value is already URL-encoded by `SessionCookie`.
fn native_cookie(header: &str, name: &str) -> Option<cookie::Cookie<'static>> {
    let value = header
        .split("; ")
        .find_map(|p| p.strip_prefix(&format!("{name}=")))?;
    Some(
        cookie::Cookie::build((name.to_string(), value.to_string()))
            .domain(".slack.com")
            .path("/")
            .secure(true)
            .http_only(true)
            .build(),
    )
}

impl Render for Probe {
    fn render(&mut self, _w: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let me = cx.entity();
        div().size_full().child(
            canvas(
                move |bounds, _w, cx| me.update(cx, |p, _| p.set_bounds(bounds)),
                |_, _, _, _| {},
            )
            .size_full(),
        )
    }
}

fn stty(arg: &str) {
    let _ = Command::new("stty").arg(arg).status();
}

fn read_cookie_stdin(piped: bool) -> Option<SessionCookie> {
    use std::io::IsTerminal;
    let tty = !piped && std::io::stdin().is_terminal();
    if tty {
        eprintln!("paste the d cookie (input is hidden), then Enter:");
        stty("-echo");
    }
    let mut line = String::new();
    let read = std::io::stdin().read_line(&mut line);
    if tty {
        stty("echo");
    }
    read.ok()?;
    let c = SessionCookie::parse(&line);
    line.clear();
    c.ok()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("usage: call_probe <TEAM_ID> <CHANNEL_ID>  (see the module doc for keys and env)");
        return;
    }
    let piped = std::env::var("CALL_PROBE_PIPED").as_deref() == Ok("1");
    let (Some(team), Some(channel)) = (args.first().cloned(), args.get(1).cloned()) else {
        eprintln!(
            "usage: call_probe <TEAM_ID> <CHANNEL_ID>  (cookie on stdin, or CALL_PROBE_KEYCHAIN=1)"
        );
        std::process::exit(2);
    };
    if huddle_url(&team, &channel).is_none() {
        eprintln!("team or channel id is malformed");
        std::process::exit(2);
    }
    let use_keychain = std::env::var("CALL_PROBE_KEYCHAIN").as_deref() == Ok("1");
    // Stdin cookie is read before the UI starts, so nothing else consumes the line.
    let stdin_cookie = if use_keychain {
        None
    } else {
        read_cookie_stdin(piped)
    };
    if !use_keychain && stdin_cookie.is_none() {
        eprintln!("no usable cookie on stdin (expected xoxd-..., d=... or a Cookie: line)");
        std::process::exit(2);
    }

    application().run(move |cx: &mut App| {
        if piped {
            println!("piped mode: commands from stdin, no TTY");
        }
        println!("RSS before webview: {} KiB", rss_kib());
        let (tx, mut rx) = mpsc::unbounded::<String>();
        let cmd_tx = tx.clone();
        std::thread::spawn(move || {
            for line in std::io::stdin().lines().map_while(Result::ok) {
                let _ = tx.unbounded_send(line.trim().to_string());
            }
        });
        if let Some(n) = std::env::var("CALL_PROBE_AUTOCLOSE_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
        {
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_secs(n));
                println!("autoclose");
                std::process::exit(0);
            });
        }

        let bounds = Bounds::centered(None, size(px(1100.0), px(800.0)), cx);
        let (t2, c2) = (team.clone(), channel.clone());
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    ..Default::default()
                },
                move |window, cx| {
                    let wv = WebViewBuilder::new()
                        .with_url("about:blank")
                        .with_user_agent(safari_user_agent(slack_auth::webview::os_major()))
                        .with_incognito(true)
                        .with_navigation_handler(|raw| {
                            let Ok(url) = Url::parse(&raw) else {
                                return false;
                            };
                            if url.scheme() == "about" {
                                return true;
                            }
                            match allow_navigation(&url, false) {
                                NavDecision::Allow => {
                                    println!("nav allow: {}", host_path(&raw));
                                    true
                                }
                                NavDecision::OpenExternally => {
                                    println!("nav external: host={}", url.host_str().unwrap_or(""));
                                    let _ = Command::new("open").arg(url.as_str()).spawn();
                                    false
                                }
                                NavDecision::Block | NavDecision::Handoff => {
                                    println!("nav blocked: scheme={}", url.scheme());
                                    false
                                }
                            }
                        })
                        .with_permission_handler(|kind| {
                            println!("permission request: {kind:?}");
                            match kind {
                                PermissionKind::Microphone | PermissionKind::Camera => {
                                    PermissionResponse::Allow
                                }
                                _ => PermissionResponse::Default,
                            }
                        })
                        .with_on_page_load_handler(|ev, u| {
                            let ev = match ev {
                                wry::PageLoadEvent::Started => "started",
                                wry::PageLoadEvent::Finished => "finished",
                            };
                            println!("page load {ev}: {}", host_path(&u));
                        })
                        .with_bounds(Rect {
                            position: LogicalPosition::new(0.0, 0.0).into(),
                            size: LogicalSize::new(1100.0, 800.0).into(),
                        })
                        .build_as_child(window)
                        .expect("build_as_child");
                    println!("webview attached; RSS: {} KiB", rss_kib());
                    cx.new(|_| Probe {
                        webview: Some(Arc::new(SendPtr(wv))),
                        team: t2,
                        channel: c2,
                    })
                },
            )
            .expect("open window");
        let probe: Entity<Probe> = window.entity(cx).expect("entity");
        cx.activate(true);

        // Obtain the cookie, then set it natively from a worker thread and load the client.
        let team2 = team.clone();
        let probe2 = probe.clone();
        cx.spawn(async move |cx| {
            let cookie = if use_keychain {
                let store = KeychainStore::new(cx.clone());
                match store.load(&WorkspaceKey::new("slack", team2.clone())).await {
                    Ok(Some(c)) => Some(c.cookie),
                    Ok(None) => None,
                    Err(e) => {
                        println!("keychain error: {e}");
                        None
                    }
                }
            } else {
                stdin_cookie
            };
            let Some(cookie) = cookie else {
                println!("no cookie available; quitting");
                cx.update(|cx| cx.quit());
                return;
            };
            let header = cookie.header_value();
            println!("cookie loaded (header len {})", header.len());
            let wv = cx.update(|cx| probe2.read(cx).webview.clone());
            let Some(wv) = wv else { return };
            let ready = cmd_tx.clone();
            std::thread::spawn(move || {
                let mut set = Vec::new();
                for name in ["d", "d-s"] {
                    if let Some(c) = native_cookie(&header, name) {
                        set.push((name, wv.0.set_cookie(&c).is_ok()));
                    }
                }
                println!("set_cookie off the main thread: {set:?}");
                let _ = ready.unbounded_send("w".to_string());
            });
        })
        .detach();

        cx.spawn(async move |cx| {
            while let Some(cmd) = rx.next().await {
                let probe = probe.clone();
                let quit = cx.update(|cx| {
                    probe.update(cx, |p, _| match cmd.as_str() {
                        "w" => p.load(&format!(
                            "https://app.slack.com/client/{}/{}",
                            p.team, p.channel
                        )),
                        "h" => {
                            let url = huddle_url(&p.team, &p.channel).expect("validated");
                            p.load(url.as_str());
                        }
                        "u" => match &p.webview {
                            Some(w) => println!(
                                "current: {}",
                                w.0.url().map(|u| host_path(&u)).unwrap_or_default()
                            ),
                            None => println!("no webview"),
                        },
                        "M" => p.capture(false, true),
                        "C" => p.capture(true, true),
                        "s" => {
                            p.capture(false, false);
                            p.capture(true, false);
                        }
                        "m" => println!("RSS: {} KiB", rss_kib()),
                        "d" => {
                            p.webview = None;
                            println!("webview dropped");
                        }
                        _ => {}
                    });
                    cmd == "q"
                });
                if quit {
                    cx.update(|cx| cx.quit());
                    break;
                }
            }
        })
        .detach();
    });
}
