//! wry/WKWebView implementation (macOS). The webview is not `Send`: it lives on the
//! main thread, where `cookies()` pumps the run loop (1 s limit) and is safe.

use std::cell::RefCell;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::channel::{mpsc, oneshot};
use futures::future::LocalBoxFuture;
use url::Url;
use wry::dpi::{LogicalPosition, LogicalSize};
use wry::{NewWindowResponse, PageLoadEvent, Rect, WebView, WebViewBuilder};

use super::capture::{RawCookie, WebviewDriver, decode_eval};
use super::policy::{NavDecision, NewWindowAction, allow_navigation, new_window_action};
use super::ua::{os_major, safari_user_agent};
use crate::AuthError;

const SIGNIN: &str = "https://app.slack.com/signin";

pub struct LoginWebview {
    webview: Rc<WebView>,
    urls: Option<mpsc::UnboundedReceiver<Url>>,
    during_sign_in: Arc<AtomicBool>,
    /// Set to let one handoff page load as is (no web-client URL was found for it).
    show_handoff: Arc<AtomicBool>,
    /// The workspaces Slack's chooser listed, posted by [`super::policy::CHOOSER_HOOK_SCRIPT`].
    chooser: Rc<RefCell<Option<super::policy::ChooserList>>>,
}

impl super::capture::AdderWebview for LoginWebview {
    type Driver<'a> = WryDriver<'a>;

    fn driver(&self) -> Self::Driver<'_> {
        LoginWebview::driver(self)
    }

    fn set_visible(&self, visible: bool) -> Result<(), AuthError> {
        LoginWebview::set_visible(self, visible)
    }

    fn sign_in_done(&self) {
        LoginWebview::sign_in_done(self)
    }
}

impl LoginWebview {
    /// Attaches a non-persistent webview to `window` and loads the sign-in page.
    pub fn open(window: &mut gpui::Window, _cx: &mut gpui::App) -> Result<LoginWebview, AuthError> {
        let (tx, rx) = mpsc::unbounded();
        let during_sign_in = Arc::new(AtomicBool::new(true));
        let nav_flag = during_sign_in.clone();
        let show_handoff = Arc::new(AtomicBool::new(false));
        let handoff_flag = show_handoff.clone();
        let nav_tx = tx.clone();
        let ipc_tx = tx.clone();
        let chooser: Rc<RefCell<Option<super::policy::ChooserList>>> = Rc::default();
        let ipc_chooser = chooser.clone();
        let win_tx = tx.clone();
        let win_flag = during_sign_in.clone();
        // The new-window handler runs on the main thread, like the webview.
        let this: Rc<RefCell<Weak<WebView>>> = Rc::default();
        let win_this = this.clone();
        let load_tx = tx;
        let webview = WebViewBuilder::new()
            .with_url(SIGNIN)
            .with_user_agent(safari_user_agent(os_major()))
            .with_incognito(true)
            .with_initialization_script(super::policy::ROUTE_HOOK_SCRIPT)
            .with_initialization_script(super::policy::CHOOSER_HOOK_SCRIPT)
            .with_ipc_handler(move |req| {
                if req.body().starts_with(super::policy::CHOOSER_PREFIX) {
                    if let Some(list) =
                        super::policy::chooser_list(&req.uri().to_string(), req.body())
                    {
                        *ipc_chooser.borrow_mut() = Some(list);
                    }
                    return;
                }
                if let Some(url) = super::policy::spa_route(&req.uri().to_string(), req.body()) {
                    let _ = ipc_tx.unbounded_send(url);
                }
            })
            .with_navigation_handler(move |raw| {
                let Ok(url) = Url::parse(&raw) else {
                    return false;
                };
                match allow_navigation(&url, nav_flag.load(Ordering::Relaxed)) {
                    NavDecision::Allow => {
                        let _ = nav_tx.unbounded_send(url);
                        true
                    }
                    NavDecision::OpenExternally => {
                        open_external(&url);
                        false
                    }
                    NavDecision::Block => false,
                    NavDecision::Handoff => {
                        // Not loaded: the capture sends the webview to the web client.
                        // The one fallback load of the page itself passes through.
                        // Scheme first, so a `slack://` link cannot use up the flag.
                        let page =
                            url.scheme() == "https" && handoff_flag.swap(false, Ordering::Relaxed);
                        if !page {
                            let _ = nav_tx.unbounded_send(url);
                        }
                        page
                    }
                }
            })
            .with_new_window_req_handler(move |raw, _| {
                // WKWebView drops new-window requests unless a handler exists;
                // Slack's workspace chooser opens "Open" that way. Never open a
                // window: run the navigation policy and act in this webview.
                if let Ok(url) = Url::parse(&raw) {
                    match new_window_action(&url, win_flag.load(Ordering::Relaxed)) {
                        NewWindowAction::LoadHere => {
                            if let Some(wv) = win_this.borrow().upgrade() {
                                let _ = wv.load_url(url.as_str());
                            }
                        }
                        NewWindowAction::Handoff => {
                            let _ = win_tx.unbounded_send(url);
                        }
                        NewWindowAction::OpenExternally => {
                            open_external(&url);
                        }
                        NewWindowAction::Drop => {}
                    }
                }
                NewWindowResponse::Deny
            })
            .with_on_page_load_handler(move |ev, raw| {
                if matches!(ev, PageLoadEvent::Finished)
                    && let Ok(url) = Url::parse(&raw)
                {
                    let _ = load_tx.unbounded_send(url);
                }
            })
            .build_as_child(window)
            .map_err(|e| AuthError::Webview(e.to_string()))?;
        let webview = Rc::new(webview);
        *this.borrow_mut() = Rc::downgrade(&webview);
        Ok(LoginWebview {
            webview,
            urls: Some(rx),
            during_sign_in,
            show_handoff,
            chooser,
        })
    }

    /// Navigation events for `run_capture`. Can be taken once.
    pub fn take_urls(&mut self) -> Option<mpsc::UnboundedReceiver<Url>> {
        self.urls.take()
    }

    /// After capture, other hosts open in the browser instead of the login window.
    pub fn sign_in_done(&self) {
        self.during_sign_in.store(false, Ordering::Relaxed);
    }

    pub fn set_bounds(&self, b: gpui::Bounds<gpui::Pixels>) {
        let _ = self.webview.set_bounds(Rect {
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

    /// Hides the native view while the host shows a native list over it, and
    /// shows it again for a workspace that needs the user.
    pub fn set_visible(&self, visible: bool) -> Result<(), AuthError> {
        self.webview
            .set_visible(visible)
            .map_err(|e| AuthError::Webview(e.to_string()))
    }

    pub fn driver(&self) -> WryDriver<'_> {
        WryDriver {
            webview: &self.webview,
            show_handoff: &self.show_handoff,
            chooser: &self.chooser,
        }
    }

    /// Drops the webview (and its incognito store) to release WebKit memory.
    pub fn close(self) {
        drop(self);
    }
}

pub struct WryDriver<'a> {
    webview: &'a WebView,
    show_handoff: &'a AtomicBool,
    chooser: &'a RefCell<Option<super::policy::ChooserList>>,
}

impl WebviewDriver for WryDriver<'_> {
    fn eval(&self, script: &'static str) -> LocalBoxFuture<'_, Result<Option<String>, AuthError>> {
        let (tx, rx) = oneshot::channel();
        let tx = std::sync::Mutex::new(Some(tx));
        let sent = self
            .webview
            .evaluate_script_with_callback(script, move |raw| {
                if let Some(tx) = tx.lock().ok().and_then(|mut g| g.take()) {
                    let _ = tx.send(decode_eval(&raw));
                }
            });
        Box::pin(async move {
            sent.map_err(|e| AuthError::Webview(e.to_string()))?;
            rx.await
                .map_err(|_| AuthError::Webview("script cancelled".into()))
        })
    }

    fn cookies(&self) -> Result<Vec<RawCookie>, AuthError> {
        let all = self
            .webview
            .cookies()
            .map_err(|e| AuthError::Webview(e.to_string()))?;
        Ok(all
            .iter()
            .map(|c| RawCookie::new(c.name(), c.domain().unwrap_or(""), c.value()))
            .collect())
    }

    fn load(&self, url: &Url) -> Result<(), AuthError> {
        self.webview
            .load_url(url.as_str())
            .map_err(|e| AuthError::Webview(e.to_string()))
    }

    fn chooser_workspaces(&self) -> Option<Vec<slack_api::SigninWorkspace>> {
        self.chooser.borrow().as_ref().map(|c| c.teams.clone())
    }

    fn open_team(&self, team_id: &str) -> Result<bool, AuthError> {
        // A one-time link: taken out so it is followed once.
        let link = self
            .chooser
            .borrow_mut()
            .as_mut()
            .and_then(|c| c.links.remove(team_id));
        if let Some(link) = link {
            let url = Url::parse(&link).map_err(|_| AuthError::Webview("bad link".into()))?;
            self.load(&url)?;
            return Ok(true);
        }
        let target = Url::parse(&format!("https://app.slack.com/client/{team_id}"))
            .map_err(|_| AuthError::Webview("bad team id".into()))?;
        self.load(&target).map(|()| false)
    }

    fn show_handoff_page(&self, url: &Url) -> Result<(), AuthError> {
        if url.scheme() != "https" {
            return Ok(());
        }
        self.show_handoff.store(true, Ordering::Relaxed);
        self.load(url)
    }
}

/// Hand a link to the default browser. The child is reaped on a short-lived thread so it
/// never lingers as a zombie.
fn open_external(url: &url::Url) {
    if let Ok(mut child) = std::process::Command::new("open").arg(url.as_str()).spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}
