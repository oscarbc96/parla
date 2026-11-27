use domain::{ConversationId, MessageId, MessageTs};
use gpui::App;
use settings::WorkspaceKey;
use url::Url;

use crate::workspaces::WorkspacesHandle;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ParlaLink {
    Open {
        url: Url,
    },
    Conversation {
        ws: WorkspaceKey,
        id: ConversationId,
        ts: Option<MessageTs>,
    },
    Thread {
        ws: WorkspaceKey,
        conversation: ConversationId,
        root: MessageTs,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolvedLink {
    Jump {
        ws: WorkspaceKey,
        conversation: ConversationId,
        message: Option<MessageId>,
    },
    OpenThread {
        ws: WorkspaceKey,
        root: MessageId,
        reply: Option<MessageId>,
    },
    OpenBrowser(String),
    Ignored,
}

pub fn parse_parla_url(s: &str) -> Option<ParlaLink> {
    let url = Url::parse(s).ok()?;
    if url.scheme() != "parla" {
        return None;
    }
    match url.host_str() {
        Some("open") => parse_open(&url),
        _ => parse_workspace_link(&url),
    }
}

fn parse_open(url: &Url) -> Option<ParlaLink> {
    if url.path() != "" && url.path() != "/" {
        return None;
    }
    let raw = url
        .query_pairs()
        .find_map(|(k, v)| (k == "url").then(|| v.into_owned()))?;
    let target = Url::parse(&raw).ok()?;
    if target.scheme() != "https" {
        return None;
    }
    Some(ParlaLink::Open { url: target })
}

fn parse_workspace_link(url: &Url) -> Option<ParlaLink> {
    let provider = url.host_str()?.to_string();
    if provider.is_empty() || provider == "open" {
        return None;
    }
    let segs: Vec<_> = url
        .path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).collect())
        .unwrap_or_default();
    let workspace_id = segs.first()?.to_string();
    if workspace_id.is_empty() {
        return None;
    }
    let ws = WorkspaceKey::new(provider, workspace_id);
    match segs.as_slice() {
        [_, "conversation", c] if !c.is_empty() => {
            let ts = url
                .query_pairs()
                .find_map(|(k, v)| (k == "ts").then(|| v.into_owned()))
                .filter(|v| !v.is_empty())
                .map(|v| MessageTs(std::sync::Arc::from(v)));
            Some(ParlaLink::Conversation {
                ws,
                id: ConversationId(std::sync::Arc::from(*c)),
                ts,
            })
        }
        [_, "thread", c, root] if !c.is_empty() && !root.is_empty() => Some(ParlaLink::Thread {
            ws,
            conversation: ConversationId(std::sync::Arc::from(*c)),
            root: MessageTs(std::sync::Arc::from(*root)),
        }),
        _ => None,
    }
}

pub fn resolve_url(url: &str, cx: &App) -> ResolvedLink {
    let workspaces = cx.try_global::<WorkspacesHandle>().map(|h| h.0.read(cx));
    resolve_url_with(url, workspaces, cx)
}

/// What an input URL is, before any workspace looks at it.
enum Link {
    /// A `parla://` link.
    Parla(ParlaLink),
    /// An http(s) URL: a provider permalink or a web page.
    Web(Url),
    /// `mailto:` and every other parseable scheme: opened outside.
    External(String),
    /// Nothing the app can open.
    Ignored,
}

fn classify(url: &str) -> Link {
    if let Some(link) = parse_parla_url(url) {
        return Link::Parla(link);
    }
    let Ok(parsed) = Url::parse(url) else {
        return Link::Ignored;
    };
    match parsed.scheme() {
        "https" | "http" => Link::Web(parsed),
        "mailto" => Link::External(url.to_string()),
        _ => Link::Ignored,
    }
}

/// Resolves `url` using the provided workspaces state when available.
pub(crate) fn resolve_url_with(
    url: &str,
    workspaces: Option<&crate::workspaces::Workspaces>,
    cx: &App,
) -> ResolvedLink {
    match classify(url) {
        Link::Parla(link) => resolve_parla_link(link, workspaces, cx),
        Link::Web(parsed) => {
            if let Some(w) = workspaces
                && let PermalinkResolution::Resolved(jump) = resolve_permalink(&parsed, w, cx)
            {
                return jump;
            }
            ResolvedLink::OpenBrowser(url.to_string())
        }
        Link::External(url) => ResolvedLink::OpenBrowser(url),
        Link::Ignored => ResolvedLink::Ignored,
    }
}

fn resolve_parla_link(
    link: ParlaLink,
    workspaces: Option<&crate::workspaces::Workspaces>,
    cx: &App,
) -> ResolvedLink {
    match link {
        ParlaLink::Open { url } => {
            let s = url.to_string();
            if let Some(w) = workspaces {
                match resolve_permalink(&url, w, cx) {
                    PermalinkResolution::Resolved(jump) => return jump,
                    PermalinkResolution::Unmatched | PermalinkResolution::NotPermalink => {}
                }
            }
            ResolvedLink::OpenBrowser(s)
        }
        ParlaLink::Conversation { ws, id, ts } => {
            if workspace_signed_in(&ws, workspaces) {
                let message = ts.map(|ts| MessageId {
                    conversation: id.clone(),
                    ts,
                });
                ResolvedLink::Jump {
                    ws,
                    conversation: id,
                    message,
                }
            } else {
                ResolvedLink::Ignored
            }
        }
        ParlaLink::Thread {
            ws,
            conversation,
            root,
        } => {
            if workspace_signed_in(&ws, workspaces) {
                ResolvedLink::OpenThread {
                    ws,
                    root: MessageId {
                        conversation,
                        ts: root,
                    },
                    reply: None,
                }
            } else {
                ResolvedLink::Ignored
            }
        }
    }
}

fn workspace_signed_in(
    ws: &WorkspaceKey,
    workspaces: Option<&crate::workspaces::Workspaces>,
) -> bool {
    workspaces.is_some_and(|w| w.session(ws).is_some())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PermalinkResolution {
    /// A provider recognized the URL as a permalink for a signed-in workspace.
    Resolved(ResolvedLink),
    /// A provider recognized the URL as a permalink, but no signed-in workspace
    /// matched it yet. It may match once workspace starts finish.
    Unmatched,
    /// No provider recognized the URL as a permalink.
    NotPermalink,
}

fn resolve_permalink(
    url: &Url,
    workspaces: &crate::workspaces::Workspaces,
    cx: &App,
) -> PermalinkResolution {
    let registry = workspaces.registry();
    let mut unmatched = false;
    for factory in registry.factories() {
        let Some(parsed) = factory.parse_permalink(url) else {
            continue;
        };
        let Some(ws) = match_workspace_hint(&parsed.workspace_hint, workspaces, cx) else {
            unmatched = true;
            continue;
        };
        if let Some(thread_ts) = parsed.thread {
            let root = MessageId {
                conversation: parsed.conversation.clone(),
                ts: thread_ts.clone(),
            };
            let reply = parsed.ts.and_then(|ts| {
                if ts == thread_ts {
                    None
                } else {
                    Some(MessageId {
                        conversation: parsed.conversation.clone(),
                        ts,
                    })
                }
            });
            return PermalinkResolution::Resolved(ResolvedLink::OpenThread { ws, root, reply });
        }
        let message = parsed.ts.map(|ts| MessageId {
            conversation: parsed.conversation.clone(),
            ts,
        });
        return PermalinkResolution::Resolved(ResolvedLink::Jump {
            ws,
            conversation: parsed.conversation,
            message,
        });
    }
    if unmatched {
        PermalinkResolution::Unmatched
    } else {
        PermalinkResolution::NotPermalink
    }
}

/// True when `url` was recognized as a provider permalink but could not be
/// matched to a signed-in workspace yet, so resolving it may need to wait for
/// workspace starts to finish.
pub fn should_queue_while_starting(url: &str, cx: &App) -> bool {
    let workspaces = cx.try_global::<WorkspacesHandle>().map(|h| h.0.read(cx));
    let unmatched = |url: &Url| {
        workspaces.is_some_and(|w| {
            matches!(
                resolve_permalink(url, w, cx),
                PermalinkResolution::Unmatched
            )
        })
    };
    match classify(url) {
        Link::Parla(ParlaLink::Open { url }) => unmatched(&url),
        Link::Parla(ParlaLink::Conversation { .. } | ParlaLink::Thread { .. }) => true,
        Link::Web(parsed) => unmatched(&parsed),
        Link::External(_) | Link::Ignored => false,
    }
}

fn match_workspace_hint(
    hint: &str,
    w: &crate::workspaces::Workspaces,
    cx: &App,
) -> Option<WorkspaceKey> {
    for account in w.accounts() {
        let Some(session) = w.session(&account.key) else {
            continue;
        };
        let origin = session.read(cx).web_origin();
        let Some(origin) = origin else { continue };
        if origin_matches_hint(&origin, hint) {
            return Some(account.key.clone());
        }
    }
    None
}

/// Handles an OS-opened URL: shows the window, then opens the link in it or
/// queues it until the workspace session it needs is ready.
pub fn handle_open_url(
    url: String,
    show: &dyn Fn(&mut App),
    window: Option<gpui::WindowHandle<crate::root_view::RootView>>,
    cx: &mut App,
) {
    show(cx);
    if let Some(handle) = window {
        handle
            .update(cx, |root, window, cx| root.open_link_url(url, window, cx))
            .ok();
    } else if let Some(w) = cx.try_global::<WorkspacesHandle>().map(|h| h.0.clone()) {
        w.update(cx, |w, cx| w.queue_link(url, cx));
    }
}

/// The trimmed URL to intercept from a quick-switcher paste, when it is a lone
/// `parla://`, `https://` or `http://` link and not text that merely contains a URL.
pub fn lone_pasted_link(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_whitespace) {
        return None;
    }
    if trimmed.starts_with("parla://")
        || trimmed.starts_with("https://")
        || trimmed.starts_with("http://")
    {
        Some(trimmed)
    } else {
        None
    }
}

fn origin_matches_hint(origin: &str, hint: &str) -> bool {
    if origin == hint {
        return true;
    }
    if let Ok(u) = Url::parse(origin) {
        if u.host_str() == Some(hint) {
            return true;
        }
        if u.as_str().trim_end_matches('/') == hint.trim_end_matches('/') {
            return true;
        }
    }
    false
}
