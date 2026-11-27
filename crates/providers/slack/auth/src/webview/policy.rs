//! Navigation allowlist for the login webview.

use slack_api::validate::{is_slack_host, is_team_id};
use url::Url;

use super::capture::{UrlKind, classify_url};

/// Hooks SPA route changes in the login webview and posts `location.href` to the
/// host. `gone` suppresses events from a page that is unloading, and `lastHref`
/// dedupes so each route change is reported once.
pub const ROUTE_HOOK_SCRIPT: &str = r#"(function () {
  var gone = false;
  var lastHref = location.href;
  var notify = function () {
    if (gone) return;
    var href = location.href;
    if (href === lastHref) return;
    lastHref = href;
    window.ipc.postMessage(href);
  };
  var wrap = function (fn) {
    return function () {
      var r = fn.apply(this, arguments);
      notify();
      return r;
    };
  };
  history.pushState = wrap(history.pushState);
  history.replaceState = wrap(history.replaceState);
  window.addEventListener("hashchange", notify);
  window.addEventListener("popstate", notify);
  window.addEventListener("beforeunload", function () { gone = true; });
  window.addEventListener("pagehide", function () { gone = true; });
})();"#;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavDecision {
    Allow,
    OpenExternally,
    Block,
    /// The desktop-app handoff (`ssb/redirect` or a `slack://` link): not loaded, the
    /// capture sends the webview to the web client instead (see [`handoff`]).
    Handoff,
}

/// Where Slack's "open in the desktop app" step wants to go.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Handoff {
    /// The team id is in the URL: load this web-client URL.
    Client(Url),
    /// Only the workspace subdomain is known; the team id must be looked up.
    Lookup { domain: String },
}

fn client_url(team: &str) -> Option<Url> {
    Url::parse(&format!("https://app.slack.com/client/{team}")).ok()
}

/// Recognises the handoff step after sign-in: `https://<ws>.slack.com/ssb/redirect`
/// (the subdomain only) or a `slack://` deep link (`slack://T.../...` or `?team=T...`).
pub fn handoff(url: &Url) -> Option<Handoff> {
    match url.scheme() {
        "slack" => {
            let team = url
                .host_str()
                .map(|h| h.to_ascii_uppercase())
                .filter(|h| is_team_id(h))
                .or_else(|| {
                    url.query_pairs()
                        .find(|(k, _)| k == "team")
                        .map(|(_, v)| v.into_owned())
                        .filter(|v| is_team_id(v))
                })?;
            client_url(&team).map(Handoff::Client)
        }
        "https" => {
            let host = url.host_str()?;
            let domain = host.strip_suffix(".slack.com")?;
            let mut segs = url.path_segments()?;
            let on_redirect = segs.next() == Some("ssb") && segs.next() == Some("redirect");
            let named = !domain.is_empty() && !domain.contains('.') && domain != "app";
            (on_redirect && named).then(|| Handoff::Lookup {
                domain: domain.to_string(),
            })
        }
        _ => None,
    }
}

/// `https` on `slack.com` and subdomains is allowed. While signing in, any other
/// `https` host (an identity provider) is allowed too. Other hosts open in the
/// default browser; non-`https` is blocked.
pub fn allow_navigation(url: &Url, during_sign_in: bool) -> NavDecision {
    if during_sign_in && handoff(url).is_some() {
        return NavDecision::Handoff;
    }
    if url.scheme() != "https" {
        return NavDecision::Block;
    }
    match url.host_str() {
        Some(h) if is_slack_host(h) => NavDecision::Allow,
        Some(_) if during_sign_in => NavDecision::Allow,
        _ => NavDecision::OpenExternally,
    }
}

/// What to do with a page that asks for a new window (`target=_blank`,
/// `window.open`). No new window is ever opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewWindowAction {
    /// Load it in this webview (it then passes the navigation policy).
    LoadHere,
    /// Desktop-app handoff: report it, the capture sends the webview to the web client.
    Handoff,
    OpenExternally,
    Drop,
}

/// Same policy as [`allow_navigation`], applied to a new-window request.
pub fn new_window_action(url: &Url, during_sign_in: bool) -> NewWindowAction {
    match allow_navigation(url, during_sign_in) {
        NavDecision::Allow => NewWindowAction::LoadHere,
        NavDecision::Handoff => NewWindowAction::Handoff,
        NavDecision::OpenExternally => NewWindowAction::OpenExternally,
        NavDecision::Block => NewWindowAction::Drop,
    }
}

/// A page-controlled SPA route change: only an `https` session or client page is
/// accepted, never a handoff or anything else. Every frame can post to the
/// handler, so the sending frame (`sender`, its document URL) must itself be on
/// `slack.com` or `app.slack.com` and share the reported URL's origin.
pub fn spa_route(sender: &str, body: &str) -> Option<Url> {
    let from = Url::parse(sender).ok()?;
    let url = Url::parse(body).ok()?;
    if url.scheme() != "https" || from.origin() != url.origin() {
        return None;
    }
    if !matches!(from.host_str(), Some("slack.com") | Some("app.slack.com")) {
        return None;
    }
    matches!(
        classify_url(&url),
        UrlKind::SessionPage | UrlKind::ClientReady
    )
    .then_some(url)
}

/// Prefix of the IPC message that carries the chooser's workspace list.
pub const CHOOSER_PREFIX: &str = "parla-chooser:";

/// Runs in every Slack sign-in document. When Slack's own chooser (after the email
/// code, before any `d` cookie exists) gets its `signin.findWorkspaces` answer, it
/// posts each team's id, name, domain, icon, flags and one-time login link to Parla,
/// followed by the current route to wake the capture. The links outlive the page:
/// following one loads a new document.
pub const CHOOSER_HOOK_SCRIPT: &str = r#"(function () {
  if (window.__parlaChooserHook) return;
  Object.defineProperty(window, "__parlaChooserHook", { value: true });
  var icon = function (t) {
    var i = t.icon || {};
    return i.image_132 || i.image_88 || i.image_68 || i.image_44 || t.icon_88 || t.icon_44 || null;
  };
  var seen = function (body) {
    var j;
    try { j = JSON.parse(body); } catch (e) { return; }
    if (!j || j.ok !== true || !Array.isArray(j.current_teams)) return;
    var out = [];
    j.current_teams.forEach(function (a) {
      (a && Array.isArray(a.teams) ? a.teams : []).forEach(function (t) {
        if (!t || typeof t.id !== "string" || typeof t.magic_login_url !== "string") return;
        out.push({
          id: t.id,
          name: String(t.name || ""),
          domain: String(t.domain || ""),
          icon: t.image_default ? null : icon(t),
          sso_required: !!t.sso_required,
          two_factor_required: !!t.two_factor_required,
          is_logged_out: !!t.is_logged_out,
          link: t.magic_login_url
        });
      });
    });
    if (!out.length) return;
    window.ipc.postMessage("parla-chooser:" + JSON.stringify(out));
    window.ipc.postMessage(location.href);
  };
  var isList = function (u) {
    try { return new URL(u, location.href).pathname === "/api/signin.findWorkspaces"; }
    catch (e) { return false; }
  };
  var f = window.fetch;
  window.fetch = function (input) {
    var u = typeof input === "string" ? input : (input && input.url) || "";
    var p = f.apply(this, arguments);
    if (isList(u)) {
      p.then(function (r) { r.clone().text().then(seen, function () {}); }, function () {});
    }
    return p;
  };
  var open = XMLHttpRequest.prototype.open;
  XMLHttpRequest.prototype.open = function (m, u) {
    if (isList(u)) {
      var x = this;
      x.addEventListener("load", function () {
        if (x.responseType === "" || x.responseType === "text") seen(x.responseText);
      });
    }
    return open.apply(this, arguments);
  };
})();"#;

#[derive(serde::Deserialize)]
struct ChooserTeam {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    domain: String,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    sso_required: bool,
    #[serde(default)]
    two_factor_required: bool,
    #[serde(default)]
    is_logged_out: bool,
    #[serde(default)]
    link: Option<String>,
}

/// What Slack's chooser listed: the workspaces, and each one's one-time login
/// link (a secret: never logged, wiped on drop).
#[derive(Default)]
pub struct ChooserList {
    pub teams: Vec<slack_api::SigninWorkspace>,
    pub links: std::collections::HashMap<String, zeroize::Zeroizing<String>>,
}

impl std::fmt::Debug for ChooserList {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ChooserList({} teams, <links redacted>)",
            self.teams.len()
        )
    }
}

/// A login link Parla may load: `https` on `slack.com` or a subdomain of it.
fn is_login_link(u: &str) -> bool {
    Url::parse(u).is_ok_and(|u| {
        u.scheme() == "https"
            && u.host_str()
                .is_some_and(|h| h == "slack.com" || h.ends_with(".slack.com"))
    })
}

/// The chooser's list from a [`CHOOSER_HOOK_SCRIPT`] message. Like [`spa_route`],
/// the sending frame must be an `https` page on `slack.com` or `app.slack.com`.
/// Teams with a malformed id or without a valid login link are dropped; icons must
/// be `https`.
pub fn chooser_list(sender: &str, body: &str) -> Option<ChooserList> {
    let from = Url::parse(sender).ok()?;
    if from.scheme() != "https"
        || !matches!(from.host_str(), Some("slack.com") | Some("app.slack.com"))
    {
        return None;
    }
    let json = zeroize::Zeroizing::new(body.strip_prefix(CHOOSER_PREFIX)?.to_string());
    let teams: Vec<ChooserTeam> = serde_json::from_str(&json).ok()?;
    let mut out = ChooserList::default();
    for t in teams {
        let Some(link) = t.link.map(zeroize::Zeroizing::new) else {
            continue;
        };
        if !is_team_id(&t.id) || !is_login_link(&link) || out.links.contains_key(&t.id) {
            continue;
        }
        out.links.insert(t.id.clone(), link);
        out.teams.push(slack_api::SigninWorkspace {
            id: t.id,
            name: t.name,
            domain: t.domain,
            icon: t.icon.filter(|u| u.starts_with("https://")),
            sso_required: t.sso_required,
            two_factor_required: t.two_factor_required,
            is_logged_out: t.is_logged_out,
        });
    }
    (!out.teams.is_empty()).then_some(out)
}
