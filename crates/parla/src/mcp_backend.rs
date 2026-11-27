//! `AppBackend`: the MCP tools' view of the running app (spec 8a, "the server
//! reuses the app's sessions, cache and rate limiter").
//!
//! The server runs on the Tokio runtime; sessions live on GPUI's main thread.
//! Every call hops to the main thread only to read entity state or start a
//! session task (a [`Hop`] job), and the provider and store futures run on the
//! runtime, never on the main thread. Reads come from the local store first;
//! a store that is closed (a lean workspace) counts as a miss.

use std::sync::{Arc, mpsc};
use std::time::Duration;

use domain::{
    ChatProvider, ClientMsgId, Conversation, ConversationId, EmojiRef, HistoryPage, Member,
    Message, MessageId, MessageTs, PersistOp, ProviderError, ScheduledMessage, SearchQuery, Status,
    UserId,
};
use futures::StreamExt as _;
use futures::channel::{mpsc as fmpsc, oneshot};
use futures::future::BoxFuture;
use gpui::App;
use mcp::backend::{Backend, BackendError, Fetched, ScheduleRequest, SendRequest, WorkspaceInfo};
use settings::WorkspaceKey;
use store::Store;

use crate::session::{QUEUED_OFFLINE, WorkspaceSession};
use crate::workspaces::WorkspacesHandle;

type Job = Box<dyn FnOnce(&mut App) + Send>;

/// How long a synchronous [`Backend`] method waits for the main thread.
const SYNC_WAIT: Duration = Duration::from_secs(2);

/// A queue of closures run on GPUI's main thread, with `&mut App`.
#[derive(Clone)]
pub struct Hop {
    tx: fmpsc::UnboundedSender<Job>,
}

impl Hop {
    /// Starts the main-thread loop. Idle cost: one task parked on a channel.
    pub fn start(cx: &mut App) -> Hop {
        let (tx, mut rx) = fmpsc::unbounded::<Job>();
        cx.spawn(async move |cx| {
            while let Some(job) = rx.next().await {
                cx.update(|cx| job(cx));
            }
        })
        .detach();
        Hop { tx }
    }

    /// Runs `f` on the main thread, fire and forget.
    pub fn run(&self, f: impl FnOnce(&mut App) + Send + 'static) {
        let _ = self.tx.unbounded_send(Box::new(f));
    }

    /// Runs `f` on the main thread; it answers through the sender, possibly
    /// later (a session task). `Closed` when the app is gone.
    pub fn call<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut App, oneshot::Sender<R>) + Send + 'static,
    ) -> BoxFuture<'static, Result<R, BackendError>> {
        let (tx, rx) = oneshot::channel();
        let sent = self
            .tx
            .unbounded_send(Box::new(move |cx| f(cx, tx)))
            .is_ok();
        Box::pin(async move {
            if !sent {
                return Err(BackendError::Closed);
            }
            rx.await.map_err(|_| BackendError::Closed)
        })
    }

    /// As [`Hop::call`] for a closure that answers at once, waiting for it
    /// from a non-main thread (the sync `Backend` methods).
    fn call_blocking<R: Send + 'static>(
        &self,
        f: impl FnOnce(&mut App) -> R + Send + 'static,
    ) -> Option<R> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .unbounded_send(Box::new(move |cx| {
                let _ = tx.send(f(cx));
            }))
            .ok()?;
        rx.recv_timeout(SYNC_WAIT).ok()
    }
}

/// What a session's I/O needs, cloned out of the entity on the main thread.
#[derive(Clone)]
pub struct SessionIo {
    pub workspace: WorkspaceKey,
    pub provider: Arc<dyn ChatProvider>,
    pub store: Arc<dyn Store>,
}

fn prov(e: ProviderError) -> BackendError {
    BackendError::Provider(e)
}

impl SessionIo {
    /// Newest first, strictly older than `before`: the store, then (when
    /// `remote` and the store came up short) the provider, persisting what
    /// it returns.
    pub async fn page(
        &self,
        c: &ConversationId,
        before: Option<MessageTs>,
        limit: u32,
        remote: bool,
    ) -> Result<Fetched<HistoryPage>, ProviderError> {
        let cached = self
            .store
            .messages_before(&self.workspace, c, before.clone(), limit as usize)
            .await
            .unwrap_or_default();
        if cached.len() >= limit as usize || !remote {
            let has_more = cached.len() >= limit as usize;
            return Ok(Fetched {
                value: HistoryPage {
                    messages: cached,
                    has_more,
                },
                provider_calls: 0,
            });
        }
        let mut page = self.provider.history(c, before, limit).await?;
        let _ = self
            .store
            .apply(
                &self.workspace,
                PersistOp::Messages {
                    conversation: c.clone(),
                    messages: page.messages.clone(),
                },
            )
            .await;
        page.messages.sort_by(|a, b| b.id.ts.cmp(&a.id.ts));
        Ok(Fetched {
            value: page,
            provider_calls: 1,
        })
    }

    /// Ascending, root first when `after` is `None`.
    pub async fn thread_page(
        &self,
        c: &ConversationId,
        root: &MessageTs,
        after: Option<MessageTs>,
        limit: u32,
        remote: bool,
    ) -> Result<Fetched<HistoryPage>, ProviderError> {
        let cached = self
            .store
            .thread(&self.workspace, c, root, limit as usize)
            .await
            .unwrap_or_default();
        let complete = after.is_none()
            && cached.first().is_some_and(|r| {
                r.id.ts == *root && cached.len() as u32 > r.reply_count.min(limit.saturating_sub(1))
            });
        if complete || !remote {
            let messages = if after.is_none() { cached } else { Vec::new() };
            return Ok(Fetched {
                value: HistoryPage {
                    messages,
                    has_more: false,
                },
                provider_calls: 0,
            });
        }
        let page = self.provider.replies(c, root, after, limit).await?;
        Ok(Fetched {
            value: page,
            provider_calls: 1,
        })
    }

    async fn search(
        &self,
        query: &str,
        limit: u32,
        remote: bool,
    ) -> Result<Fetched<Vec<Message>>, ProviderError> {
        let hits = self
            .store
            .search(&self.workspace, query, limit as usize)
            .await
            .unwrap_or_default();
        let mut out = Vec::new();
        for h in hits {
            if let Ok(Some(m)) = self
                .store
                .message(&self.workspace, &h.id.conversation, &h.id.ts)
                .await
            {
                out.push(m);
            }
        }
        let mut calls = 0;
        if remote && out.len() < limit as usize && self.provider.capabilities().search {
            let q = SearchQuery {
                text: Arc::from(query),
                conversations: Vec::new(),
                authors: Vec::new(),
                before: None,
                after: None,
                has: Vec::new(),
            };
            let page = self.provider.search_messages(&q, None, limit).await?;
            calls = 1;
            for hit in page.items {
                if !out.iter().any(|m| m.id == hit.message.id) {
                    out.push(hit.message);
                }
            }
            out.truncate(limit as usize);
        }
        Ok(Fetched {
            value: out,
            provider_calls: calls,
        })
    }
}

impl WorkspaceSession {
    /// The handles the MCP backend runs provider and store calls with.
    pub fn io(&self) -> SessionIo {
        SessionIo {
            workspace: self.state().workspace.clone(),
            provider: self.provider.clone(),
            store: self.store_handle(),
        }
    }

    /// A page of a conversation: store first, then the provider through the
    /// same persistence as scrolling.
    pub fn page(
        &mut self,
        c: ConversationId,
        before: Option<MessageTs>,
        limit: u32,
        remote: bool,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Task<Result<Fetched<HistoryPage>, ProviderError>> {
        let io = self.io();
        spawn_io(cx, async move { io.page(&c, before, limit, remote).await })
    }

    /// As [`WorkspaceSession::page`] for a thread.
    pub fn thread_page(
        &mut self,
        c: ConversationId,
        root: MessageTs,
        after: Option<MessageTs>,
        limit: u32,
        remote: bool,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::Task<Result<Fetched<HistoryPage>, ProviderError>> {
        let io = self.io();
        spawn_io(cx, async move {
            io.thread_page(&c, &root, after, limit, remote).await
        })
    }
}

/// Runs `fut` on the Tokio runtime (directly on GPUI's executor in tests that
/// start none) and hands the result back as a GPUI task.
fn spawn_io<T: Send + 'static>(
    cx: &mut gpui::App,
    fut: impl std::future::Future<Output = Result<T, ProviderError>> + Send + 'static,
) -> gpui::Task<Result<T, ProviderError>> {
    let handle = crate::runtime::handle(cx);
    cx.spawn(async move |_| match handle {
        Some(h) => h
            .spawn(fut)
            .await
            .unwrap_or_else(|e| Err(ProviderError::Other(e.to_string()))),
        None => fut.await,
    })
}

/// `impl Backend` over the workspace sessions.
pub struct AppBackend {
    hop: Hop,
}

impl AppBackend {
    pub fn new(hop: Hop) -> AppBackend {
        AppBackend { hop }
    }

    /// The session's I/O handles, read on the main thread.
    fn io(&self, ws: &WorkspaceKey) -> BoxFuture<'static, Result<SessionIo, BackendError>> {
        let ws = ws.clone();
        let call = self.hop.call(move |cx, tx| {
            let io = session(cx, &ws).map(|s| s.read(cx).io());
            let _ = tx.send(io.ok_or(BackendError::NotFound));
        });
        Box::pin(async move { call.await? })
    }
}

fn session(cx: &App, ws: &WorkspaceKey) -> Option<gpui::Entity<WorkspaceSession>> {
    cx.try_global::<WorkspacesHandle>()?.0.read(cx).session(ws)
}

impl Backend for AppBackend {
    fn workspaces(&self) -> Vec<WorkspaceInfo> {
        self.hop
            .call_blocking(|cx| {
                let Some(h) = cx.try_global::<WorkspacesHandle>() else {
                    return Vec::new();
                };
                let w = h.0.read(cx);
                w.sessions()
                    .map(|s| {
                        let s = s.read(cx);
                        let st = s.state();
                        WorkspaceInfo {
                            key: st.workspace.clone(),
                            name: st.name.clone(),
                            provider_display: Arc::from(s.display_name()),
                            self_user: st.self_user.clone(),
                            caps: st.caps.clone(),
                        }
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    fn conversations(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<Conversation>, BackendError>> {
        let ws = ws.clone();
        let call = self.hop.call(move |cx, tx| {
            let out = session(cx, &ws)
                .map(|s| s.read(cx).state().conversations.values().cloned().collect())
                .ok_or(BackendError::NotFound);
            let _ = tx.send(out);
        });
        Box::pin(async move { call.await? })
    }

    fn messages(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        before: Option<MessageTs>,
        limit: u32,
        remote: bool,
    ) -> BoxFuture<'_, Result<Fetched<HistoryPage>, BackendError>> {
        let (io, c) = (self.io(ws), c.clone());
        Box::pin(async move {
            io.await?
                .page(&c, before, limit, remote)
                .await
                .map_err(prov)
        })
    }

    fn thread(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        root: &MessageTs,
        after: Option<MessageTs>,
        limit: u32,
        remote: bool,
    ) -> BoxFuture<'_, Result<Fetched<HistoryPage>, BackendError>> {
        let (io, c, root) = (self.io(ws), c.clone(), root.clone());
        Box::pin(async move {
            io.await?
                .thread_page(&c, &root, after, limit, remote)
                .await
                .map_err(prov)
        })
    }

    fn search(
        &self,
        ws: &WorkspaceKey,
        query: &str,
        limit: u32,
        remote: bool,
    ) -> BoxFuture<'_, Result<Fetched<Vec<Message>>, BackendError>> {
        let (io, query) = (self.io(ws), query.to_string());
        Box::pin(async move { io.await?.search(&query, limit, remote).await.map_err(prov) })
    }

    fn member(
        &self,
        ws: &WorkspaceKey,
        id: &UserId,
    ) -> BoxFuture<'_, Result<Option<Member>, BackendError>> {
        let (ws, id) = (ws.clone(), id.clone());
        let known = {
            let (ws, id) = (ws.clone(), id.clone());
            self.hop.call(move |cx, tx| {
                let out = session(cx, &ws)
                    .map(|s| s.read(cx).state().members.get(&id).cloned())
                    .ok_or(BackendError::NotFound);
                let _ = tx.send(out);
            })
        };
        let io = self.io(&ws);
        Box::pin(async move {
            if let Some(m) = known.await?? {
                return Ok(Some(m));
            }
            let io = io.await?;
            // A provider that cannot look users up knows only the cached ones.
            let found = match io.provider.members(std::slice::from_ref(&id)).await {
                Ok(found) => found,
                Err(ProviderError::Unsupported) => Vec::new(),
                Err(e) => return Err(prov(e)),
            };
            Ok(found.into_iter().find(|m| m.id == id))
        })
    }

    fn search_members(
        &self,
        ws: &WorkspaceKey,
        query: &str,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<Member>, BackendError>> {
        let (ws, q) = (ws.clone(), query.to_lowercase());
        let call = self.hop.call(move |cx, tx| {
            let out = session(cx, &ws)
                .map(|s| {
                    let mut v: Vec<Member> = s
                        .read(cx)
                        .state()
                        .members
                        .values()
                        .filter(|m| {
                            m.display_name.to_lowercase().contains(&q)
                                || m.real_name.to_lowercase().contains(&q)
                        })
                        .cloned()
                        .collect();
                    v.sort_by(|a, b| a.display_name.cmp(&b.display_name).then(a.id.cmp(&b.id)));
                    v.truncate(limit as usize);
                    v
                })
                .ok_or(BackendError::NotFound);
            let _ = tx.send(out);
        });
        Box::pin(async move { call.await? })
    }

    fn scheduled(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<ScheduledMessage>, BackendError>> {
        let io = self.io(ws);
        Box::pin(async move { io.await?.provider.scheduled().await.map_err(prov) })
    }

    fn send(
        &self,
        ws: &WorkspaceKey,
        req: SendRequest,
    ) -> BoxFuture<'_, Result<Message, BackendError>> {
        let ws = ws.clone();
        let call = self.hop.call(move |cx, tx| {
            let Some(s) = session(cx, &ws) else {
                let _ = tx.send(Err(BackendError::NotFound));
                return;
            };
            let id = req.idempotency_key.clone().map(ClientMsgId);
            let task = s.update(cx, |s, cx| {
                s.send_and_wait(
                    req.conversation.clone(),
                    req.thread.clone(),
                    req.broadcast,
                    req.text.to_string(),
                    id,
                    cx,
                )
            });
            cx.spawn(async move |_| {
                let sent = task.await.map_err(|e| match e {
                    // Left in the outbox: the agent must not send it again.
                    ProviderError::Network(m) if m == QUEUED_OFFLINE => BackendError::Offline,
                    e => prov(e),
                });
                let _ = tx.send(sent);
            })
            .detach();
        });
        Box::pin(async move { call.await? })
    }

    fn schedule(
        &self,
        ws: &WorkspaceKey,
        req: ScheduleRequest,
    ) -> BoxFuture<'_, Result<ScheduledMessage, BackendError>> {
        let io = self.io(ws);
        Box::pin(async move {
            io.await?
                .provider
                .schedule(&req.conversation, req.thread, &req.text, req.at.as_second())
                .await
                .map_err(prov)
        })
    }

    fn react(
        &self,
        ws: &WorkspaceKey,
        id: &MessageId,
        emoji: &EmojiRef,
    ) -> BoxFuture<'_, Result<(), BackendError>> {
        let (io, id, emoji) = (self.io(ws), id.clone(), emoji.clone());
        Box::pin(async move {
            io.await?
                .provider
                .react(&id, &emoji, true)
                .await
                .map_err(prov)
        })
    }

    fn set_status(
        &self,
        ws: &WorkspaceKey,
        status: Option<Status>,
    ) -> BoxFuture<'_, Result<(), BackendError>> {
        let io = self.io(ws);
        Box::pin(async move { io.await?.provider.set_status(status).await.map_err(prov) })
    }

    fn web_url(&self, ws: &WorkspaceKey, id: &MessageId) -> Option<String> {
        let (ws, id) = (ws.clone(), id.clone());
        self.hop
            .call_blocking(move |cx| {
                session(cx, &ws).and_then(|s| s.read(cx).provider.web_url(&id))
            })
            .flatten()
    }
}
