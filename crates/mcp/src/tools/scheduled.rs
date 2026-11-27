//! `list_scheduled`: the user's scheduled messages in visible conversations.

use std::collections::HashMap;

use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    audit::AuditSpan, dto::ScheduledOut, errors::ToolFailure, render::capped, server::Shared,
};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListScheduledArgs {
    /// Workspace key as listed by `list_workspaces`; all enabled workspaces when absent.
    pub workspace: Option<String>,
}

#[derive(Serialize)]
struct ScheduledListOut<'a> {
    scheduled: &'a [ScheduledOut],
    truncated: bool,
    partial: bool,
}

impl Shared {
    pub(crate) async fn list_scheduled(
        &self,
        span: &mut AuditSpan,
        args: ListScheduledArgs,
    ) -> Result<CallToolResult, ToolFailure> {
        let (policy, one, targets) = self.scope(span, args.workspace.as_deref()).await?;
        let mut out: Vec<ScheduledOut> = Vec::new();
        let mut partial = false;
        for ws in &targets {
            // Scheduled messages live with the provider; charge the budget.
            if let Err(e) = self.limits.provider_reads(ws, 1) {
                if one {
                    return Err(e.into());
                }
                partial = true;
                continue;
            }
            let all = match self.backend.conversations(ws).await {
                Ok(all) => all,
                Err(e) if one => return Err(e.into()),
                Err(_) => continue,
            };
            let visible: HashMap<_, _> = policy
                .visible(ws, &all)
                .into_iter()
                .map(|c| (c.id.clone(), c))
                .collect();
            let list = match self.backend.scheduled(ws).await {
                Ok(l) => l,
                Err(e) if one => return Err(e.into()),
                Err(_) => continue,
            };
            out.extend(
                list.into_iter()
                    .filter(|s| visible.contains_key(&s.conversation))
                    .map(|s| ScheduledOut {
                        workspace: ws.to_string(),
                        id: s.id.0.to_string(),
                        conversation: s.conversation.0.to_string(),
                        thread_ts: s.thread.as_ref().map(|t| t.0.to_string()),
                        untrusted_text: s.text.to_string(),
                        post_at: s.post_at,
                    }),
            );
        }
        out.sort_by(|a, b| {
            a.post_at
                .cmp(&b.post_at)
                .then_with(|| a.workspace.cmp(&b.workspace))
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(capped(&out, false, |scheduled, truncated| {
            ScheduledListOut {
                scheduled,
                truncated,
                partial,
            }
        }))
    }
}
