//! Channel admin and stars (2026-11-24 web-client capture; see
//! docs/providers/slack/web-client/conversations.md "Channel admin").

use std::sync::Arc;

use domain::{
    ChannelName, Conversation, ConversationId, InviteReport, NameProblem, ProviderError, UserId,
};
use slack_api::ApiError;

use crate::map::{map_conversation, map_error};
use crate::provider::{OrFail, SlackProvider};

/// Admin answers the user can act on read as provider text; the rest map as
/// any other call.
fn localized(id: &'static str) -> ProviderError {
    ProviderError::Localized(i18n::LocalizedText::new("slack", id))
}

/// Answers about one person in an invite, not about the call.
fn per_person(e: &ApiError) -> bool {
    matches!(e, ApiError::Slack { error, .. } if matches!(
        error.as_str(),
        "already_in_channel"
            | "cant_invite_self"
            | "user_is_restricted"
            | "user_is_ultra_restricted"
            | "ura_max_channels"
            | "user_not_found"
            | "cant_invite"
    ))
}

enum Kind {
    Topic,
    Purpose,
    Other,
}

/// Longest channel name the workspace accepts.
const CHANNEL_NAME_MAX: u32 = 80;

/// A typed name as Slack stores it: lowercase, spaces and dots become
/// hyphens; only letters, digits, `-` and `_` are allowed (the web client's
/// `conversations.validateName` rules).
pub(crate) fn check_channel_name(typed: &str) -> ChannelName {
    let mut name = String::with_capacity(typed.len());
    for ch in typed.trim().chars() {
        let ch = if ch.is_whitespace() || ch == '.' {
            '-'
        } else {
            ch
        };
        name.extend(ch.to_lowercase());
    }
    let problem = if name.is_empty() {
        Some(NameProblem::Empty)
    } else if name.chars().count() > CHANNEL_NAME_MAX as usize {
        Some(NameProblem::TooLong {
            max: CHANNEL_NAME_MAX,
        })
    } else if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        Some(NameProblem::Characters)
    } else {
        None
    };
    ChannelName { name, problem }
}

fn admin_error(e: &ApiError) -> ProviderError {
    let id = match e {
        ApiError::Slack { error, .. } => match error.as_str() {
            "name_taken" => "error-name-taken",
            "invalid_name"
            | "invalid_name_specials"
            | "invalid_name_punctuation"
            | "invalid_name_maxlength"
            | "invalid_name_required"
            | "no_channel" => "error-name-invalid",
            "cant_leave_general" | "cant_archive_general" => "error-general-channel",
            "already_archived" | "is_archived" => "error-archived",
            "method_not_supported_for_channel_type" | "not_authorized" | "restricted_action" => {
                "error-admin-not-allowed"
            }
            "already_in_channel" => "error-already-in-channel",
            "not_in_channel" => "error-not-in-channel",
            "cant_invite_self" => "error-cant-invite-self",
            "user_is_restricted" | "user_is_ultra_restricted" | "ura_max_channels" => {
                "error-user-restricted"
            }
            _ => return map_error(e),
        },
        _ => return map_error(e),
    };
    localized(id)
}

impl SlackProvider {
    fn admin_fail(&self, e: &ApiError) -> ProviderError {
        if e.is_auth() {
            return self.sh.fail(e);
        }
        admin_error(e)
    }

    fn admin_map(&self, raw: &slack_api::Conversation) -> Conversation {
        let mut c = map_conversation(raw, None, &|_| None);
        c.is_member = true;
        self.sh.cache_conversation(&c);
        c
    }

    pub(crate) async fn do_create_channel(
        &self,
        name: &str,
        private: bool,
    ) -> Result<Conversation, ProviderError> {
        let raw = self
            .sh
            .client
            .create_channel(name, private)
            .await
            .map_err(|e| self.admin_fail(&e))?;
        Ok(self.admin_map(&raw))
    }

    pub(crate) async fn do_join(&self, c: &ConversationId) -> Result<Conversation, ProviderError> {
        let raw = self
            .sh
            .client
            .join_channel(&c.0)
            .await
            .map_err(|e| self.admin_fail(&e))?;
        Ok(self.admin_map(&raw))
    }

    pub(crate) async fn do_browse(
        &self,
        query: &str,
        cursor: Option<Arc<str>>,
        limit: u32,
    ) -> Result<domain::Page<Conversation>, ProviderError> {
        let r = self
            .sh
            .client
            .browse_channels(query, cursor.as_deref(), limit)
            .await
            .or_fail(&self.sh)?;
        Ok(domain::Page {
            items: r
                .items
                .iter()
                .map(|c| map_conversation(c, None, &|_| None))
                .collect(),
            next: r.next_cursor.map(Arc::from),
        })
    }

    /// Invites in one batch; when the batch fails on a per-person answer
    /// (someone already in, restricted, ...) and there was more than one
    /// person, it asks again one by one so the report says who failed.
    pub(crate) async fn do_invite(
        &self,
        c: &str,
        users: &[UserId],
    ) -> Result<InviteReport, ProviderError> {
        let client = &self.sh.client;
        let ids: Vec<&str> = users.iter().map(|u| &*u.0).collect();
        match client.invite_to_channel(c, &ids).await {
            Ok(()) => {
                return Ok(InviteReport {
                    added: users.to_vec(),
                    failed: Vec::new(),
                });
            }
            Err(e) if users.len() > 1 && per_person(&e) => {}
            Err(e) if per_person(&e) => {
                return Ok(InviteReport {
                    added: Vec::new(),
                    failed: vec![(users[0].clone(), admin_error(&e))],
                });
            }
            Err(e) => return Err(self.admin_fail(&e)),
        }
        let mut report = InviteReport::default();
        for u in users {
            match client.invite_to_channel(c, &[&*u.0]).await {
                Ok(()) => report.added.push(u.clone()),
                Err(e) if per_person(&e) => report.failed.push((u.clone(), admin_error(&e))),
                Err(e) => return Err(self.admin_fail(&e)),
            }
        }
        Ok(report)
    }

    pub(crate) async fn do_channel_members(&self, c: &str) -> Result<Vec<UserId>, ProviderError> {
        let ids = self.sh.client.channel_members(c).await.or_fail(&self.sh)?;
        Ok(ids.into_iter().map(|u| UserId(Arc::from(u))).collect())
    }

    /// One admin write that answers nothing.
    pub(crate) async fn do_admin(&self, op: AdminOp<'_>) -> Result<(), ProviderError> {
        let client = &self.sh.client;
        let op_kind = match op {
            AdminOp::Topic(..) => Kind::Topic,
            AdminOp::Purpose(..) => Kind::Purpose,
            _ => Kind::Other,
        };
        let r = match op {
            AdminOp::Leave(c) => client.leave_channel(c).await,
            AdminOp::Archive(c) => client.archive_channel(c).await,
            AdminOp::Rename(c, name) => client.rename_channel(c, name).await,
            AdminOp::Topic(c, t) => client.set_channel_topic(c, t).await,
            AdminOp::Purpose(c, p) => client.set_channel_purpose(c, p).await,
            AdminOp::Star(c, on) => client.star_channel(c, on).await,
        };
        r.map_err(|e| match (&op_kind, e) {
            (Kind::Topic, ApiError::Slack { error, .. }) if error == "too_long" => {
                localized("error-topic-too-long")
            }
            (Kind::Purpose, ApiError::Slack { error, .. }) if error == "too_long" => {
                localized("error-purpose-too-long")
            }
            (_, e) => self.admin_fail(&e),
        })
    }
}

pub(crate) enum AdminOp<'a> {
    Leave(&'a str),
    Archive(&'a str),
    Rename(&'a str, &'a str),
    Topic(&'a str, &'a str),
    Purpose(&'a str, &'a str),
    Star(&'a str, bool),
}
