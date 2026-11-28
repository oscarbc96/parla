//! spike: a wry child webview inside a GPUI window (macOS).
//!
//! Opens `https://app.slack.com/signin` in a non-persistent WKWebView attached to
//! a GPUI window. After the owner signs in, commands typed on stdin read the
//! session material. This probe NEVER prints a token or cookie value: only
//! presence, counts, lengths and booleans.
//!
//! Commands (stdin, one per line):
//!   c  cookies on the main thread (wry pumps the run loop, 1 s limit)
//!   t  cookies from a worker thread (unsafe Send wrapper, to test the deadlock claim)
//!   s  read localStorage localConfig_v2 (evaluate_script_with_callback)
//!   a  auth.test per team using only the `d` cookie + token (needs c and s first)
//!   b  GET /ssb/redirect with only the `d` cookie, report whether the boot page has an xoxc token
//!   u  print the current URL
//!   m  print RSS of this process in KiB
//!   d  drop the webview (then `m` again to see memory released)
//!   q  quit
//!
//! Env: PROBE_STORE=incognito (default) | named | default;  PROBE_AUTOCLOSE_SECS=N.

// A WKWebView probe: the real code builds on macOS only.
#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("login_probe drives a WKWebView and runs on macOS only");
    std::process::exit(1);
}

#[cfg(target_os = "macos")]
fn main() {
    probe::main();
}

#[cfg(target_os = "macos")]
mod probe {
    use std::process::Command;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    use futures::StreamExt;
    use futures::channel::mpsc;
    use gpui::{
        App, Bounds, Context, Entity, IntoElement, Pixels, Render, Window, WindowBounds,
        WindowOptions, canvas, div, prelude::*, px, size,
    };
    use gpui_platform::application;
    use wry::dpi::{LogicalPosition, LogicalSize};
    use wry::{Rect, WebView, WebViewBuilder};

    /// WKWebView's default user agent has no `Version/x Safari/x` token, and the sign-in
    /// page answers "your browser is not supported". A Safari-shaped agent fixes that.
    const SAFARI_UA: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Safari/605.1.15";
    const SIGNIN: &str = "https://app.slack.com/signin";

    struct SendPtr(WebView);
    // SAFETY: spike only; used to test whether a worker thread can call cookies().
    unsafe impl Send for SendPtr {}
    unsafe impl Sync for SendPtr {}

    #[derive(Default, Clone)]
    struct Session {
        d_len: Option<usize>,
        /// (token, workspace url), kept in memory only, never printed.
        teams: Vec<(String, String)>,
    }

    struct Probe {
        webview: Option<Arc<SendPtr>>,
        session: Arc<Mutex<Session>>,
        url: Arc<Mutex<String>>,
    }

    impl Probe {
        fn set_bounds(&mut self, b: Bounds<Pixels>) {
            if let Some(w) = &self.webview {
                let r = Rect {
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
                };
                let _ = w.0.set_bounds(r);
            }
        }

        fn cookies_main(&self) {
            let Some(w) = &self.webview else {
                return println!("no webview");
            };
            let t = Instant::now();
            match w.0.cookies() {
                Ok(all) => {
                    let for_url =
                        w.0.cookies_for_url("https://app.slack.com")
                            .map(|v| v.len())
                            .unwrap_or(0);
                    report_cookies(&all, for_url, t.elapsed(), &self.session);
                }
                Err(e) => println!("cookies() error after {:?}: {e}", t.elapsed()),
            }
        }

        fn cookies_thread(&self) {
            let Some(w) = self.webview.clone() else {
                return println!("no webview");
            };
            let session = self.session.clone();
            std::thread::spawn(move || {
                let t = Instant::now();
                match w.0.cookies() {
                    Ok(all) => report_cookies(&all, 0, t.elapsed(), &session),
                    Err(e) => println!("[thread] cookies() error after {:?}: {e}", t.elapsed()),
                }
            });
            println!("[thread] cookies requested off the main thread");
        }

        fn storage(&self) {
            let Some(w) = &self.webview else {
                return println!("no webview");
            };
            let session = self.session.clone();
            let js = r#"(function(){var v=localStorage.getItem("localConfig_v2");return v===null?"__none__":v;})()"#;
            let r = w.0.evaluate_script_with_callback(js, move |json| {
            // The callback receives a JSON string (a JSON string literal here).
            let s: String = serde_json::from_str(&json).unwrap_or_default();
            if s == "__none__" || s.is_empty() {
                println!("localConfig_v2: absent");
                return;
            }
            let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) else {
                return println!("localConfig_v2: present ({} bytes), not JSON", s.len());
            };
            let teams = v.get("teams").and_then(|t| t.as_object());
            let mut found = Vec::new();
            let (mut org, mut with_token) = (0, 0);
            if let Some(teams) = teams {
                for (id, t) in teams {
                    if id.starts_with('E') {
                        org += 1;
                        continue;
                    }
                    let tok = t.get("token").and_then(|x| x.as_str()).unwrap_or("");
                    let url = t.get("url").and_then(|x| x.as_str()).unwrap_or("");
                    if tok.starts_with("xoxc-") {
                        with_token += 1;
                        found.push((tok.to_string(), url.to_string()));
                    }
                }
            }
            let mut keys: Vec<_> = teams
                .and_then(|m| m.values().next())
                .and_then(|t| t.as_object())
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default();
            keys.sort();
            println!(
                "localConfig_v2: present ({} bytes), teams={}, org_level_skipped={org}, xoxc_tokens={with_token}, team_field_names={keys:?}",
                s.len(),
                teams.map_or(0, |m| m.len()),
            );
            session.lock().unwrap().teams = found;
        });
            println!("storage read requested: {r:?}");
        }

        fn auth_test(&self) {
            let s = self.session.lock().unwrap().clone();
            let d = DCOOKIE.lock().unwrap().clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                    let c = reqwest::Client::new();
                    for (i, (tok, url)) in s.teams.iter().enumerate() {
                        for with_d in [true, false] {
                            let mut req = c
                                .post(format!("{}api/auth.test", url_base(url)))
                                .form(&[("token", tok.as_str())]);
                            if with_d && let Some(d) = &d {
                                req = req.header("Cookie", format!("d={d}"));
                            }
                            match req.send().await {
                                Ok(r) => {
                                    let j: serde_json::Value = r.json().await.unwrap_or_default();
                                    println!(
                                        "auth.test team#{i} cookie_d={with_d}: ok={} error={}",
                                        j["ok"],
                                        j.get("error").and_then(|e| e.as_str()).unwrap_or("-")
                                    );
                                }
                                Err(e) => println!("auth.test team#{i} error: {e}"),
                            }
                        }
                    }
                });
            });
        }

        fn boot(&self) {
            let s = self.session.lock().unwrap().clone();
            let d = DCOOKIE.lock().unwrap().clone();
            std::thread::spawn(move || {
                let rt = tokio::runtime::Runtime::new().unwrap();
                rt.block_on(async {
                let c = reqwest::Client::builder().redirect(reqwest::redirect::Policy::limited(5)).build().unwrap();
                for (i, (_, url)) in s.teams.iter().enumerate() {
                    let mut req = c.get(format!("{}ssb/redirect", url_base(url)));
                    if let Some(d) = &d {
                        req = req.header("Cookie", format!("d={d}"));
                    }
                    match req.send().await {
                        Ok(r) => {
                            let status = r.status();
                            let body = r.text().await.unwrap_or_default();
                            println!(
                                "ssb/redirect team#{i}: status={status}, body_len={}, has_api_token_xoxc={}",
                                body.len(),
                                body.contains("\"api_token\":\"xoxc-")
                            );
                        }
                        Err(e) => println!("ssb/redirect team#{i} error: {e}"),
                    }
                }
            });
            });
        }
    }

    static DCOOKIE: Mutex<Option<String>> = Mutex::new(None);

    fn url_base(u: &str) -> String {
        if u.ends_with('/') {
            u.to_string()
        } else {
            format!("{u}/")
        }
    }

    fn report_cookies(
        all: &[cookie::Cookie<'static>],
        for_url: usize,
        took: Duration,
        session: &Arc<Mutex<Session>>,
    ) {
        let names: Vec<String> = all
            .iter()
            .filter(|c| c.domain().is_some_and(|d| d.ends_with("slack.com")))
            .map(|c| c.name().to_string())
            .collect();
        let d = all
            .iter()
            .find(|c| c.name() == "d" && c.domain().is_some_and(|d| d.ends_with("slack.com")));
        match d {
            Some(c) => {
                println!(
                    "cookies in {took:?}: total={}, slack_names={names:?}, d: present, len={}, domain={:?}, http_only={:?}, secure={:?}, cookies_for_url(app.slack.com)={for_url}",
                    all.len(),
                    c.value().len(),
                    c.domain(),
                    c.http_only(),
                    c.secure()
                );
                session.lock().unwrap().d_len = Some(c.value().len());
                *DCOOKIE.lock().unwrap() = Some(c.value().to_string());
            }
            None => println!(
                "cookies in {took:?}: total={}, slack_names={names:?}, d: ABSENT",
                all.len()
            ),
        }
    }

    fn rss_kib() -> String {
        Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
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

    pub fn main() {
        application().run(|cx: &mut App| {
            println!("RSS before webview: {} KiB", rss_kib());
            let (tx, mut rx) = mpsc::unbounded::<String>();
            std::thread::spawn(move || {
                for line in std::io::stdin().lines().map_while(Result::ok) {
                    let _ = tx.unbounded_send(line.trim().to_string());
                }
            });
            let auto = std::env::var("PROBE_AUTOCLOSE_SECS")
                .ok()
                .and_then(|s| s.parse::<u64>().ok());
            if let Some(n) = auto {
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(n));
                    println!("autoclose");
                    std::process::exit(0);
                });
            }

            let bounds = Bounds::centered(None, size(px(1100.0), px(800.0)), cx);
            let window = cx
                .open_window(
                    WindowOptions {
                        window_bounds: Some(WindowBounds::Windowed(bounds)),
                        ..Default::default()
                    },
                    |window, cx| {
                        let url = Arc::new(Mutex::new(String::new()));
                        let url2 = url.clone();
                        let mut b = WebViewBuilder::new()
                            .with_url(
                                std::env::var("PROBE_URL").unwrap_or_else(|_| SIGNIN.to_string()),
                            )
                            .with_user_agent(
                                std::env::var("PROBE_UA").unwrap_or_else(|_| SAFARI_UA.to_string()),
                            )
                            .with_navigation_handler(move |u| {
                                *url2.lock().unwrap() = u;
                                true
                            })
                            .with_on_page_load_handler(|ev, u| {
                                let host = u.split('/').nth(2).unwrap_or("").to_string();
                                let ev = match ev {
                                    wry::PageLoadEvent::Started => "started",
                                    wry::PageLoadEvent::Finished => "finished",
                                };
                                println!("page load {ev}: host={host}");
                            });
                        match std::env::var("PROBE_STORE").as_deref() {
                            Ok("named") => {
                                use wry::WebViewBuilderExtDarwin;
                                b = b.with_data_store_identifier(*b"parla-login-prob");
                            }
                            Ok("default") => {}
                            _ => b = b.with_incognito(true),
                        }
                        let wv = b
                            .with_bounds(Rect {
                                position: LogicalPosition::new(0.0, 0.0).into(),
                                size: LogicalSize::new(1100.0, 800.0).into(),
                            })
                            .build_as_child(window)
                            .expect("build_as_child");
                        println!("webview attached; RSS: {} KiB", rss_kib());
                        cx.new(|_| Probe {
                            webview: Some(Arc::new(SendPtr(wv))),
                            session: Arc::default(),
                            url,
                        })
                    },
                )
                .expect("open window");
            let probe: Entity<Probe> = window.entity(cx).expect("entity");
            cx.activate(true);
            cx.spawn(async move |cx| {
                while let Some(cmd) = rx.next().await {
                    let probe = probe.clone();
                    let quit = cx.update(|cx| {
                        probe.update(cx, |p, _| match cmd.as_str() {
                            "c" => p.cookies_main(),
                            "t" => p.cookies_thread(),
                            "s" => p.storage(),
                            "a" => p.auth_test(),
                            "b" => p.boot(),
                            "u" => println!(
                                "last navigation host: {}",
                                p.url.lock().unwrap().split('/').nth(2).unwrap_or("")
                            ),
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
}
