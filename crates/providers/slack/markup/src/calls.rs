//! Huddle and third-party call cards. Never panics.

use std::sync::Arc;

use domain::{CallCard, CallKind, UserId};
use serde_json::Value;

fn str_of<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

fn secs(v: &Value, key: &str) -> Option<i64> {
    v.get(key).and_then(Value::as_i64).filter(|t| *t > 0)
}

fn users(v: Option<&Value>) -> Arc<[UserId]> {
    let mut out: Vec<UserId> = Vec::new();
    for p in v.and_then(Value::as_array).into_iter().flatten() {
        let id = match p {
            Value::String(s) => Some(s.as_str()),
            o => str_of(o, "slack_id").or_else(|| str_of(o, "user")),
        };
        if let Some(id) = id.filter(|s| !s.is_empty()) {
            let id = UserId(id.into());
            if !out.contains(&id) {
                out.push(id);
            }
        }
    }
    out.into()
}

/// The card of a huddle `room` object (the `room` of a `huddle_thread`
/// message). `None` when `room` is not a huddle. The join link is dropped
/// once the huddle has ended.
pub fn huddle_card(room: &Value) -> Option<CallCard> {
    if !room.is_object() {
        return None;
    }
    if str_of(room, "call_family").is_some_and(|f| f != "huddle") {
        return None;
    }
    let ended = room
        .get("has_ended")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    Some(CallCard {
        kind: CallKind::Huddle,
        title: str_of(room, "name").map(Into::into),
        join_url: str_of(room, "huddle_link")
            .filter(|_| !ended)
            .map(Into::into),
        started: secs(room, "date_start"),
        ended: secs(room, "date_end"),
        participants: users(
            room.get("participant_history")
                .filter(|h| h.as_array().is_some_and(|a| !a.is_empty()))
                .or_else(|| room.get("participants")),
        ),
        created_by: str_of(room, "created_by").map(|u| UserId(u.into())),
    })
}

/// The card of a third-party `call` block (`call.v1`). The provider name
/// comes from the call's app fields; a missing join URL still yields a card.
pub(crate) fn call_block_card(block: &Value) -> CallCard {
    let call = block
        .get("call")
        .and_then(|c| c.get("v1").or(Some(c)))
        .unwrap_or(&Value::Null);
    let provider = str_of(call, "provider_name")
        .or_else(|| str_of(call, "app_name"))
        .or_else(|| str_of(call, "external_display_id"))
        .unwrap_or("Call");
    let ended = call
        .get("has_ended")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let join = str_of(call, "join_url").or_else(|| str_of(call, "desktop_app_join_url"));
    CallCard {
        kind: CallKind::External {
            provider: provider.into(),
        },
        title: str_of(call, "name").map(Into::into),
        join_url: join.filter(|_| !ended).map(Into::into),
        started: secs(call, "date_start"),
        ended: secs(call, "date_end"),
        participants: users(
            call.get("all_participants")
                .or_else(|| call.get("active_participants")),
        ),
        created_by: None,
    }
}
