//! Test helpers for the sign-in webview capture. Not used by the application.

use std::cell::RefCell;

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::future::LocalBoxFuture;
use url::Url;

use crate::AuthError;
use crate::webview::{RawCookie, WebviewDriver};

/// A configurable [`WebviewDriver`] for tests.
pub struct FakeWebview {
    pub config: Option<String>,
    pub cookies: Vec<RawCookie>,
    pub evals: RefCell<u32>,
    pub loads: RefCell<Vec<String>>,
    /// Navigation events a `load` of a URL produces (a real webview emits them).
    pub script: RefCell<Vec<(String, Vec<&'static str>)>>,
    pub tx: RefCell<Option<UnboundedSender<Url>>>,
    /// What Slack's chooser listed, if it was seen.
    pub chooser: RefCell<Option<Vec<slack_api::SigninWorkspace>>>,
    /// Chooser login links by team id; `open_team` follows (and removes) one.
    pub links: RefCell<std::collections::HashMap<String, String>>,
}

impl FakeWebview {
    pub fn new(config: Option<&str>, cookies: Vec<RawCookie>) -> Self {
        Self {
            config: config.map(String::from),
            cookies,
            evals: RefCell::new(0),
            loads: RefCell::new(Vec::new()),
            script: RefCell::new(Vec::new()),
            tx: RefCell::new(None),
            chooser: RefCell::new(None),
            links: RefCell::new(std::collections::HashMap::new()),
        }
    }

    pub fn emit(&self, key: &str) {
        if let Some(tx) = &*self.tx.borrow() {
            for (to, events) in self.script.borrow().iter() {
                if to == key {
                    for e in events {
                        tx.unbounded_send(Url::parse(e).unwrap()).unwrap();
                    }
                }
            }
        }
    }
}

impl WebviewDriver for FakeWebview {
    fn eval(&self, _script: &'static str) -> LocalBoxFuture<'_, Result<Option<String>, AuthError>> {
        *self.evals.borrow_mut() += 1;
        let r = self.config.clone();
        Box::pin(async move { Ok(r) })
    }
    fn cookies(&self) -> Result<Vec<RawCookie>, AuthError> {
        Ok(self.cookies.clone())
    }
    fn load(&self, url: &Url) -> Result<(), AuthError> {
        self.loads.borrow_mut().push(url.to_string());
        self.emit(url.as_str());
        Ok(())
    }
    fn chooser_workspaces(&self) -> Option<Vec<slack_api::SigninWorkspace>> {
        self.chooser.borrow().clone()
    }
    fn open_team(&self, team_id: &str) -> Result<bool, AuthError> {
        let link = self.links.borrow_mut().remove(team_id);
        let via_link = link.is_some();
        let url = link.unwrap_or_else(|| format!("https://app.slack.com/client/{team_id}"));
        self.load(&Url::parse(&url).unwrap())?;
        Ok(via_link)
    }
    fn show_handoff_page(&self, url: &Url) -> Result<(), AuthError> {
        self.loads.borrow_mut().push(format!("page:{url}"));
        self.emit(&format!("page:{url}"));
        Ok(())
    }
}

/// Wires `fake` so each load emits the scripted navigation events and returns
/// the URL stream to drive captures with.
pub fn scripted(
    fake: &FakeWebview,
    script: Vec<(&str, Vec<&'static str>)>,
) -> UnboundedReceiver<Url> {
    let (tx, rx) = unbounded();
    *fake.tx.borrow_mut() = Some(tx);
    *fake.script.borrow_mut() = script
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
    rx
}

/// A skip stream that never skips.
pub fn no_skip() -> futures::stream::Pending<()> {
    futures::stream::pending()
}

/// The session cookie `d` used in capture tests.
pub fn dcookie() -> RawCookie {
    RawCookie::new("d", ".slack.com", "xoxd-AbCdEf%2Bghi")
}

/// A discovered-team config for T0000001.
pub const CONFIG: &str = r#"{"teams":{"T0000001":{"name":"Acme","url":"https://acme.slack.com/","token":"xoxc-1111-2222-3333-abcdef"}}}"#;

/// A discovered-team config for T0000001 and T0000002.
pub const CONFIG2: &str = r#"{"teams":{"T0000001":{"name":"Acme","url":"https://acme.slack.com/","token":"xoxc-1111-2222-3333-abcdef"},"T0000002":{"name":"Beta","url":"https://beta.slack.com/","token":"xoxc-4444-5555-6666-abcdef"}}}"#;

/// Client URLs for the scripted teams.
pub const C1: &str = "https://app.slack.com/client/T0000001";
pub const C2: &str = "https://app.slack.com/client/T0000002";
