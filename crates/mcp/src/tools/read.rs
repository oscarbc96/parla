//! The read tools: `list_workspaces`, `list_channels`, `get_unreads`,
//! `read_channel` and `read_thread`. Each handler runs after the audit span
//! was opened and the read limit was charged (see `server`), applies the
//! policy, then asks the backend.

use std::{cmp::Reverse, collections::HashMap, future::Future};

use domain::{Conversation, ConversationId, HistoryPage, Message, MessageTs};
use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use settings::WorkspaceKey;

use crate::{
    audit::{AuditSpan, target_conversation, target_message},
    backend::{BackendError, Fetched},
    dto::{ConversationOut, MessageOut, PageOut, UnreadOut, WorkspaceOut},
    errors::ToolFailure,
    ids::{Cursor, parse_timestamp, parse_workspace},
    policy::Policy,
    render::{Names, capped, conversation_out, fit, json_len, kind_name, message_out, ok_result},
    server::Shared,
};

const LIST_DEFAULT: u32 = 200;
const LIST_MAX: u32 = 1_000;
const UNREAD_DEFAULT: u32 = 5;
const UNREAD_MAX: u32 = 20;
/// Conversations `get_unreads` reads messages for in one call.
const UNREAD_CONVERSATIONS_MAX: usize = 50;
const CHANNEL_DEFAULT: u32 = 20;
const CHANNEL_MAX: u32 = 100;
const THREAD_DEFAULT: u32 = 50;
const THREAD_MAX: u32 = 200;

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListWorkspacesArgs {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum KindArg {
    Channel,
    Private,
    Dm,
    Group,
}

impl KindArg {
    fn name(self) -> &'static str {
        match self {
            KindArg::Channel => "channel",
            KindArg::Private => "private",
            KindArg::Dm => "dm",
            KindArg::Group => "group",
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListChannelsArgs {
    /// Workspace key as listed by `list_workspaces`; all enabled workspaces when absent.
    pub workspace: Option<String>,
    /// Only conversations with unread messages.
    pub unread_only: Option<bool>,
    /// Only this kind of conversation.
    pub kind: Option<KindArg>,
    /// At most this many (default 200, max 1000).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetUnreadsArgs {
    /// Workspace key as listed by `list_workspaces`; all enabled workspaces when absent.
    pub workspace: Option<String>,
    /// Newest unread messages per conversation (default 5, max 20).
    pub per_conversation: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadChannelArgs {
    /// Workspace key as listed by `list_workspaces`.
    pub workspace: String,
    /// Conversation id as listed by `list_channels`.
    pub conversation: String,
    /// `next_cursor` of the previous page, unchanged.
    pub cursor: Option<String>,
    /// Messages per page, newest first (default 20, max 100).
    pub limit: Option<u32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReadThreadArgs {
    /// Workspace key as listed by `list_workspaces`.
    pub workspace: String,
    /// Conversation id as listed by `list_channels`.
    pub conversation: String,
    /// Timestamp of the thread's root message.
    pub ts: String,
    /// `next_cursor` of the previous page, unchanged.
    pub cursor: Option<String>,
    /// Messages per page, oldest first (default 50, max 200).
    pub limit: Option<u32>,
}

#[derive(Serialize)]
struct WorkspacesOut {
    workspaces: Vec<WorkspaceOut>,
}

#[derive(Serialize)]
struct PageRef<'a> {
    items: &'a [MessageOut],
    next_cursor: Option<&'a str>,
    truncated: bool,
    partial: bool,
}

#[derive(Serialize)]
struct ChannelsOut<'a> {
    conversations: &'a [ConversationOut],
    truncated: bool,
}

#[derive(Serialize)]
struct UnreadsRef<'a> {
    conversations: &'a [UnreadOut],
    truncated: bool,
    partial: bool,
}

fn clamp(v: Option<u32>, default: u32, max: u32) -> u32 {
    v.unwrap_or(default).clamp(1, max)
}

/// A cursor placeholder as long as a real one, for sizing.
fn cursor_pad() -> String {
    "x".repeat(64)
}

impl Shared {
    /// Spends one read of `ws`'s provider budget; `false` when it is used up.
    pub(crate) fn spend_provider_read(&self, ws: &WorkspaceKey) -> bool {
        self.limits.provider_reads(ws, 1).is_ok()
    }

    /// Charges the calls a fetch made beyond the first one
    /// [`Self::spend_provider_read`] already paid for.
    pub(crate) fn settle_provider_calls(&self, ws: &WorkspaceKey, calls: u32) {
        if calls > 1 {
            let _ = self.limits.provider_reads(ws, calls - 1);
        }
    }

    /// Every signed-in workspace. The backend answers synchronously by waiting
    /// on the app's main thread, so it runs off the async workers.
    pub(crate) async fn workspaces(&self) -> Vec<crate::backend::WorkspaceInfo> {
        let backend = self.backend.clone();
        tokio::task::spawn_blocking(move || backend.workspaces())
            .await
            .unwrap_or_default()
    }

    pub(crate) async fn web_url(
        &self,
        ws: &WorkspaceKey,
        id: &domain::MessageId,
    ) -> Option<String> {
        let backend = self.backend.clone();
        let (ws, id) = (ws.clone(), id.clone());
        tokio::task::spawn_blocking(move || backend.web_url(&ws, &id))
            .await
            .ok()
            .flatten()
    }

    /// Web links of `msgs`, in order, looked up in one blocking hop.
    pub(crate) async fn web_urls(
        &self,
        ws: &WorkspaceKey,
        msgs: &[Message],
    ) -> Vec<Option<String>> {
        let backend = self.backend.clone();
        let (ws, ids): (WorkspaceKey, Vec<_>) =
            (ws.clone(), msgs.iter().map(|m| m.id.clone()).collect());
        let n = ids.len();
        tokio::task::spawn_blocking(move || ids.iter().map(|id| backend.web_url(&ws, id)).collect())
            .await
            .unwrap_or_else(|_| vec![None; n])
    }

    /// The workspaces a call may look at: the named one, or every enabled one.
    pub(crate) async fn target_workspaces(
        &self,
        policy: &Policy,
        arg: Option<&str>,
    ) -> Result<Vec<WorkspaceKey>, ToolFailure> {
        match arg {
            Some(s) => {
                let ws = parse_workspace(s)?;
                policy.workspace(&ws)?;
                Ok(vec![ws])
            }
            None => Ok(self
                .workspaces()
                .await
                .into_iter()
                .map(|w| w.key)
                .filter(|k| policy.workspace(k).is_ok())
                .collect()),
        }
    }

    /// The policy and the workspaces a call may look at: the named one, or
    /// every enabled one. Names the workspace in the span when there is one,
    /// and says whether the caller named it.
    pub(crate) async fn scope(
        &self,
        span: &mut AuditSpan,
        arg: Option<&str>,
    ) -> Result<(Policy, bool, Vec<WorkspaceKey>), ToolFailure> {
        let policy = Policy::from_source(&*self.settings);
        let one = arg.is_some();
        let targets = self.target_workspaces(&policy, arg).await?;
        if let [ws] = targets.as_slice() {
            span.workspace(ws.clone());
        }
        Ok((policy, one, targets))
    }

    /// A conversation the client may see. Unknown and hidden fail alike.
    pub(crate) async fn visible_conversation(
        &self,
        policy: &Policy,
        ws: &WorkspaceKey,
        id: &str,
    ) -> Result<Conversation, ToolFailure> {
        policy.workspace(ws)?;
        let all = self.backend.conversations(ws).await?;
        let c = all
            .into_iter()
            .find(|c| &*c.id.0 == id)
            .ok_or_else(ToolFailure::unknown)?;
        policy.conversation(ws, &c)?;
        Ok(c)
    }

    /// Display names for the authors of `msgs`, added to `names`.
    pub(crate) async fn resolve_names(
        &self,
        ws: &WorkspaceKey,
        msgs: &[Message],
        names: &mut Names,
    ) {
        for m in msgs {
            let Some(u) = &m.author else { continue };
            if names.contains_key(u) {
                continue;
            }
            if let Ok(Some(member)) = self.backend.member(ws, u).await {
                let name = if member.display_name.is_empty() {
                    member.real_name.to_string()
                } else {
                    member.display_name.to_string()
                };
                names.insert(u.clone(), name);
            }
        }
    }

    async fn render_messages(
        &self,
        ws: &WorkspaceKey,
        msgs: &[Message],
        names: &Names,
    ) -> Vec<MessageOut> {
        let links = self.web_urls(ws, msgs).await;
        msgs.iter()
            .zip(links)
            .map(|(m, link)| message_out(ws, m, names, link))
            .collect()
    }

    /// The cached page when `covered`; else one provider read when the budget
    /// allows. The flag is true when the budget stopped the provider part.
    /// `remote` is only called once both checks pass: constructing the app's
    /// backend call enqueues work on its main thread, so it must stay lazy.
    async fn page_or_fetch<F, Fut>(
        &self,
        ws: &WorkspaceKey,
        cached: HistoryPage,
        covered: bool,
        remote: F,
    ) -> Result<(HistoryPage, bool), ToolFailure>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<Fetched<HistoryPage>, BackendError>>,
    {
        if covered {
            return Ok((cached, false));
        }
        if !self.spend_provider_read(ws) {
            return Ok((cached, true));
        }
        let fetched = remote().await?;
        self.settle_provider_calls(ws, fetched.provider_calls);
        Ok((fetched.value, false))
    }

    /// Newest-first history: the store first, then the provider when the
    /// cached part is not `covered` and the provider budget allows. The flag
    /// is true when the budget stopped the provider part.
    async fn history(
        &self,
        ws: &WorkspaceKey,
        c: &ConversationId,
        before: Option<MessageTs>,
        limit: u32,
        covered: impl Fn(&[Message]) -> bool,
    ) -> Result<(HistoryPage, bool), ToolFailure> {
        let cached = self
            .backend
            .messages(ws, c, before.clone(), limit, false)
            .await?
            .value;
        let covered = covered(&cached.messages);
        let remote = || self.backend.messages(ws, c, before, limit, true);
        self.page_or_fetch(ws, cached, covered, remote).await
    }

    pub(crate) async fn list_workspaces(
        &self,
        _span: &mut AuditSpan,
    ) -> Result<CallToolResult, ToolFailure> {
        let policy = Policy::from_source(&*self.settings);
        let mut workspaces = Vec::new();
        for w in self.workspaces().await {
            let Ok(s) = policy.workspace(&w.key) else {
                continue;
            };
            workspaces.push(WorkspaceOut {
                workspace: w.key.to_string(),
                untrusted_name: w.name.to_string(),
                provider: w.provider_display.to_string(),
                allow_writes: s.allow_writes,
                confirm_writes: s.confirm_writes,
            });
        }
        Ok(ok_result(&WorkspacesOut { workspaces }))
    }

    pub(crate) async fn list_channels(
        &self,
        span: &mut AuditSpan,
        args: ListChannelsArgs,
    ) -> Result<CallToolResult, ToolFailure> {
        let (policy, one, targets) = self.scope(span, args.workspace.as_deref()).await?;
        let limit = clamp(args.limit, LIST_DEFAULT, LIST_MAX) as usize;
        let mut found: Vec<(WorkspaceKey, Conversation)> = Vec::new();
        for ws in &targets {
            let all = match self.backend.conversations(ws).await {
                Ok(all) => all,
                Err(e) if one => return Err(e.into()),
                Err(_) => continue,
            };
            for c in policy.visible(ws, &all) {
                if args.unread_only == Some(true) && !(c.unread || c.mentions > 0) {
                    continue;
                }
                if let Some(k) = args.kind
                    && kind_name(c.kind) != k.name()
                {
                    continue;
                }
                found.push((ws.clone(), c.clone()));
            }
        }
        found.sort_by(|(wa, a), (wb, b)| {
            let unread = |c: &Conversation| c.unread || c.mentions > 0;
            unread(b)
                .cmp(&unread(a))
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
                .then_with(|| wa.to_string().cmp(&wb.to_string()))
                .then_with(|| a.id.0.cmp(&b.id.0))
        });
        let more = found.len() > limit;
        found.truncate(limit);
        let out: Vec<ConversationOut> = found
            .iter()
            .map(|(ws, c)| conversation_out(ws, c))
            .collect();
        Ok(capped(&out, more, |conversations, truncated| ChannelsOut {
            conversations,
            truncated,
        }))
    }

    pub(crate) async fn get_unreads(
        &self,
        span: &mut AuditSpan,
        args: GetUnreadsArgs,
    ) -> Result<CallToolResult, ToolFailure> {
        let (policy, one, targets) = self.scope(span, args.workspace.as_deref()).await?;
        let per = clamp(args.per_conversation, UNREAD_DEFAULT, UNREAD_MAX);
        let mut entries: Vec<(WorkspaceKey, Conversation)> = Vec::new();
        for ws in &targets {
            let all = match self.backend.conversations(ws).await {
                Ok(all) => all,
                Err(e) if one => return Err(e.into()),
                Err(_) => continue,
            };
            for c in policy.visible(ws, &all) {
                if (c.unread || c.mentions > 0) && !c.is_archived {
                    entries.push((ws.clone(), c.clone()));
                }
            }
        }
        // Mentions first (most, then newest), then the rest newest first.
        entries.sort_by_key(|(_, c)| {
            (
                c.mentions == 0,
                Reverse(c.mentions),
                Reverse(c.latest.clone()),
            )
        });
        let truncated = entries.len() > UNREAD_CONVERSATIONS_MAX;
        entries.truncate(UNREAD_CONVERSATIONS_MAX);

        let mut partial = false;
        let mut names: HashMap<WorkspaceKey, Names> = HashMap::new();
        let mut out: Vec<UnreadOut> = Vec::new();
        for (ws, c) in &entries {
            let last_read = c.last_read.clone();
            let covered = |ms: &[Message]| {
                ms.len() >= per as usize
                    || match (&last_read, ms.last()) {
                        (Some(lr), Some(oldest)) => &oldest.id.ts <= lr,
                        _ => false,
                    }
            };
            let (page, was_partial) = self.history(ws, &c.id, None, per, covered).await?;
            partial |= was_partial;
            let unread: Vec<Message> = page
                .messages
                .into_iter()
                .filter(|m| last_read.as_ref().is_none_or(|lr| &m.id.ts > lr))
                .take(per as usize)
                .collect();
            let ws_names = names.entry(ws.clone()).or_default();
            self.resolve_names(ws, &unread, ws_names).await;
            out.push(UnreadOut {
                conversation: conversation_out(ws, c),
                messages: self.render_messages(ws, &unread, ws_names).await,
            });
        }
        Ok(capped(&out, truncated, |conversations, truncated| {
            UnreadsRef {
                conversations,
                truncated,
                partial,
            }
        }))
    }

    pub(crate) async fn read_channel(
        &self,
        span: &mut AuditSpan,
        args: ReadChannelArgs,
    ) -> Result<CallToolResult, ToolFailure> {
        let policy = Policy::from_source(&*self.settings);
        let ws = parse_workspace(&args.workspace)?;
        span.workspace(ws.clone());
        span.target(target_conversation(&args.conversation));
        let before = args
            .cursor
            .as_deref()
            .map(Cursor::decode)
            .transpose()?
            .map(|c| c.before);
        let limit = clamp(args.limit, CHANNEL_DEFAULT, CHANNEL_MAX);
        let c = self
            .visible_conversation(&policy, &ws, &args.conversation)
            .await?;
        let (page, partial) = self
            .history(&ws, &c.id, before, limit, |ms| ms.len() >= limit as usize)
            .await?;
        let mut msgs = page.messages;
        msgs.sort_by(|a, b| b.id.ts.cmp(&a.id.ts));
        let more = page.has_more || msgs.len() > limit as usize;
        msgs.truncate(limit as usize);
        let mut names = Names::new();
        self.resolve_names(&ws, &msgs, &mut names).await;
        let items = self.render_messages(&ws, &msgs, &names).await;
        Ok(self.page_result(items, more, partial))
    }

    pub(crate) async fn read_thread(
        &self,
        span: &mut AuditSpan,
        args: ReadThreadArgs,
    ) -> Result<CallToolResult, ToolFailure> {
        let policy = Policy::from_source(&*self.settings);
        let ws = parse_workspace(&args.workspace)?;
        span.workspace(ws.clone());
        let root = parse_timestamp(&args.ts)?;
        span.target(target_message(&args.conversation, &root.0));
        // For a thread the cursor marks the last message already returned.
        let after = args
            .cursor
            .as_deref()
            .map(Cursor::decode)
            .transpose()?
            .map(|c| c.before);
        let limit = clamp(args.limit, THREAD_DEFAULT, THREAD_MAX);
        let c = self
            .visible_conversation(&policy, &ws, &args.conversation)
            .await?;

        let cached = self
            .backend
            .thread(&ws, &c.id, &root, after.clone(), limit, false)
            .await?
            .value;
        let covered = cached.messages.len() >= limit as usize;
        let remote = || self.backend.thread(&ws, &c.id, &root, after, limit, true);
        let (page, partial) = self.page_or_fetch(&ws, cached, covered, remote).await?;
        let mut msgs = page.messages;
        msgs.sort_by(|a, b| a.id.ts.cmp(&b.id.ts));
        let more = page.has_more || msgs.len() > limit as usize;
        msgs.truncate(limit as usize);
        let mut names = Names::new();
        self.resolve_names(&ws, &msgs, &mut names).await;
        let items = self.render_messages(&ws, &msgs, &names).await;
        Ok(self.page_result(items, more, partial))
    }

    /// Caps the page at the result size, drops from the end and points the
    /// cursor at the last kept item so nothing is skipped.
    fn page_result(&self, mut items: Vec<MessageOut>, more: bool, partial: bool) -> CallToolResult {
        let pad = cursor_pad();
        let k = fit(items.len(), |k| {
            json_len(&PageRef {
                items: &items[..k],
                next_cursor: Some(&pad),
                truncated: true,
                partial,
            })
        });
        let cut = k < items.len();
        items.truncate(k);
        let next_cursor = (more || cut).then(|| items.last()).flatten().map(|m| {
            Cursor {
                before: MessageTs(m.ts.as_str().into()),
            }
            .encode()
        });
        ok_result(&PageOut {
            items,
            next_cursor,
            truncated: cut,
            partial,
        })
    }
}
