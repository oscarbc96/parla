//! Profiles (a bounded cache), and the optimistic writes of own status and
//! presence. Writes follow the DND pattern: an in-flight count per key, no
//! stored priors, and the truth is refetched when the last answer is in and
//! any write failed.

use crate::calls::{CallOutput, Load, Loaded, ProviderCall};
use crate::flight::{Flight, settle};
use crate::ids::UserId;
use crate::model::{Member, ProviderError};
use crate::people::{PresenceSetting, Profile, Status};
use crate::reduce::{Changed, Effect};
use crate::state::WorkspaceState;

/// The most profiles held; the oldest loaded are dropped first.
pub const PROFILE_CACHE: usize = 200;

/// In-flight optimistic writes of own status and presence.
#[derive(Clone, Debug, Default)]
pub(crate) struct ProfileBook {
    status: Option<Flight>,
    presence: Option<Flight>,
}

/// A status is shown until its expiry (unix seconds).
pub fn status_visible(s: &Status, now: i64) -> bool {
    s.expires.is_none_or(|e| e > now)
}

/// The profile's wall clock at `now`, from its IANA zone name. `None` when
/// the zone is unknown or unnamed.
pub fn local_time(p: &Profile, now: jiff::Timestamp) -> Option<jiff::ZonedDateTime> {
    let tz = jiff::tz::TimeZone::get(p.tz.as_deref()?).ok()?;
    Some(now.to_zoned_datetime(tz))
}

/// Show a profile: load it unless cached; DND is always re-read.
pub(crate) fn open(state: &mut WorkspaceState, user: UserId, fx: &mut Vec<Effect>) {
    if !state.profiles.contains_key(&user) {
        fx.push(Effect::Load(Load::Profile(user.clone())));
    }
    if state.caps.dnd {
        fx.push(Effect::Load(Load::DndUsers(vec![user.clone()])));
    }
    state.open_profile = Some(user);
}

/// Opening a direct message: load the peer's profile (its time zone gives the
/// header's local time) unless the bounded cache holds it. Not for a self DM:
/// the own profile loads at bootstrap.
pub(crate) fn on_open_direct(
    state: &WorkspaceState,
    c: &crate::ids::ConversationId,
    fx: &mut Vec<Effect>,
) {
    let Some(conv) = state.conversations.get(c) else {
        return;
    };
    let (crate::model::ConversationKind::Direct, Some(peer)) = (&conv.kind, &conv.peer) else {
        return;
    };
    if state.self_user.as_ref() == Some(peer) || state.profiles.contains_key(peer) {
        return;
    }
    fx.push(Effect::Load(Load::Profile(peer.clone())));
}

/// `ProfileChanged`: drop the cached copy; reload it only when on screen.
pub(crate) fn changed(state: &mut WorkspaceState, user: UserId, fx: &mut Vec<Effect>) {
    let was = state.profiles.remove(&user).is_some();
    state.profile_order.retain(|u| *u != user);
    if state.open_profile.as_ref() == Some(&user) {
        fx.push(Effect::Load(Load::Profile(user.clone())));
    }
    if was {
        fx.push(Effect::Changed(Changed::Profile(user)));
    }
}

/// A `MemberUpdated` event is about to replace a member: keep an optimistic
/// own status that is still in flight, and copy the status into a cached
/// profile.
pub(crate) fn member_updated(state: &mut WorkspaceState, m: &mut Member, fx: &mut Vec<Effect>) {
    if state.profile_book.status.is_some()
        && state.self_user.as_ref() == Some(&m.id)
        && let Some(old) = state.members.get(&m.id)
    {
        m.status = old.status.clone();
    }
    if let Some(p) = state.profiles.get_mut(&m.id)
        && p.status != m.status
    {
        p.status = m.status.clone();
        fx.push(Effect::Changed(Changed::Profile(m.id.clone())));
    }
}

/// Set or clear own status, optimistic on the member and a cached profile.
/// Nothing is sent without a self user.
pub(crate) fn set_status(state: &mut WorkspaceState, s: Option<Status>, fx: &mut Vec<Effect>) {
    let Some(me) = state.self_user.clone() else {
        return;
    };
    state
        .profile_book
        .status
        .get_or_insert_with(Flight::default)
        .begin();
    if let Some(m) = state.members.get_mut(&me)
        && m.status != s
    {
        m.status = s.clone();
        fx.push(Effect::Changed(Changed::Members));
    }
    if let Some(p) = state.profiles.get_mut(&me)
        && p.status != s
    {
        p.status = s.clone();
        fx.push(Effect::Changed(Changed::Profile(me)));
    }
    fx.push(Effect::Call(ProviderCall::SetStatus(s)));
}

/// Set own presence, optimistic.
pub(crate) fn set_presence(state: &mut WorkspaceState, p: PresenceSetting, fx: &mut Vec<Effect>) {
    let Some(me) = state.self_user.clone() else {
        return;
    };
    state
        .profile_book
        .presence
        .get_or_insert_with(Flight::default)
        .begin();
    if state.presence_setting != Some(p) {
        state.presence_setting = Some(p);
        fx.push(Effect::Changed(Changed::Profile(me)));
    }
    fx.push(Effect::Call(ProviderCall::SetPresence(p)));
}

/// The answers of `ProviderCall::{SetStatus, SetPresence}`. Status: when the
/// last answer is in and any failed, the profile is refetched. Presence has
/// nothing to read the truth from, so it becomes unknown.
pub(crate) fn call_finished(
    state: &mut WorkspaceState,
    call: &ProviderCall,
    result: &Result<CallOutput, ProviderError>,
    fx: &mut Vec<Effect>,
) {
    match call {
        ProviderCall::SetStatus(_) => {
            let st = settle(&mut state.profile_book.status, result.is_err());
            if st.refetch
                && let Some(me) = state.self_user.clone()
            {
                fx.push(Effect::Load(Load::Profile(me)));
            }
        }
        ProviderCall::SetPresence(_) => {
            let st = settle(&mut state.profile_book.presence, result.is_err());
            if st.refetch
                && state.presence_setting.take().is_some()
                && let Some(me) = state.self_user.clone()
            {
                fx.push(Effect::Changed(Changed::Profile(me)));
            }
        }
        _ => {}
    }
}

/// `Loaded(Profile)`: cache it (bounded), and sync the member's status.
/// While an own status write is in flight the optimistic value stands.
pub(crate) fn handle_loaded(
    state: &mut WorkspaceState,
    r: &Result<Loaded, (Load, ProviderError)>,
    fx: &mut Vec<Effect>,
) {
    let Ok(Loaded::Profile(p)) = r else { return };
    let mut p = p.clone();
    let mine = state.self_user.as_ref() == Some(&p.user);
    let pinned = mine && state.profile_book.status.is_some();
    if pinned {
        if let Some(m) = state.members.get(&p.user) {
            p.status = m.status.clone();
        } else if let Some(old) = state.profiles.get(&p.user) {
            p.status = old.status.clone();
        }
    } else if let Some(m) = state.members.get_mut(&p.user)
        && m.status != p.status
    {
        m.status = p.status.clone();
        fx.push(Effect::Changed(Changed::Members));
    }
    let user = p.user.clone();
    state.profile_order.retain(|u| *u != user);
    state.profile_order.push_back(user.clone());
    state.profiles.insert(user.clone(), p);
    while state.profiles.len() > PROFILE_CACHE {
        let Some(i) = state
            .profile_order
            .iter()
            .position(|u| state.open_profile.as_ref() != Some(u))
        else {
            break;
        };
        if let Some(old) = state.profile_order.remove(i) {
            state.profiles.remove(&old);
        }
    }
    fx.push(Effect::Changed(Changed::Profile(user)));
}
