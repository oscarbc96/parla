//! The write tools: `send_message`, `schedule_message`, `add_reaction` and
//! `set_status`. Order for every write: the write limit (in `server`), the
//! policy (workspace, target visibility, writes allowed), argument checks,
//! the idempotency replay, the user's confirmation, a second policy check,
//! then the backend. Nothing reaches the user or the provider for a request
//! that is hidden, invalid or already answered.

use std::{sync::Arc, time::Instant};

use domain::{Conversation, EmojiRef, Message, MessageId, MessageTs, Status};
use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;
use settings::WorkspaceKey;

use crate::{
    audit::{AuditSpan, ClientInfo, target_conversation, target_message},
    backend::{ScheduleRequest, SendRequest, WorkspaceInfo},
    confirm::{Confirmation, WriteAction, WriteRequest},
    dto::{ReactionOut, ScheduledOut, SendOut, StatusOut},
    errors::ToolFailure,
    idempotency::MAX_KEY_LEN,
    ids::{parse_timestamp, parse_workspace},
    limits::PendingGuard,
    policy::{Policy, SessionScope, WriteMode},
    render::{ok_result, truncate_text},
    server::Shared,
};
use store::WriteDecision;

const TEXT_MAX_CHARS: usize = 12_000;
const STATUS_TEXT_MAX_CHARS: usize = 100;
const EMOJI_MAX_CHARS: usize = 64;
const EXCERPT_CHARS: usize = 200;
/// How long the prompt waits for the user's own name before showing "You".
const AS_USER_LOOKUP: std::time::Duration = std::time::Duration::from_millis(250);

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendMessageArgs {
    /// Workspace key as listed by `list_workspaces`.
    pub workspace: String,
    /// Conversation id as listed by `list_channels`.
    pub conversation: String,
    /// The message text (1 to 12,000 characters), sent as typed.
    pub text: String,
    /// Reply in the thread of the message with this timestamp.
    pub thread_ts: Option<String>,
    /// With `thread_ts`: also show the reply in the conversation.
    pub also_send_to_conversation: Option<bool>,
    /// A key you choose (up to 64 characters): repeating the call with the
    /// same key returns the first result instead of sending again.
    pub idempotency_key: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ScheduleMessageArgs {
    /// Workspace key as listed by `list_workspaces`.
    pub workspace: String,
    /// Conversation id as listed by `list_channels`.
    pub conversation: String,
    /// The message text (1 to 12,000 characters), sent as typed.
    pub text: String,
    /// When to send, RFC 3339 with an offset, such as `2030-01-31T09:00:00+01:00`.
    pub at: String,
    /// Schedule a reply in the thread of the message with this timestamp.
    pub thread_ts: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddReactionArgs {
    /// Workspace key as listed by `list_workspaces`.
    pub workspace: String,
    /// Conversation id as listed by `list_channels`.
    pub conversation: String,
    /// Timestamp of the message to react to.
    pub ts: String,
    /// Emoji shortcode without colons, such as `tada`.
    pub emoji: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetStatusArgs {
    /// Workspace key as listed by `list_workspaces`.
    pub workspace: String,
    /// Status text (up to 100 characters).
    pub text: Option<String>,
    /// Status emoji shortcode without colons.
    pub emoji: Option<String>,
    /// When the status ends, RFC 3339 with an offset.
    pub expires: Option<String>,
    /// Remove the status instead of setting one.
    pub clear: Option<bool>,
}

fn check_text(text: &str) -> Result<(), ToolFailure> {
    if text.trim().is_empty() || text.chars().count() > TEXT_MAX_CHARS {
        return Err(ToolFailure::bad_argument(format!(
            "text must be 1 to {TEXT_MAX_CHARS} characters and not only whitespace"
        )));
    }
    Ok(())
}

fn parse_instant(name: &str, s: &str) -> Result<jiff::Timestamp, ToolFailure> {
    s.parse::<jiff::Timestamp>().map_err(|_| {
        ToolFailure::bad_argument(format!(
            "{name} must be RFC 3339 with an offset, such as 2030-01-31T09:00:00+01:00"
        ))
    })
}

fn shortcode(s: &str) -> Result<EmojiRef, ToolFailure> {
    let ok = !s.is_empty()
        && s.chars().count() <= EMOJI_MAX_CHARS
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+' | '\''));
    if !ok {
        return Err(ToolFailure::bad_argument(
            "emoji must be a shortcode without colons, such as tada",
        ));
    }
    Ok(EmojiRef {
        shortcode: Arc::from(s),
        unicode: None,
        skin_tone: None,
    })
}

/// A write that passed the policy and argument checks.
struct Target {
    ws: WorkspaceKey,
    info: WorkspaceInfo,
    mode: WriteMode,
    conversation: Option<Conversation>,
}

fn now() -> jiff::Timestamp {
    jiff::Timestamp::now()
}

impl Shared {
    async fn workspace_info(&self, ws: &WorkspaceKey) -> Result<WorkspaceInfo, ToolFailure> {
        self.workspaces()
            .await
            .into_iter()
            .find(|w| &w.key == ws)
            .ok_or_else(ToolFailure::unknown)
    }

    /// Policy for a write aimed at a conversation: visible, and writes on.
    async fn write_target(
        &self,
        span: &mut AuditSpan,
        workspace: &str,
        conversation: &str,
    ) -> Result<Target, ToolFailure> {
        let policy = Policy::from_source(&*self.settings);
        let ws = parse_workspace(workspace)?;
        span.workspace(ws.clone());
        span.target(target_conversation(conversation));
        let c = self
            .visible_conversation(&policy, &ws, conversation)
            .await?;
        let mode = policy.write(&ws)?;
        Ok(Target {
            info: self.workspace_info(&ws).await?,
            ws,
            mode,
            conversation: Some(c),
        })
    }

    /// Asks the user when the workspace wants confirmation, unless they
    /// chose "Always for this session" for this tool and destination, then
    /// checks the policy again: a yes does not outlive a settings change.
    /// The returned slot keeps the confirmation queue place until the write
    /// is done.
    async fn settle(
        &self,
        span: &mut AuditSpan,
        client: &ClientInfo,
        t: &Target,
        action: WriteAction,
    ) -> Result<Option<PendingGuard>, ToolFailure> {
        if t.mode == WriteMode::Auto {
            span.decision(WriteDecision::Auto);
            return Ok(None);
        }
        let scope = SessionScope {
            tool: action.tool(),
            workspace: t.ws.clone(),
            conversation: t.conversation.as_ref().map(|c| c.id.clone()),
        };
        if self.session_allows.allows(&scope) {
            span.decision(WriteDecision::SessionAllowed);
            return Ok(None);
        }
        let slot = self.limits.pending(client.id)?;
        let as_user = self.as_user(&t.info).await;
        let asked = self.confirmer.confirm(WriteRequest {
            client: client.clone(),
            workspace: t.info.clone(),
            as_user,
            action,
        });
        let answer = tokio::time::timeout(self.confirm_timeout, asked)
            .await
            .unwrap_or(Confirmation::TimedOut);
        let for_session = match answer {
            Confirmation::Approved => false,
            Confirmation::ApprovedForSession => true,
            Confirmation::Declined => {
                span.decision(WriteDecision::Declined);
                return Err(ToolFailure::declined());
            }
            Confirmation::TimedOut => {
                span.decision(WriteDecision::TimedOut);
                return Err(ToolFailure::timed_out());
            }
        };
        span.decision(if for_session {
            WriteDecision::ConfirmedForSession
        } else {
            WriteDecision::Confirmed
        });
        let policy = Policy::from_source(&*self.settings);
        policy.write(&t.ws)?;
        if let Some(c) = &t.conversation {
            policy.conversation(&t.ws, c)?;
        }
        if for_session {
            // A full set keeps asking; nothing else changes.
            self.session_allows.allow(scope);
        }
        Ok(Some(slot))
    }

    /// The signed-in user's display name, for the prompt's "As" row. A slow
    /// lookup never holds the prompt back: past [`AS_USER_LOOKUP`] the row
    /// says "You".
    async fn as_user(&self, info: &WorkspaceInfo) -> Option<Arc<str>> {
        let me = info.self_user.as_ref()?;
        let m = tokio::time::timeout(AS_USER_LOOKUP, self.backend.member(&info.key, me))
            .await
            .ok()?
            .ok()??;
        [&m.real_name, &m.display_name]
            .into_iter()
            .find(|n| !n.trim().is_empty())
            .cloned()
    }

    pub(crate) async fn send_message(
        &self,
        span: &mut AuditSpan,
        client: &ClientInfo,
        args: SendMessageArgs,
    ) -> Result<CallToolResult, ToolFailure> {
        let t = self
            .write_target(span, &args.workspace, &args.conversation)
            .await?;
        span.content(|| args.text.clone());
        check_text(&args.text)?;
        if let Some(k) = &args.idempotency_key
            && (k.is_empty() || k.len() > MAX_KEY_LEN)
        {
            return Err(ToolFailure::bad_argument(format!(
                "idempotency_key must be 1 to {MAX_KEY_LEN} characters"
            )));
        }
        let thread = args.thread_ts.as_deref().map(parse_timestamp).transpose()?;
        let broadcast = args.also_send_to_conversation == Some(true);
        if broadcast && thread.is_none() {
            return Err(ToolFailure::bad_argument(
                "also_send_to_conversation needs thread_ts",
            ));
        }
        if broadcast && !t.info.caps.threads {
            return Err(ToolFailure::bad_argument(
                "this workspace has no threads, so also_send_to_conversation does not apply",
            ));
        }
        // Claim the key first: a concurrent retry waits here, then replays.
        let mut claim = None;
        if let Some(k) = &args.idempotency_key {
            let held = self
                .idempotency
                .slot(&client.name, k, Instant::now())
                .acquire()
                .await;
            if let Some(stored) = held.replay(Instant::now()) {
                span.note("idempotent_replay");
                return Ok(ok_result(&stored));
            }
            claim = Some(held);
        }
        let c = t.conversation.as_ref().expect("conversation target");
        let _slot = self
            .settle(
                span,
                client,
                &t,
                WriteAction::Send {
                    conversation_name: conversation_name(c),
                    thread: thread.clone(),
                    broadcast,
                    text: Arc::from(args.text.as_str()),
                },
            )
            .await?;
        let sent: Message = self
            .backend
            .send(
                &t.ws,
                SendRequest {
                    conversation: c.id.clone(),
                    thread: thread.clone(),
                    broadcast,
                    text: Arc::from(args.text.as_str()),
                    idempotency_key: args.idempotency_key.as_deref().map(Arc::from),
                },
            )
            .await?;
        let out = SendOut {
            workspace: t.ws.to_string(),
            conversation: c.id.0.to_string(),
            ts: sent.id.ts.0.to_string(),
            thread_ts: sent
                .thread_root
                .as_ref()
                .or(thread.as_ref())
                .map(|x| x.0.to_string()),
            link: self.web_url(&t.ws, &sent.id).await,
        };
        let value = serde_json::to_value(&out).unwrap_or(Value::Null);
        if let Some(held) = claim.as_mut() {
            held.store(value.clone(), Instant::now());
        }
        Ok(ok_result(&value))
    }

    pub(crate) async fn schedule_message(
        &self,
        span: &mut AuditSpan,
        client: &ClientInfo,
        args: ScheduleMessageArgs,
    ) -> Result<CallToolResult, ToolFailure> {
        let t = self
            .write_target(span, &args.workspace, &args.conversation)
            .await?;
        span.content(|| args.text.clone());
        check_text(&args.text)?;
        let thread = args.thread_ts.as_deref().map(parse_timestamp).transpose()?;
        let at = parse_instant("at", &args.at)?;
        // The scheduler's own rules run before the user is asked.
        domain::validate(
            at.as_second(),
            now().as_second(),
            false,
            t.info.caps.scheduled_send,
        )
        .map_err(|e| {
            ToolFailure::bad_argument(match e {
                domain::ScheduleError::Past => "at is in the past".to_string(),
                domain::ScheduleError::TooFar => {
                    format!("at is more than {} days ahead", domain::MAX_AHEAD_DAYS)
                }
                domain::ScheduleError::FilesNeedLocal => {
                    "this workspace cannot schedule messages".to_string()
                }
            })
        })?;
        let c = t.conversation.as_ref().expect("conversation target");
        let _slot = self
            .settle(
                span,
                client,
                &t,
                WriteAction::Schedule {
                    conversation_name: conversation_name(c),
                    thread: thread.clone(),
                    text: Arc::from(args.text.as_str()),
                    at,
                },
            )
            .await?;
        let s = self
            .backend
            .schedule(
                &t.ws,
                ScheduleRequest {
                    conversation: c.id.clone(),
                    thread,
                    text: Arc::from(args.text.as_str()),
                    at,
                },
            )
            .await?;
        Ok(ok_result(&ScheduledOut {
            workspace: t.ws.to_string(),
            id: s.id.0.to_string(),
            conversation: s.conversation.0.to_string(),
            thread_ts: s.thread.as_ref().map(|x| x.0.to_string()),
            untrusted_text: s.text.to_string(),
            post_at: s.post_at,
        }))
    }

    /// The target of a reaction, from the store or, budget allowing, the provider.
    async fn find_message(
        &self,
        ws: &WorkspaceKey,
        c: &Conversation,
        ts: &MessageTs,
    ) -> Result<Message, ToolFailure> {
        let pick = |page: domain::HistoryPage| page.messages.into_iter().find(|m| &m.id.ts == ts);
        let local = self.backend.thread(ws, &c.id, ts, None, 1, false).await?;
        if let Some(m) = pick(local.value) {
            return Ok(m);
        }
        if self.spend_provider_read(ws) {
            let remote = self.backend.thread(ws, &c.id, ts, None, 1, true).await?;
            self.settle_provider_calls(ws, remote.provider_calls);
            if let Some(m) = pick(remote.value) {
                return Ok(m);
            }
        }
        Err(ToolFailure::unknown())
    }

    pub(crate) async fn add_reaction(
        &self,
        span: &mut AuditSpan,
        client: &ClientInfo,
        args: AddReactionArgs,
    ) -> Result<CallToolResult, ToolFailure> {
        let t = self
            .write_target(span, &args.workspace, &args.conversation)
            .await?;
        let ts = parse_timestamp(&args.ts)?;
        span.target(target_message(&args.conversation, &ts.0));
        span.content(|| args.emoji.clone());
        let emoji = shortcode(&args.emoji)?;
        let c = t.conversation.as_ref().expect("conversation target");
        let target = self.find_message(&t.ws, c, &ts).await?;
        let _slot = self
            .settle(
                span,
                client,
                &t,
                WriteAction::React {
                    conversation_name: conversation_name(c),
                    message_excerpt: Arc::from(
                        truncate_text(&target.plain, EXCERPT_CHARS).0.as_str(),
                    ),
                    emoji: emoji.clone(),
                },
            )
            .await?;
        self.backend
            .react(
                &t.ws,
                &MessageId {
                    conversation: c.id.clone(),
                    ts: ts.clone(),
                },
                &emoji,
            )
            .await?;
        Ok(ok_result(&ReactionOut {
            workspace: t.ws.to_string(),
            conversation: c.id.0.to_string(),
            ts: ts.0.to_string(),
            emoji: args.emoji,
        }))
    }

    pub(crate) async fn set_status(
        &self,
        span: &mut AuditSpan,
        client: &ClientInfo,
        args: SetStatusArgs,
    ) -> Result<CallToolResult, ToolFailure> {
        let policy = Policy::from_source(&*self.settings);
        let ws = parse_workspace(&args.workspace)?;
        span.workspace(ws.clone());
        policy.workspace(&ws)?;
        let mode = policy.write(&ws)?;
        span.content(|| args.text.clone().unwrap_or_default());
        let clear = args.clear == Some(true);
        let status = if clear {
            if args.text.is_some() || args.emoji.is_some() || args.expires.is_some() {
                return Err(ToolFailure::bad_argument(
                    "clear cannot be combined with text, emoji or expires",
                ));
            }
            None
        } else {
            if args.text.is_none() && args.emoji.is_none() {
                return Err(ToolFailure::bad_argument(
                    "give text or emoji, or set clear to true",
                ));
            }
            let text = args.text.clone().unwrap_or_default();
            if text.chars().count() > STATUS_TEXT_MAX_CHARS {
                return Err(ToolFailure::bad_argument(format!(
                    "text must be at most {STATUS_TEXT_MAX_CHARS} characters"
                )));
            }
            let expires = match &args.expires {
                Some(e) => {
                    let at = parse_instant("expires", e)?;
                    if at <= now() {
                        return Err(ToolFailure::bad_argument("expires is in the past"));
                    }
                    Some(at.as_second())
                }
                None => None,
            };
            Some(Status {
                emoji: args.emoji.as_deref().map(shortcode).transpose()?,
                text: Arc::from(text.as_str()),
                expires,
            })
        };
        let t = Target {
            info: self.workspace_info(&ws).await?,
            ws,
            mode,
            conversation: None,
        };
        let _slot = self
            .settle(
                span,
                client,
                &t,
                WriteAction::SetStatus {
                    status: status.clone(),
                },
            )
            .await?;
        self.backend.set_status(&t.ws, status).await?;
        Ok(ok_result(&StatusOut {
            workspace: t.ws.to_string(),
            cleared: clear,
        }))
    }
}

fn conversation_name(c: &Conversation) -> Arc<str> {
    if c.name.is_empty() {
        c.peer
            .as_ref()
            .map(|p| p.0.clone())
            .unwrap_or_else(|| c.id.0.clone())
    } else {
        c.name.clone()
    }
}
