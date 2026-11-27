//! `FakeBackend`: a [`Backend`] over `FakeProvider`s and a `MemoryStore`,
//! store first then provider, counting provider requests.

use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};

use domain::{
    ChatProvider, ClientMsgId, Conversation, ConversationId, EmojiRef, HistoryPage, Member,
    Message, MessageId, MessageTs, PersistOp, ProviderError, ScheduledMessage, SearchQuery, Status,
    UserId, test_support::FakeProvider,
};
use futures::future::BoxFuture;
use settings::WorkspaceKey;
use store::{MemoryStore, Store, StoreError};

use crate::backend::{Backend, BackendError, Fetched, ScheduleRequest, SendRequest, WorkspaceInfo};

pub struct FakeBackend {
    providers: Vec<Arc<FakeProvider>>,
    store: Arc<MemoryStore>,
    sent: Mutex<Vec<(WorkspaceKey, SendRequest)>>,
    next_id: AtomicU64,
    /// Provider reads asked for with `remote` set, counted when the backend
    /// method is called: the app enqueues main-thread work there, before the
    /// returned future runs.
    remote_reads: AtomicUsize,
    /// Milliseconds the sync methods block, like the app waiting on its main thread.
    sync_delay_ms: AtomicU64,
}

impl FakeBackend {
    pub fn new(providers: Vec<Arc<FakeProvider>>) -> Self {
        Self {
            providers,
            store: Arc::new(MemoryStore::default()),
            sent: Mutex::new(Vec::new()),
            next_id: AtomicU64::new(1),
            remote_reads: AtomicUsize::new(0),
            sync_delay_ms: AtomicU64::new(0),
        }
    }

    /// How many provider reads the tools asked for with `remote` set.
    pub fn remote_reads(&self) -> usize {
        self.remote_reads.load(Ordering::Relaxed)
    }

    /// Makes `workspaces` and `web_url` block for `ms`, as the app does.
    pub fn set_sync_delay_ms(&self, ms: u64) {
        self.sync_delay_ms.store(ms, Ordering::Relaxed);
    }

    fn block(&self) {
        let ms = self.sync_delay_ms.load(Ordering::Relaxed);
        if ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
    }

    /// The local store the reads try first; fill it to simulate a warm cache.
    pub fn store(&self) -> &Arc<MemoryStore> {
        &self.store
    }

    /// Sends that reached a provider, in order.
    pub fn sent(&self) -> Vec<(WorkspaceKey, SendRequest)> {
        self.sent.lock().expect("sent lock").clone()
    }

    fn provider(&self, ws: &WorkspaceKey) -> Result<&FakeProvider, BackendError> {
        self.providers
            .iter()
            .find(|p| p.workspace() == ws)
            .map(|p| &**p)
            .ok_or(BackendError::NotFound)
    }
}

fn store_err(e: StoreError) -> BackendError {
    match e {
        StoreError::Closed => BackendError::Closed,
        other => BackendError::Provider(ProviderError::Other(other.to_string())),
    }
}

fn prov_err(e: ProviderError) -> BackendError {
    BackendError::Provider(e)
}

impl Backend for FakeBackend {
    fn workspaces(&self) -> Vec<WorkspaceInfo> {
        self.block();
        self.providers
            .iter()
            .map(|p| WorkspaceInfo {
                key: p.workspace().clone(),
                name: Arc::from(p.workspace().id.0.as_str()),
                provider_display: Arc::from("Fake"),
                self_user: None,
                caps: p.capabilities().clone(),
            })
            .collect()
    }

    fn conversations(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<Conversation>, BackendError>> {
        let ws = ws.clone();
        Box::pin(async move {
            let p = self.provider(&ws)?;
            let cached = self.store.conversations(&ws).await.map_err(store_err)?;
            if !cached.is_empty() {
                return Ok(cached);
            }
            let boot = p.bootstrap().await.map_err(prov_err)?;
            self.store
                .apply(&ws, PersistOp::Conversations(boot.conversations.clone()))
                .await
                .map_err(store_err)?;
            Ok(boot.conversations)
        })
    }

    fn messages(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        before: Option<MessageTs>,
        limit: u32,
        remote: bool,
    ) -> BoxFuture<'_, Result<Fetched<HistoryPage>, BackendError>> {
        if remote {
            self.remote_reads.fetch_add(1, Ordering::Relaxed);
        }
        let (ws, c) = (ws.clone(), c.clone());
        Box::pin(async move {
            let p = self.provider(&ws)?;
            let cached = self
                .store
                .messages_before(&ws, &c, before.clone(), limit as usize)
                .await
                .map_err(store_err)?;
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
            let mut page = p.history(&c, before, limit).await.map_err(prov_err)?;
            self.store
                .apply(
                    &ws,
                    PersistOp::Messages {
                        conversation: c,
                        messages: page.messages.clone(),
                    },
                )
                .await
                .map_err(store_err)?;
            page.messages.sort_by(|a, b| b.id.ts.cmp(&a.id.ts));
            Ok(Fetched {
                value: page,
                provider_calls: 1,
            })
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
        if remote {
            self.remote_reads.fetch_add(1, Ordering::Relaxed);
        }
        let (ws, c, root) = (ws.clone(), c.clone(), root.clone());
        Box::pin(async move {
            let p = self.provider(&ws)?;
            if after.is_none() {
                let cached = self
                    .store
                    .thread(&ws, &c, &root, limit as usize)
                    .await
                    .map_err(store_err)?;
                let complete = cached.first().is_some_and(|r| {
                    r.id.ts == root && cached.len() as u32 > r.reply_count.min(limit - 1)
                });
                if complete {
                    return Ok(Fetched {
                        value: HistoryPage {
                            messages: cached,
                            has_more: false,
                        },
                        provider_calls: 0,
                    });
                }
            }
            if !remote {
                let cached = self
                    .store
                    .thread(&ws, &c, &root, limit as usize)
                    .await
                    .map_err(store_err)?;
                let messages = if after.is_none() { cached } else { Vec::new() };
                return Ok(Fetched {
                    value: HistoryPage {
                        messages,
                        has_more: false,
                    },
                    provider_calls: 0,
                });
            }
            let page = p.replies(&c, &root, after, limit).await.map_err(prov_err)?;
            Ok(Fetched {
                value: page,
                provider_calls: 1,
            })
        })
    }

    fn search(
        &self,
        ws: &WorkspaceKey,
        query: &str,
        limit: u32,
        remote: bool,
    ) -> BoxFuture<'_, Result<Fetched<Vec<Message>>, BackendError>> {
        let (ws, query) = (ws.clone(), query.to_string());
        Box::pin(async move {
            let p = self.provider(&ws)?;
            let hits = self
                .store
                .search(&ws, &query, limit as usize)
                .await
                .map_err(store_err)?;
            let mut out = Vec::new();
            for h in hits {
                if let Some(m) = self
                    .store
                    .message(&ws, &h.id.conversation, &h.id.ts)
                    .await
                    .map_err(store_err)?
                {
                    out.push(m);
                }
            }
            let mut calls = 0;
            if remote && out.len() < limit as usize && p.capabilities().search {
                let q = SearchQuery {
                    text: Arc::from(query.as_str()),
                    conversations: Vec::new(),
                    authors: Vec::new(),
                    before: None,
                    after: None,
                    has: Vec::new(),
                };
                let page = p.search_messages(&q, None, limit).await.map_err(prov_err)?;
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
        })
    }

    fn member(
        &self,
        ws: &WorkspaceKey,
        id: &UserId,
    ) -> BoxFuture<'_, Result<Option<Member>, BackendError>> {
        let (ws, id) = (ws.clone(), id.clone());
        Box::pin(async move {
            let p = self.provider(&ws)?;
            let cached = self.store.members(&ws).await.map_err(store_err)?;
            if let Some(m) = cached.into_iter().find(|m| m.id == id) {
                return Ok(Some(m));
            }
            // A provider that cannot look users up knows only the cached ones.
            let found = match p.members(std::slice::from_ref(&id)).await {
                Ok(found) => found,
                Err(ProviderError::Unsupported) => Vec::new(),
                Err(e) => return Err(prov_err(e)),
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
        Box::pin(async move {
            self.provider(&ws)?;
            let all = self.store.members(&ws).await.map_err(store_err)?;
            Ok(all
                .into_iter()
                .filter(|m| {
                    m.display_name.to_lowercase().contains(&q)
                        || m.real_name.to_lowercase().contains(&q)
                })
                .take(limit as usize)
                .collect())
        })
    }

    fn scheduled(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<ScheduledMessage>, BackendError>> {
        let ws = ws.clone();
        Box::pin(async move { self.provider(&ws)?.scheduled().await.map_err(prov_err) })
    }

    fn send(
        &self,
        ws: &WorkspaceKey,
        req: SendRequest,
    ) -> BoxFuture<'_, Result<Message, BackendError>> {
        let ws = ws.clone();
        Box::pin(async move {
            let p = self.provider(&ws)?;
            let id = ClientMsgId(match &req.idempotency_key {
                Some(k) => k.clone(),
                None => Arc::from(format!(
                    "fake-{}",
                    self.next_id.fetch_add(1, Ordering::Relaxed)
                )),
            });
            let msg = match &req.thread {
                Some(root) => {
                    p.reply(&req.conversation, root, &req.text, req.broadcast, id)
                        .await
                }
                None => p.send(&req.conversation, &req.text, None, id).await,
            }
            .map_err(prov_err)?;
            self.sent.lock().expect("sent lock").push((ws, req));
            Ok(msg)
        })
    }

    fn schedule(
        &self,
        ws: &WorkspaceKey,
        req: ScheduleRequest,
    ) -> BoxFuture<'_, Result<ScheduledMessage, BackendError>> {
        let ws = ws.clone();
        Box::pin(async move {
            self.provider(&ws)?
                .schedule(&req.conversation, req.thread, &req.text, req.at.as_second())
                .await
                .map_err(prov_err)
        })
    }

    fn react(
        &self,
        ws: &WorkspaceKey,
        id: &MessageId,
        emoji: &EmojiRef,
    ) -> BoxFuture<'_, Result<(), BackendError>> {
        let (ws, id, emoji) = (ws.clone(), id.clone(), emoji.clone());
        Box::pin(async move {
            self.provider(&ws)?
                .react(&id, &emoji, true)
                .await
                .map_err(prov_err)
        })
    }

    fn set_status(
        &self,
        ws: &WorkspaceKey,
        status: Option<Status>,
    ) -> BoxFuture<'_, Result<(), BackendError>> {
        let ws = ws.clone();
        Box::pin(async move {
            self.provider(&ws)?
                .set_status(status)
                .await
                .map_err(prov_err)
        })
    }

    fn web_url(&self, ws: &WorkspaceKey, id: &MessageId) -> Option<String> {
        self.block();
        self.provider(ws).ok()?.web_url(id)
    }
}
