//! Slack types into the neutral `domain` model. No Slack type leaves this crate.

use std::collections::HashMap;
use std::sync::Arc;

use domain::{
    Conversation, ConversationCounts, ConversationId, ConversationKind, ConversationPrefs,
    DeliveryState, DndStatus, ImageRef, Member, Message, MessageId, MessageTs, NotifyLevel,
    ProviderError, ProviderEvent, ProviderExt, Reaction, RemoteUrl, Status, UserId,
};
use slack_api::{ApiError, ChannelCount, DndRaw, NotifyPrefsRaw, ProfileRaw, types};
use slack_realtime::event::{Presence, SlackEvent};

use crate::ext::{SlackConversationExt, SlackMessageExt};

pub(crate) fn ts_of(s: &str) -> MessageTs {
    MessageTs(Arc::from(s))
}

/// `None` for empty text and for Slack's "never" value (`0000000000.000000`).
pub(crate) fn ts_opt(s: &str) -> Option<MessageTs> {
    let secs = s.split('.').next().unwrap_or("");
    if secs.is_empty() || secs.bytes().all(|b| b == b'0') {
        return None;
    }
    Some(ts_of(s))
}

pub(crate) fn user_id(s: &str) -> UserId {
    UserId(Arc::from(s))
}

/// The user names inside a group DM's internal name (`mpdm-a--b--c-1`).
fn mpdm_handles(name: &str) -> Arc<[Arc<str>]> {
    let Some(body) = name.strip_prefix("mpdm-") else {
        return Arc::from([]);
    };
    let body = match body.rsplit_once('-') {
        Some((head, n)) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => body,
    };
    body.split("--")
        .filter(|h| !h.is_empty())
        .map(Arc::from)
        .collect()
}

/// How long a group DM without an open flag counts as open after its last message.
const GROUP_DM_RECENT_SECS: i64 = 30 * 86_400;

/// Group DMs carry no `is_open` in what the web client loads, so openness is
/// inferred: open when unread, or when `client.counts` (which lists only
/// conversations with recent state) shows a message in the last 30 days.
/// `None` when the payload has the flag or the conversation is not a group DM.
pub fn group_dm_fallback_open(
    c: &types::Conversation,
    count: Option<&ChannelCount>,
    now_secs: i64,
) -> Option<bool> {
    if !c.is_mpim || c.is_open.is_some() {
        return None;
    }
    let Some(n) = count else { return Some(false) };
    let latest = n
        .latest
        .split('.')
        .next()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    Some(n.has_unreads || now_secs - latest <= GROUP_DM_RECENT_SECS)
}

pub fn map_conversation(
    c: &types::Conversation,
    count: Option<&ChannelCount>,
    names: &impl Fn(&str) -> Option<Arc<str>>,
) -> Conversation {
    let kind = if c.is_im {
        ConversationKind::Direct
    } else if c.is_mpim {
        ConversationKind::Group
    } else if c.is_private || c.is_group {
        ConversationKind::PrivateChannel
    } else {
        ConversationKind::Channel
    };
    let name: Arc<str> = match (kind, c.user.as_deref()) {
        (ConversationKind::Direct, Some(u)) => names(u).unwrap_or_else(|| Arc::from("")),
        _ => Arc::from(c.name.as_deref().unwrap_or(&c.id)),
    };
    Conversation {
        id: ConversationId(Arc::from(c.id.as_str())),
        kind,
        name,
        peer: c.user.as_deref().filter(|_| c.is_im).map(user_id),
        is_member: c.is_member || c.is_im || c.is_mpim,
        is_archived: c.is_archived,
        is_open: c.is_open.unwrap_or(true),
        is_muted: false,
        last_read: count.and_then(|n| ts_opt(&n.last_read)),
        latest: count.and_then(|n| ts_opt(&n.latest)),
        unread: count.is_some_and(|n| n.has_unreads),
        mentions: count.map_or(0, |n| n.mention_count),
        participants: if kind == ConversationKind::Group {
            mpdm_handles(&c.name.clone().unwrap_or_default())
        } else {
            Arc::from([])
        },
        member_ids: if kind == ConversationKind::Group {
            c.members.iter().flatten().map(|u| user_id(u)).collect()
        } else {
            Arc::from([])
        },
        topic: Arc::from(c.topic.as_ref().map_or("", |t| t.value.as_str())),
        member_count: c.num_members,
        purpose: Arc::from(c.purpose.as_ref().map_or("", |t| t.value.as_str())),
        ext: ProviderExt::new(SlackConversationExt { updated: c.updated }),
    }
}

fn reactions(m: &types::Message, me: Option<&str>) -> Arc<[Reaction]> {
    m.reactions_typed()
        .into_iter()
        .map(|r| Reaction {
            emoji: slack_markup::emoji_ref(&r.name),
            count: r.count.max(r.users.len() as u32),
            me: me.is_some_and(|me| r.users.iter().any(|u| u == me)),
            users: r.users.iter().map(|u| user_id(u)).collect(),
        })
        .collect()
}

/// `me`: the signed-in user, to flag own reactions.
pub fn map_message_as(channel: &str, m: types::Message, me: Option<&str>) -> Message {
    let thread_root = m.thread_ts.as_deref().filter(|t| *t != m.ts).map(ts_of);
    let rich = slack_markup::parse_message(
        &m.text,
        m.blocks.as_deref(),
        m.attachments.as_deref(),
        m.room.as_deref(),
    );
    let mentions: Arc<[_]> = slack_markup::mentions(&m.text).into();
    let reactions = reactions(&m, me);
    let files: Arc<[domain::FileRef]> =
        m.files_typed().iter().map(crate::files::map_file).collect();
    Message {
        id: MessageId {
            conversation: ConversationId(Arc::from(channel)),
            ts: ts_of(&m.ts),
        },
        author: m.user.as_deref().or(m.bot_id.as_deref()).map(user_id),
        plain: Arc::from(rich.plain_text()),
        text: Arc::from(m.text.as_str()),
        thread_root,
        reply_count: m.reply_count.unwrap_or(0),
        edited: m.edited.as_ref().map(|e| ts_of(&e.ts)),
        client_msg_id: m
            .client_msg_id
            .as_deref()
            .map(|c| domain::ClientMsgId(Arc::from(c))),
        state: DeliveryState::Delivered,
        mentions,
        reactions,
        files,
        latest_reply: m.latest_reply.as_deref().and_then(ts_opt),
        reply_users: m.reply_users.iter().flatten().map(|u| user_id(u)).collect(),
        broadcast: m.subtype.as_deref() == Some("thread_broadcast"),
        pinned: m.pinned_to.iter().flatten().any(|c| c == channel),
        saved: m.is_starred.unwrap_or(false),
        ext: ProviderExt::new(SlackMessageExt {
            subtype: m.subtype,
            bot_id: m.bot_id,
            blocks: m.blocks,
            files: m.files,
            attachments: m.attachments,
            reactions: m.reactions,
            room: m.room,
        }),
    }
}

pub(crate) fn map_pin(channel: &str, p: slack_api::PinRaw) -> domain::Pin {
    domain::Pin {
        message: MessageId {
            conversation: ConversationId(Arc::from(channel)),
            ts: ts_of(&p.ts),
        },
        by: p.by.as_deref().map(user_id),
        at: p.at,
    }
}

pub(crate) fn map_bookmark(b: slack_api::BookmarkRaw) -> domain::Bookmark {
    domain::Bookmark {
        id: domain::BookmarkId(Arc::from(b.id.as_str())),
        conversation: ConversationId(Arc::from(b.channel.as_str())),
        title: Arc::from(b.title.as_str()),
        link: Arc::from(b.link.as_str()),
        emoji: b
            .emoji
            .as_deref()
            .filter(|e| !e.trim_matches(':').is_empty())
            .map(slack_markup::emoji_ref),
    }
}

pub(crate) fn map_saved(r: slack_api::SavedRaw) -> domain::SavedItem {
    domain::SavedItem {
        message: MessageId {
            conversation: ConversationId(Arc::from(r.channel.as_str())),
            ts: ts_of(&r.ts),
        },
        saved_at: r.saved_at,
        due: r.due,
        done: r.done,
    }
}

/// Unread replies are counted as sent; a mention is a reply that names `me`.
pub(crate) fn map_inbox(t: slack_api::InboxRaw, me: &str) -> domain::InboxThread {
    let mentions = t
        .unread_replies
        .iter()
        .filter(|m| {
            slack_markup::mentions(&m.text)
                .iter()
                .any(|x| matches!(x, domain::Mention::User(u) if &*u.0 == me))
        })
        .count() as u32;
    domain::InboxThread {
        root: MessageId {
            conversation: ConversationId(Arc::from(t.channel.as_str())),
            ts: ts_of(&t.root_ts),
        },
        unread_replies: t.unread_replies.len() as u32,
        mentions,
        latest: ts_of(&t.latest),
    }
}

pub fn map_message(channel: &str, m: types::Message) -> Message {
    map_message_as(channel, m, None)
}

pub fn map_user(u: types::User) -> Member {
    let display = if u.profile.display_name.is_empty() {
        if u.profile.real_name.is_empty() {
            u.name.as_str()
        } else {
            u.profile.real_name.as_str()
        }
    } else {
        u.profile.display_name.as_str()
    };
    Member {
        id: user_id(&u.id),
        display_name: Arc::from(display),
        real_name: Arc::from(u.profile.real_name.as_str()),
        handle: Arc::from(u.name.as_str()),
        // The built-in bot comes as a user without `is_bot`.
        is_bot: u.is_bot || u.id == "USLACKBOT",
        deleted: u.deleted,
        status: status_of(
            &u.profile.status_text,
            &u.profile.status_emoji,
            u.profile.status_expiration,
        ),
        title: Some(u.profile.title.as_str())
            .filter(|t| !t.trim().is_empty())
            .map(Arc::from),
        tz: u.tz.as_deref().filter(|t| !t.is_empty()).map(Arc::from),
        avatar: domain::Avatar::new(
            avatar_ref([
                u.profile.image_72.as_deref(),
                u.profile.image_48.as_deref(),
                u.profile.image_192.as_deref(),
                u.profile.image_512.as_deref(),
            ]),
            avatar_ref([
                u.profile.image_192.as_deref(),
                u.profile.image_512.as_deref(),
                u.profile.image_72.as_deref(),
                u.profile.image_48.as_deref(),
            ]),
        ),
        ext: ProviderExt::none(),
    }
}

/// The first usable picture URL: `https` only. A Slack host is fetched through
/// the session (`Provider`); any other (a gravatar) without credentials (`Web`).
/// Callers list the sizes nearest 2x the row avatar first.
pub(crate) fn avatar_ref<'a>(urls: impl IntoIterator<Item = Option<&'a str>>) -> Option<ImageRef> {
    let u = urls
        .into_iter()
        .flatten()
        .find(|u| u.starts_with("https://"))?;
    let on_slack = url::Url::parse(u)
        .map(|p| slack_api::is_file_host(&p))
        .unwrap_or(false);
    Some(if on_slack {
        ImageRef::Provider(RemoteUrl(Arc::from(u)))
    } else {
        ImageRef::Web(Arc::from(u))
    })
}

/// A status from the text, `:emoji:` and expiry; `None` when both text and emoji are empty.
pub(crate) fn status_of(text: &str, emoji: &str, expiration: i64) -> Option<Status> {
    let (text, emoji) = (text.trim(), emoji.trim());
    if text.is_empty() && emoji.is_empty() {
        return None;
    }
    Some(Status {
        emoji: (!emoji.trim_matches(':').is_empty()).then(|| slack_markup::emoji_ref(emoji)),
        text: Arc::from(text),
        expires: (expiration > 0).then_some(expiration),
    })
}

/// `0` and negative stamps mean "none".
fn stamp(ts: i64) -> Option<i64> {
    (ts > 0).then_some(ts)
}

pub(crate) fn map_dnd(d: &DndRaw) -> DndStatus {
    DndStatus {
        enabled: d.dnd_enabled,
        next_start: stamp(d.next_dnd_start_ts),
        next_end: stamp(d.next_dnd_end_ts),
        snooze_until: if d.snooze_enabled {
            stamp(d.snooze_endtime)
        } else {
            None
        },
    }
}

pub(crate) fn map_profile(p: ProfileRaw) -> domain::Profile {
    let opt = |s: &str| (!s.is_empty()).then(|| Arc::<str>::from(s));
    let display: &str = if !p.display_name.is_empty() {
        &p.display_name
    } else {
        &p.real_name
    };
    domain::Profile {
        user: user_id(&p.id),
        display_name: Arc::from(display),
        real_name: Arc::from(p.real_name.as_str()),
        title: opt(&p.title),
        pronouns: opt(&p.pronouns),
        tz: p.tz.as_deref().and_then(opt),
        status: status_of(&p.status_text, &p.status_emoji, p.status_expiration),
        email: opt(&p.email),
        phone: opt(&p.phone),
        fields: p
            .fields
            .iter()
            .map(|(l, v)| (Arc::from(l.as_str()), Arc::from(v.as_str())))
            .collect(),
        image: avatar_ref([p.image.as_deref()]),
        is_bot: p.is_bot,
        deleted: p.deleted,
    }
}

pub(crate) fn map_notify_prefs(p: &NotifyPrefsRaw) -> HashMap<ConversationId, ConversationPrefs> {
    p.channels
        .iter()
        .filter(|c| !c.id.is_empty())
        .map(|c| {
            let level = match c.desktop.as_deref() {
                Some("everything") => NotifyLevel::All,
                Some("mentions") => NotifyLevel::Mentions,
                Some("nothing") => NotifyLevel::Nothing,
                _ => NotifyLevel::Default,
            };
            (
                ConversationId(Arc::from(c.id.as_str())),
                ConversationPrefs {
                    level,
                    muted: c.muted,
                },
            )
        })
        .collect()
}

/// The stored `desktop` value of a level; `None` removes it.
pub(crate) fn level_value(l: NotifyLevel) -> Option<&'static str> {
    match l {
        NotifyLevel::Default => None,
        NotifyLevel::All => Some("everything"),
        NotifyLevel::Mentions => Some("mentions"),
        NotifyLevel::Nothing => Some("nothing"),
    }
}

pub(crate) fn map_counts(c: &ChannelCount) -> ConversationCounts {
    ConversationCounts {
        conversation: ConversationId(Arc::from(c.id.as_str())),
        latest: ts_opt(&c.latest),
        last_read: ts_opt(&c.last_read),
        unread: c.has_unreads,
        mentions: c.mention_count,
    }
}

fn reaction_event(
    user: &str,
    reaction: &str,
    channel: &str,
    ts: &str,
    added: bool,
) -> ProviderEvent {
    ProviderEvent::ReactionChanged {
        id: MessageId {
            conversation: ConversationId(Arc::from(channel)),
            ts: ts_of(ts),
        },
        emoji: slack_markup::emoji_ref(reaction),
        user: user_id(user),
        added,
    }
}

/// A pin or unpin of one message.
fn pin_event(channel: &str, ts: &str, pinned: bool) -> ProviderEvent {
    ProviderEvent::PinChanged {
        id: MessageId {
            conversation: ConversationId(Arc::from(channel)),
            ts: ts_of(ts),
        },
        pinned,
    }
}

/// All neutral events a frame stands for (a batched presence change is several).
/// `me` is the signed-in user, who a huddle join may ring.
pub fn map_events_as(e: SlackEvent, me: Option<&str>) -> Vec<ProviderEvent> {
    match e {
        SlackEvent::Message { channel, message } => {
            let m = *message;
            vec![ProviderEvent::MessagePosted(map_message_as(&channel, m, me))]
        }
        // The inner message is the whole edited message (text, blocks, files, unfurls,
        // `edited` only when a person edited it), so it replaces every held copy.
        SlackEvent::MessageChanged { channel, message } => {
            let m = *message;
            vec![ProviderEvent::MessageUpdated(map_message_as(
                &channel, m, me,
            ))]
        }
        SlackEvent::MessageDeleted {
            channel,
            deleted_ts,
        } => {
            vec![ProviderEvent::MessageDeleted(MessageId {
                conversation: ConversationId(Arc::from(channel.as_str())),
                ts: ts_of(&deleted_ts),
            })]
        }
        SlackEvent::ChannelMarked { channel, ts } => vec![ProviderEvent::ReadMarker {
            conversation: ConversationId(Arc::from(channel.as_str())),
            ts: ts_of(&ts),
        }],
        SlackEvent::UserTyping {
            channel,
            user,
            thread_ts,
        } => vec![ProviderEvent::Typing {
            conversation: ConversationId(Arc::from(channel.as_str())),
            user: user_id(&user),
            thread: thread_ts.as_deref().map(ts_of),
        }],
        SlackEvent::PresenceChange { users, presence } => users
            .iter()
            .map(|u| ProviderEvent::Presence {
                user: user_id(u),
                active: presence == Presence::Active,
            })
            .collect(),
        SlackEvent::UserChange { user } => {
            if user.id.is_empty() {
                return Vec::new();
            }
            let m = map_user(*user);
            let id = m.id.clone();
            vec![
                ProviderEvent::MemberUpdated(m),
                ProviderEvent::ProfileChanged(id),
            ]
        }
        SlackEvent::DndUpdated { user, dnd } | SlackEvent::DndUpdatedUser { user, dnd } => {
            if user.is_empty() {
                return Vec::new();
            }
            vec![ProviderEvent::DndChanged {
                user: user_id(&user),
                status: map_dnd(&dnd),
            }]
        }
        SlackEvent::MessageReplied { channel, message } => vec![ProviderEvent::ThreadUpdated {
            root: MessageId {
                conversation: ConversationId(Arc::from(channel.as_str())),
                ts: ts_of(&message.ts),
            },
            reply_count: message.reply_count.unwrap_or(0),
            latest_reply: message.latest_reply.as_deref().and_then(ts_opt),
            reply_users: message
                .reply_users
                .iter()
                .flatten()
                .map(|u| user_id(u))
                .collect(),
        }],
        SlackEvent::ThreadMarked {
            channel,
            thread_ts,
            ts,
        } => vec![
            ProviderEvent::ThreadReadMarker {
                root: MessageId {
                    conversation: ConversationId(Arc::from(channel.as_str())),
                    ts: ts_of(&thread_ts),
                },
                ts: ts_of(&ts),
            },
            // The Threads view shows unread counts: a read mark makes it stale.
            ProviderEvent::ThreadInboxChanged,
        ],
        SlackEvent::PinAdded { channel, ts } => vec![pin_event(&channel, &ts, true)],
        SlackEvent::PinRemoved { channel, ts } => vec![pin_event(&channel, &ts, false)],
        SlackEvent::BookmarksChanged { channel } => vec![ProviderEvent::BookmarksChanged(
            ConversationId(Arc::from(channel.as_str())),
        )],
        SlackEvent::SavedChanged => vec![ProviderEvent::SavedChanged],
        SlackEvent::ThreadInboxChanged => vec![ProviderEvent::ThreadInboxChanged],
        SlackEvent::ThreadSubscribed {
            channel,
            thread_ts,
            active,
        } => vec![
            ProviderEvent::ThreadFollowChanged {
                root: MessageId {
                    conversation: ConversationId(Arc::from(channel.as_str())),
                    ts: ts_of(&thread_ts),
                },
                followed: active,
            },
            ProviderEvent::ThreadInboxChanged,
        ],
        SlackEvent::ReactionAdded {
            user,
            reaction,
            channel,
            ts,
        } => vec![reaction_event(&user, &reaction, &channel, &ts, true)],
        SlackEvent::ReactionRemoved {
            user,
            reaction,
            channel,
            ts,
        } => vec![reaction_event(&user, &reaction, &channel, &ts, false)],
        SlackEvent::FileDeleted { file_id } => {
            vec![ProviderEvent::FileDeleted(domain::FileId(Arc::from(
                file_id,
            )))]
        }
        SlackEvent::Huddle { event, .. } => crate::huddles::map_huddle(event, me),
        SlackEvent::EmojiChanged => vec![ProviderEvent::CustomEmojiChanged],
        SlackEvent::TeamRename { name } => vec![ProviderEvent::WorkspaceRenamed {
            name: Arc::from(name.as_str()),
        }],
        // A refresh needs `files_info`, which only the async provider can do.
        SlackEvent::FileShared { .. } | SlackEvent::FileChange { .. } => Vec::new(),
        SlackEvent::ChannelRenamed { channel, name } => vec![meta(&channel, |m| {
            m.name = Some(Arc::from(name.as_str()));
        })],
        SlackEvent::ChannelArchived { channel, archived } => {
            vec![meta(&channel, |m| m.is_archived = Some(archived))]
        }
        SlackEvent::ChannelLeft { channel } => vec![meta(&channel, |m| m.is_member = Some(false))],
        // The signed-in user's own membership changes the list itself; anyone
        // else's changes its member list and count.
        SlackEvent::MemberJoined { channel, user } if me == Some(user.as_str()) => {
            vec![meta(&channel, |m| m.is_member = Some(true))]
        }
        SlackEvent::MemberLeft { channel, user } if me == Some(user.as_str()) => {
            vec![meta(&channel, |m| m.is_member = Some(false))]
        }
        SlackEvent::MemberJoined { channel, user } => vec![member(&channel, &user, true)],
        SlackEvent::MemberLeft { channel, user } => vec![member(&channel, &user, false)],
        SlackEvent::Hello
        // The notification prefs need `prefs_get`, which only the async provider can do.
        | SlackEvent::PrefChange { .. }
        | SlackEvent::Pong { .. }
        // `conn.rs` consumes these two before the pump ever sees them.
        | SlackEvent::ReconnectUrl { .. }
        | SlackEvent::Goodbye
        | SlackEvent::Error { .. }
        | SlackEvent::Unknown { .. } => Vec::new(),
    }
}

/// A metadata change of `channel` with the fields `f` sets.
fn meta(channel: &str, f: impl FnOnce(&mut domain::ConversationMeta)) -> ProviderEvent {
    let mut m = domain::ConversationMeta::new(ConversationId(Arc::from(channel)));
    f(&mut m);
    ProviderEvent::ConversationMetaChanged(m)
}

/// Someone else joining (`joined`) or leaving `channel`.
fn member(channel: &str, user: &str, joined: bool) -> ProviderEvent {
    ProviderEvent::ConversationMemberChanged {
        conversation: ConversationId(Arc::from(channel)),
        user: user_id(user),
        joined,
    }
}

pub(crate) fn map_error(e: &ApiError) -> ProviderError {
    if e.is_auth() {
        return ProviderError::SessionExpired;
    }
    match e {
        ApiError::RateLimited { .. } => ProviderError::RateLimited,
        ApiError::Http { .. } | ApiError::Transport(_) => ProviderError::Network(e.to_string()),
        ApiError::TooLarge { .. } => ProviderError::TooLarge,
        ApiError::ForeignHost => {
            ProviderError::Localized(i18n::LocalizedText::new("slack", "error-foreign-host"))
        }
        ApiError::Slack { error, .. } if is_not_allowed(error) => {
            ProviderError::NotAllowed(error.clone())
        }
        _ => ProviderError::Other(e.to_string()),
    }
}

fn is_not_allowed(error: &str) -> bool {
    matches!(
        error,
        "cant_update_message"
            | "cant_delete_message"
            | "edit_window_closed"
            | "not_in_channel"
            | "is_archived"
            | "restricted_action"
            | "channel_not_found"
            | "missing_scope"
            | "no_permission"
            | "message_not_found"
            | "invalid_scheduled_message_id"
            | "user_not_found"
            | "user_not_visible"
            | "profile_status_set_failed_not_valid_emoji"
            | "profile_status_set_failed_not_valid_text"
    )
}

#[cfg(test)]
mod avatar_tests {
    use super::*;

    fn raw(image: Option<&str>) -> ProfileRaw {
        ProfileRaw {
            id: "U1".into(),
            image: image.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn the_profile_picture_goes_through_the_session_on_a_slack_host() {
        let p = map_profile(raw(Some("https://avatars.slack-edge.com/a-192.png")));
        assert_eq!(
            p.image,
            Some(ImageRef::Provider(RemoteUrl(
                "https://avatars.slack-edge.com/a-192.png".into()
            )))
        );
    }

    #[test]
    fn the_profile_picture_on_another_host_is_fetched_without_credentials() {
        let p = map_profile(raw(Some("https://secure.gravatar.com/avatar/x.jpg")));
        assert_eq!(
            p.image,
            Some(ImageRef::Web(
                "https://secure.gravatar.com/avatar/x.jpg".into()
            ))
        );
    }

    #[test]
    fn a_plain_http_or_missing_profile_picture_is_dropped() {
        assert_eq!(
            map_profile(raw(Some("http://avatars.slack-edge.com/a.png"))).image,
            None
        );
        assert_eq!(map_profile(raw(None)).image, None);
    }
}
