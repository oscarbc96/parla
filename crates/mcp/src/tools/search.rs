//! `search_messages`: local full text first, then the provider only when the
//! local hits fall short, the caller allows it and the provider budget has
//! room. Every hit passes the conversation policy before it counts.

use std::collections::HashMap;

use domain::{Conversation, Message};
use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use settings::WorkspaceKey;

use crate::{
    audit::AuditSpan,
    dto::{AuthorOut, SearchHitOut},
    errors::ToolFailure,
    render::{Names, capped, conversation_out, truncate_text},
    server::Shared,
};

const QUERY_MAX_CHARS: usize = 256;
const LIMIT_DEFAULT: u32 = 20;
const LIMIT_MAX: u32 = 50;
const SNIPPET_CHARS: usize = 300;
/// Hits asked from the backend per workspace, so hidden ones can be dropped
/// without starving the page.
const FETCH_FACTOR: u32 = 4;
const FETCH_MAX: u32 = 200;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchMessagesArgs {
    /// Words to look for (1 to 256 characters).
    pub query: String,
    /// Workspace key as listed by `list_workspaces`; all enabled workspaces when absent.
    pub workspace: Option<String>,
    /// At most this many hits (default 20, max 50).
    pub limit: Option<u32>,
    /// Also ask the chat service when local results fall short (default true).
    pub remote: Option<bool>,
}

#[derive(Serialize)]
struct SearchHitsOut<'a> {
    hits: &'a [SearchHitOut],
    truncated: bool,
    partial: bool,
}

impl Shared {
    pub(crate) async fn search_messages(
        &self,
        span: &mut AuditSpan,
        args: SearchMessagesArgs,
    ) -> Result<CallToolResult, ToolFailure> {
        let query = args.query.trim().to_string();
        if query.is_empty() || query.chars().count() > QUERY_MAX_CHARS {
            return Err(ToolFailure::bad_argument(format!(
                "query must be 1 to {QUERY_MAX_CHARS} characters"
            )));
        }
        span.content(|| query.clone());
        let (policy, one, targets) = self.scope(span, args.workspace.as_deref()).await?;
        let limit = args.limit.unwrap_or(LIMIT_DEFAULT).clamp(1, LIMIT_MAX);
        let fetch = (limit * FETCH_FACTOR).min(FETCH_MAX);
        let remote = args.remote.unwrap_or(true);

        let mut partial = false;
        let mut found: Vec<(WorkspaceKey, Message, Conversation)> = Vec::new();
        for ws in &targets {
            let all = match self.backend.conversations(ws).await {
                Ok(all) => all,
                Err(e) if one => return Err(e.into()),
                Err(_) => continue,
            };
            let by_id: HashMap<_, _> = all.iter().map(|c| (c.id.clone(), c)).collect();
            let keep = |msgs: Vec<Message>| -> Vec<(Message, Conversation)> {
                msgs.into_iter()
                    .filter_map(|m| {
                        let c = by_id.get(&m.id.conversation)?;
                        policy.conversation(ws, c).ok()?;
                        Some((m, (*c).clone()))
                    })
                    .collect()
            };
            let local = match self.backend.search(ws, &query, fetch, false).await {
                Ok(f) => f.value,
                Err(e) if one => return Err(e.into()),
                Err(_) => continue,
            };
            let mut hits = keep(local);
            if remote && hits.len() < limit as usize {
                if !self.spend_provider_read(ws) {
                    partial = true;
                } else {
                    match self.backend.search(ws, &query, fetch, true).await {
                        Ok(f) => {
                            self.settle_provider_calls(ws, f.provider_calls);
                            hits = keep(f.value);
                        }
                        Err(_) => partial = true,
                    }
                }
            }
            found.extend(hits.into_iter().map(|(m, c)| (ws.clone(), m, c)));
        }
        found.sort_by(|(wa, a, _), (wb, b, _)| {
            b.id.ts
                .cmp(&a.id.ts)
                .then_with(|| wa.to_string().cmp(&wb.to_string()))
        });
        found.dedup_by(|(wa, a, _), (wb, b, _)| wa == wb && a.id == b.id);
        let more = found.len() > limit as usize;
        found.truncate(limit as usize);

        let mut names: HashMap<WorkspaceKey, Names> = HashMap::new();
        let mut out = Vec::with_capacity(found.len());
        for (ws, m, c) in &found {
            let ws_names = names.entry(ws.clone()).or_default();
            self.resolve_names(ws, std::slice::from_ref(m), ws_names)
                .await;
            out.push(SearchHitOut {
                workspace: ws.to_string(),
                conversation: m.id.conversation.0.to_string(),
                untrusted_conversation_name: conversation_out(ws, c).untrusted_name,
                ts: m.id.ts.0.to_string(),
                thread_ts: m.thread_root.as_ref().map(|t| t.0.to_string()),
                author: m.author.as_ref().map(|u| AuthorOut {
                    user: u.0.to_string(),
                    untrusted_name: ws_names.get(u).cloned().unwrap_or_else(|| u.0.to_string()),
                }),
                untrusted_snippet: truncate_text(&m.plain, SNIPPET_CHARS).0,
                link: self.web_url(ws, &m.id).await,
            });
        }
        Ok(capped(&out, more, |hits, truncated| SearchHitsOut {
            hits,
            truncated,
            partial,
        }))
    }
}
