//! Read endpoints: conversations, history and users.

use futures::{Stream, StreamExt};

use crate::types::conversation::{ConversationInfo, ConversationsPage};
use crate::types::user::{BotInfo, UserInfo, UsersPage};
use crate::types::{Bot, Conversation, HistoryPage, User};
use crate::{ApiError, Priority, WebClient, method};

fn flatten<R, T>(
    s: impl Stream<Item = Result<R, ApiError>>,
    f: fn(R) -> Vec<T>,
) -> impl Stream<Item = Result<Vec<T>, ApiError>> {
    s.map(move |p| p.map(f))
}

impl WebClient {
    /// The conversations the user is in, one `Vec` per page.
    pub fn users_conversations<'a>(
        &'a self,
        types: &str,
        prio: Priority,
    ) -> impl Stream<Item = Result<Vec<Conversation>, ApiError>> + 'a {
        let params = vec![
            ("types".to_string(), types.to_string()),
            ("exclude_archived".to_string(), "true".to_string()),
            ("limit".to_string(), "200".to_string()),
        ];
        flatten(
            self.pages::<ConversationsPage>(method::USERS_CONVERSATIONS, params, prio),
            |p| p.channels,
        )
    }

    /// One page of history; `oldest` is exclusive. With both `latest` and `oldest`
    /// (`history_between`) `inclusive=false` is sent explicitly.
    pub async fn conversations_history(
        &self,
        channel: &str,
        latest: Option<&str>,
        oldest: Option<&str>,
        limit: u32,
        prio: Priority,
    ) -> Result<HistoryPage, ApiError> {
        let limit = limit.to_string();
        let mut params = vec![("channel", channel), ("limit", limit.as_str())];
        if let Some(l) = latest {
            params.push(("latest", l));
        }
        if let Some(o) = oldest {
            params.push(("oldest", o));
            if latest.is_some() {
                params.push(("inclusive", "false"));
            }
        }
        self.call(method::CONVERSATIONS_HISTORY, &params, prio)
            .await
    }

    pub async fn conversations_info(&self, channel: &str) -> Result<Conversation, ApiError> {
        let r: ConversationInfo = self
            .call(
                method::CONVERSATIONS_INFO,
                &[("channel", channel)],
                Priority::Visible,
            )
            .await?;
        Ok(r.channel)
    }

    /// `conversations.open` with `return_im`: opens (or creates) the direct
    /// message with `user` and returns it. Unverified against a capture.
    pub async fn conversations_open(&self, user: &str) -> Result<Conversation, ApiError> {
        let r: ConversationInfo = self
            .call(
                method::CONVERSATIONS_OPEN,
                &[("users", user), ("return_im", "true")],
                Priority::User,
            )
            .await?;
        Ok(r.channel)
    }

    pub async fn users_info(&self, user: &str) -> Result<User, ApiError> {
        let r: UserInfo = self
            .call(method::USERS_INFO, &[("user", user)], Priority::Visible)
            .await?;
        Ok(r.user)
    }

    /// `bots.info`: unverified against a capture (see docs/providers/slack/web-client/messages.md).
    pub async fn bots_info(&self, bot: &str) -> Result<Bot, ApiError> {
        let r: BotInfo = self
            .call(method::BOTS_INFO, &[("bot", bot)], Priority::Visible)
            .await?;
        Ok(r.bot)
    }

    pub fn users_list<'a>(
        &'a self,
        prio: Priority,
    ) -> impl Stream<Item = Result<Vec<User>, ApiError>> + 'a {
        let params = vec![("limit".to_string(), "200".to_string())];
        flatten(
            self.pages::<UsersPage>(method::USERS_LIST, params, prio),
            |p| p.members,
        )
    }
}
