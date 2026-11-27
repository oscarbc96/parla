//! Notification decision: pure and deterministic (time is injected).
//!
//! Callers only ask for a decision on live posts (`Effect::MaybeNotify`);
//! catch-up and history never notify.

use std::sync::Arc;

use jiff::civil::{Time, Weekday};
use settings::{DesktopNotifications, NotificationSettings, QuietHours, WorkspaceKey};

use crate::people::{ConversationPrefs, DndStatus, NotifyExtras, NotifyLevel};
use crate::unread::{direct_mention, mentions_self};
use crate::{
    Conversation, ConversationId, ConversationKind, Message, MessageId, MessageTs, UserId,
};

const BODY_MAX_CHARS: usize = 200;

pub struct NotifyContext<'a> {
    pub settings: &'a NotificationSettings,
    pub self_user: &'a UserId,
    pub now: Time,
    /// The local weekday of `now`, for the quiet-hours days.
    pub weekday: Weekday,
    /// Wall clock in unix seconds, so the DND window check stays pure.
    pub now_unix: i64,
    /// The user's own do-not-disturb status, when known.
    pub dnd: Option<&'a DndStatus>,
    /// The conversation's notification level and mute flag, when loaded.
    pub prefs: Option<&'a ConversationPrefs>,
    /// Parla's own extras for the conversation (thread replies, sound, timed mute).
    pub extras: Option<&'a NotifyExtras>,
    /// The message is a reply in a thread the user follows (their own, or one
    /// they took part in); the shell knows, the decision stays pure.
    pub thread_followed: bool,
    /// The message is a reply in a thread the user started (their message is
    /// the root): it notifies even with thread replies off.
    pub thread_started_by_me: bool,
    pub focused: Option<&'a ConversationId>,
    pub window_active: bool,
    /// The workspace's user groups, so a mention of one the user is in counts.
    pub user_groups: Option<&'a crate::user_groups::UserGroups>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notification {
    pub workspace: WorkspaceKey,
    pub conversation: ConversationId,
    pub message: MessageId,
    pub thread: Option<MessageTs>,
    /// Who wrote the message (user content, never translated).
    pub author: Arc<str>,
    /// The channel name, for a channel message; `None` for a direct or group
    /// conversation. The shell builds the localised title from these.
    pub channel: Option<Arc<str>>,
    /// The message text, cut to a notification-sized prefix; empty when the
    /// message only carries files.
    pub text: Arc<str>,
    /// Number of attached files (the body when `text` is empty).
    pub file_count: usize,
    pub silent: bool,
}

fn parse_hm(s: &str) -> Option<Time> {
    let (h, m) = s.split_once(':')?;
    let (h, m): (i8, i8) = (h.trim().parse().ok()?, m.trim().parse().ok()?);
    Time::new(h, m, 0, 0).ok()
}

fn same_day(day: settings::Weekday, wd: Weekday) -> bool {
    use settings::Weekday as S;
    matches!(
        (day, wd),
        (S::Mon, Weekday::Monday)
            | (S::Tue, Weekday::Tuesday)
            | (S::Wed, Weekday::Wednesday)
            | (S::Thu, Weekday::Thursday)
            | (S::Fri, Weekday::Friday)
            | (S::Sat, Weekday::Saturday)
            | (S::Sun, Weekday::Sunday)
    )
}

/// Window may wrap midnight; malformed times or an empty window never match.
/// `q.days` are the days a window starts on (empty: every day), so the part
/// after midnight belongs to the previous day.
fn in_quiet_hours(q: &QuietHours, now: Time, weekday: Weekday) -> bool {
    let (Some(from), Some(to)) = (parse_hm(&q.from), parse_hm(&q.to)) else {
        return false;
    };
    let on = |wd: Weekday| q.days.is_empty() || q.days.iter().any(|d| same_day(*d, wd));
    if from == to {
        false
    } else if from < to {
        now >= from && now < to && on(weekday)
    } else if now >= from {
        on(weekday)
    } else {
        now < to && on(weekday.wrapping_sub(1))
    }
}

/// Whether a local reminder's desktop alert may show: not when desktop
/// notifications are off, nor inside quiet hours. Pause is held by the caller.
pub fn reminder_alert_allowed(s: &NotificationSettings, now: Time, weekday: Weekday) -> bool {
    s.desktop != DesktopNotifications::None
        && !s
            .quiet_hours
            .as_ref()
            .is_some_and(|q| in_quiet_hours(q, now, weekday))
}

/// Neutral description of an account problem the shell reports with a native
/// notification. The shell builds the localised title/body and routes a click
/// back to sign-in. No provider names or types live here.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionExpiredNotice {
    pub workspace: WorkspaceKey,
    /// The workspace's own name (user content, never translated).
    pub workspace_name: Arc<str>,
}

/// Case-insensitive whole-word match.
fn has_keyword(text: &str, keywords: &[String]) -> bool {
    let text = text.to_lowercase();
    keywords.iter().any(|k| {
        let k = k.trim().to_lowercase();
        if k.is_empty() {
            return false;
        }
        text.match_indices(&k).any(|(i, _)| {
            let before = text[..i].chars().next_back();
            let after = text[i + k.len()..].chars().next();
            !before.is_some_and(char::is_alphanumeric) && !after.is_some_and(char::is_alphanumeric)
        })
    })
}

pub fn decide(
    ctx: &NotifyContext,
    ws: &WorkspaceKey,
    conv: &Conversation,
    m: &Message,
    author: &str,
) -> Option<Notification> {
    let s = ctx.settings;
    // M3: the user's own do-not-disturb suppresses everything first, and a
    // per-conversation level and mute flag replace the conversation's own.
    if ctx.dnd.is_some_and(|d| d.active(ctx.now_unix)) {
        return None;
    }
    let muted = ctx.prefs.map(|p| p.muted).unwrap_or(conv.is_muted)
        || ctx.extras.is_some_and(|e| e.muted_at(ctx.now_unix));
    let desktop = match ctx.prefs.map(|p| p.level) {
        Some(NotifyLevel::Nothing) => return None,
        Some(NotifyLevel::All) => DesktopNotifications::All,
        Some(NotifyLevel::Mentions) => DesktopNotifications::Mentions,
        Some(NotifyLevel::Default) | None => s.desktop,
    };
    if desktop == DesktopNotifications::None
        || m.author.as_ref() == Some(ctx.self_user)
        || (ctx.window_active && ctx.focused == Some(&conv.id))
        || s.quiet_hours
            .as_ref()
            .is_some_and(|q| in_quiet_hours(q, ctx.now, ctx.weekday))
    {
        return None;
    }
    // A label-less group token reads as its id; show (and match) the handle.
    let plain = match ctx.user_groups {
        Some(g) => std::borrow::Cow::Owned(crate::user_groups::resolve_group_text(
            &m.plain,
            &m.mentions,
            g,
        )),
        None => std::borrow::Cow::Borrowed(&*m.plain),
    };
    let direct = matches!(
        conv.kind,
        ConversationKind::Direct | ConversationKind::Group
    );
    let mention = mentions_self(m, ctx.self_user, conv.kind, ctx.user_groups);
    let threads_off = ctx.extras.is_some_and(|e| e.threads_off);
    let wanted = if muted {
        direct_mention(m, ctx.self_user)
    } else if m.thread_root.is_some() && threads_off {
        // The option off: a reply gets through by naming the user, or in a
        // thread the user started.
        mention || ctx.thread_started_by_me
    } else if m.thread_root.is_some() && ctx.thread_followed {
        true
    } else {
        match desktop {
            DesktopNotifications::None => false,
            DesktopNotifications::All => {
                conv.is_member
                    && (m.thread_root.is_none()
                        || mentions_self(m, ctx.self_user, conv.kind, ctx.user_groups))
            }
            DesktopNotifications::Mentions => {
                (conv.kind == ConversationKind::Direct && conv.is_member)
                    || mentions_self(m, ctx.self_user, conv.kind, ctx.user_groups)
                    || has_keyword(&plain, &s.keywords)
            }
        }
    };
    if !wanted {
        return None;
    }
    let channel: Option<Arc<str>> = (!direct).then(|| conv.name.clone());
    // Preview off: sender and channel only, no text and no file count.
    let preview = s.show_preview;
    let text: Arc<str> = if preview {
        Arc::from(plain.chars().take(BODY_MAX_CHARS).collect::<String>())
    } else {
        Arc::from("")
    };
    Some(Notification {
        workspace: ws.clone(),
        conversation: conv.id.clone(),
        message: m.id.clone(),
        thread: m.thread_root.clone(),
        author: Arc::from(author),
        channel,
        text,
        file_count: if preview { m.files.len() } else { 0 },
        silent: s.sound == "none" || ctx.extras.is_some_and(|e| e.sound_off),
    })
}
