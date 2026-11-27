//! Huddle rooms and frames into neutral `domain` huddle types. The ring path (a join frame
//! that lists the signed-in user as a pending invitee) is unverified against a capture.

use std::sync::Arc;

use domain::{ConversationId, Huddle, HuddleId, ProviderEvent, UserId};
use slack_api::Room;
use slack_realtime::huddle::HuddleEvent;

/// `None` when the room names no conversation (it could not be shown anywhere).
pub(crate) fn map_room(r: &Room) -> Option<Huddle> {
    let channel = r.channels.iter().find(|c| !c.is_empty())?;
    Some(Huddle {
        id: HuddleId(Arc::from(r.id.as_str())),
        conversation: ConversationId(Arc::from(channel.as_str())),
        participants: r
            .participants
            .iter()
            .map(|u| UserId(Arc::from(u.as_str())))
            .collect(),
        started: r.date_start,
        created_by: r.created_by.as_deref().map(|u| UserId(Arc::from(u))),
        is_dm: r.is_dm_call,
    })
}

fn room_events(r: &Room, ring_for: Option<(&str, &str)>) -> Vec<ProviderEvent> {
    let Some(h) = map_room(r) else {
        return Vec::new();
    };
    if !r.is_active() {
        return vec![ProviderEvent::HuddleEnded {
            conversation: h.conversation,
            id: h.id,
            ended: r.date_end.max(r.date_start),
        }];
    }
    let ring = ring_for.and_then(|(me, caller)| {
        r.pending_invitees.iter().any(|u| u == me).then(|| {
            let from = r.created_by.as_deref().unwrap_or(caller);
            ProviderEvent::HuddleRing {
                conversation: h.conversation.clone(),
                huddle: h.id.clone(),
                from: UserId(Arc::from(from)),
            }
        })
    });
    let mut out = vec![ProviderEvent::HuddleUpdated(h)];
    out.extend(ring);
    out
}

/// `me`: the signed-in user; without it a join never rings.
pub(crate) fn map_huddle(e: HuddleEvent, me: Option<&str>) -> Vec<ProviderEvent> {
    match e {
        HuddleEvent::RoomJoin { room, user } => {
            room_events(&room, me.map(|me| (me, user.as_str())))
        }
        HuddleEvent::RoomUpdate { room }
        | HuddleEvent::RoomLeave { room, .. }
        | HuddleEvent::Ended { room } => room_events(&room, None),
        // Advisory: only the displayed in-huddle flag. It never starts, ends or rings a huddle.
        HuddleEvent::UserHuddleChanged { user, in_huddle } => vec![ProviderEvent::InHuddle {
            user: UserId(Arc::from(user.as_str())),
            active: in_huddle,
        }],
        HuddleEvent::InviteCancel { channel } => vec![ProviderEvent::HuddleRingCancelled {
            conversation: ConversationId(Arc::from(channel.as_str())),
        }],
    }
}
