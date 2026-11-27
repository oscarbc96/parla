//! Reminders (`reminders.*`). All run at `Priority::User`; the endpoints are unverified
//! against a capture.

use serde::Deserialize;

use crate::client::Retry;
use crate::{ApiError, Priority, WebClient, method};

/// One reminder as the server reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReminderRaw {
    pub id: String,
    pub text: String,
    /// Unix seconds; `None` for a recurring reminder or when the server sent none.
    pub time: Option<i64>,
    /// Unix seconds it was completed; `None` while open (the server sends `0` or `null`).
    pub complete_ts: Option<i64>,
    pub recurring: bool,
    /// The user it is for (the creator when the server names none).
    pub user: String,
}

/// Wire shape: every field may be missing or `null`.
#[derive(Deserialize, Default)]
struct Wire {
    id: Option<String>,
    creator: Option<String>,
    user: Option<String>,
    text: Option<String>,
    recurring: Option<bool>,
    time: Option<i64>,
    complete_ts: Option<i64>,
}

impl Wire {
    /// `None` for an entry without an id (it could not be completed or deleted later).
    fn into_raw(self) -> Option<ReminderRaw> {
        let id = self.id.filter(|i| !i.is_empty())?;
        let user = [self.user, self.creator]
            .into_iter()
            .flatten()
            .find(|u| !u.is_empty())
            .unwrap_or_default();
        Some(ReminderRaw {
            id,
            text: self.text.unwrap_or_default(),
            time: self.time.filter(|t| *t > 0),
            complete_ts: self.complete_ts.filter(|t| *t > 0),
            recurring: self.recurring.unwrap_or(false),
            user,
        })
    }
}

#[derive(Deserialize, Default)]
struct Added {
    reminder: Option<Wire>,
}

#[derive(Deserialize, Default)]
struct Listed {
    reminders: Option<Vec<Wire>>,
}

impl WebClient {
    /// `reminders.add`: `text` and `time` go verbatim (the server parses phrases such as
    /// "in 20 minutes"; an unparsable one comes back as `cannot_parse`). `user` is
    /// someone else to remind. Never resent after a timeout or 5xx (it may have been stored);
    /// only a 429 is retried.
    pub async fn reminders_add(
        &self,
        text: &str,
        time: &str,
        user: Option<&str>,
    ) -> Result<ReminderRaw, ApiError> {
        let mut params = vec![("text", text), ("time", time)];
        if let Some(u) = user {
            params.push(("user", u));
        }
        let r: Added = self
            .call_write(method::REMINDERS_ADD, &params, Priority::User)
            .await?;
        r.reminder
            .and_then(Wire::into_raw)
            .ok_or_else(|| ApiError::Slack {
                method: method::REMINDERS_ADD.name,
                error: "no_reminder_in_response".into(),
                needed: None,
            })
    }

    /// `reminders.list`: open and completed reminders, unpaged.
    pub async fn reminders_list(&self) -> Result<Vec<ReminderRaw>, ApiError> {
        let r: Listed = self
            .call(method::REMINDERS_LIST, &[], Priority::User)
            .await?;
        Ok(r.reminders
            .unwrap_or_default()
            .into_iter()
            .filter_map(Wire::into_raw)
            .collect())
    }

    /// `reminders.complete`. `not_found` (gone already) stays an `ApiError::Slack`.
    pub async fn reminders_complete(&self, id: &str) -> Result<(), ApiError> {
        self.call_ok(
            method::REMINDERS_COMPLETE,
            &[("reminder", id)],
            Priority::User,
            Retry::Write,
        )
        .await
    }

    /// `reminders.delete`. `not_found` (gone already) stays an `ApiError::Slack`.
    pub async fn reminders_delete(&self, id: &str) -> Result<(), ApiError> {
        self.call_ok(
            method::REMINDERS_DELETE,
            &[("reminder", id)],
            Priority::User,
            Retry::Write,
        )
        .await
    }
}
