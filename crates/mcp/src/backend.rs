//! The seam between the neutral tool server and the app: the app implements
//! [`Backend`] over its workspace sessions, tests over `FakeProvider`s.

use std::sync::Arc;

use domain::{
    Capabilities, Conversation, ConversationId, EmojiRef, HistoryPage, Member, Message, MessageId,
    MessageTs, ProviderError, ScheduledMessage, Status, UserId,
};
use futures::future::BoxFuture;
use settings::WorkspaceKey;

#[derive(Debug, Clone)]
pub struct WorkspaceInfo {
    pub key: WorkspaceKey,
    pub name: Arc<str>,
    pub provider_display: Arc<str>,
    pub self_user: Option<UserId>,
    pub caps: Capabilities,
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum BackendError {
    #[error("not found")]
    NotFound,
    #[error(transparent)]
    Provider(ProviderError),
    #[error("workspace is offline")]
    Offline,
    #[error("app is shutting down")]
    Closed,
}

/// An answer plus how many provider requests it cost (0 when the local store
/// had it), for the rate limiter.
#[derive(Debug, Clone)]
pub struct Fetched<T> {
    pub value: T,
    pub provider_calls: u32,
}

#[derive(Debug, Clone)]
pub struct SendRequest {
    pub conversation: ConversationId,
    pub thread: Option<MessageTs>,
    pub broadcast: bool,
    pub text: Arc<str>,
    pub idempotency_key: Option<Arc<str>>,
}

#[derive(Debug, Clone)]
pub struct ScheduleRequest {
    pub conversation: ConversationId,
    pub thread: Option<MessageTs>,
    pub text: Arc<str>,
    pub at: jiff::Timestamp,
}

/// Everything the tools need from the running app. Reads answer from the
/// local store first. MCP policy is applied above this trait, not in it.
pub trait Backend: Send + Sync + 'static {
    /// Signed-in workspaces; MCP enablement is ignored here.
    fn workspaces(&self) -> Vec<WorkspaceInfo>;
    fn conversations(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<Conversation>, BackendError>>;
    /// Newest first, strictly older than `before`. With `remote` false the
    /// answer comes from the local store alone (possibly short) and costs no
    /// provider request; the tools use it to honour the provider read budget.
    fn messages(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        before: Option<MessageTs>,
        limit: u32,
        remote: bool,
    ) -> BoxFuture<'_, Result<Fetched<HistoryPage>, BackendError>>;
    /// Ascending, root first when `after` is `None`. `remote` as in
    /// [`Backend::messages`].
    fn thread(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        root: &MessageTs,
        after: Option<MessageTs>,
        limit: u32,
        remote: bool,
    ) -> BoxFuture<'_, Result<Fetched<HistoryPage>, BackendError>>;
    /// Local full text first; `remote` also asks the provider.
    fn search(
        &self,
        ws: &WorkspaceKey,
        query: &str,
        limit: u32,
        remote: bool,
    ) -> BoxFuture<'_, Result<Fetched<Vec<Message>>, BackendError>>;
    fn member(
        &self,
        ws: &WorkspaceKey,
        id: &UserId,
    ) -> BoxFuture<'_, Result<Option<Member>, BackendError>>;
    fn search_members(
        &self,
        ws: &WorkspaceKey,
        query: &str,
        limit: u32,
    ) -> BoxFuture<'_, Result<Vec<Member>, BackendError>>;
    fn scheduled(
        &self,
        ws: &WorkspaceKey,
    ) -> BoxFuture<'_, Result<Vec<ScheduledMessage>, BackendError>>;
    fn send(
        &self,
        ws: &WorkspaceKey,
        req: SendRequest,
    ) -> BoxFuture<'_, Result<Message, BackendError>>;
    fn schedule(
        &self,
        ws: &WorkspaceKey,
        req: ScheduleRequest,
    ) -> BoxFuture<'_, Result<ScheduledMessage, BackendError>>;
    fn react(
        &self,
        ws: &WorkspaceKey,
        id: &MessageId,
        emoji: &EmojiRef,
    ) -> BoxFuture<'_, Result<(), BackendError>>;
    fn set_status(
        &self,
        ws: &WorkspaceKey,
        status: Option<Status>,
    ) -> BoxFuture<'_, Result<(), BackendError>>;
    fn web_url(&self, ws: &WorkspaceKey, id: &MessageId) -> Option<String>;
}
