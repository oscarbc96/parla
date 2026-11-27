//! Channel admin: create, join, leave, archive, rename, topic, description and
//! invite. Each is one provider call (`ProviderCall::Admin`); a success updates
//! the conversation list, a failure leaves it as it was (the host shows why).

use std::sync::Arc;

use crate::calls::{CallOutput, Load, ProviderCall};
use crate::ids::{ConversationId, UserId};
use crate::model::{Conversation, ConversationMeta, PersistOp, ProviderError};
use crate::reduce::{Changed, Effect};
use crate::state::WorkspaceState;

/// Most conversations asked for by id at once: a live change to a channel the
/// app does not hold asks for it, and the answer adds it.
pub const MAX_CONVERSATION_FETCHES: usize = 32;

/// One channel admin write.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdminCall {
    /// A new channel; `private` makes it invite-only.
    Create {
        name: Arc<str>,
        private: bool,
    },
    Join(ConversationId),
    Leave(ConversationId),
    Archive(ConversationId),
    Rename {
        conversation: ConversationId,
        name: Arc<str>,
    },
    SetTopic {
        conversation: ConversationId,
        topic: Arc<str>,
    },
    /// The description: what the channel is for.
    SetPurpose {
        conversation: ConversationId,
        purpose: Arc<str>,
    },
    Invite {
        conversation: ConversationId,
        users: Vec<UserId>,
    },
}

impl AdminCall {
    /// The conversation the call is about; `None` for a new channel.
    pub fn conversation(&self) -> Option<&ConversationId> {
        match self {
            Self::Create { .. } => None,
            Self::Join(c) | Self::Leave(c) | Self::Archive(c) => Some(c),
            Self::Rename { conversation, .. }
            | Self::SetTopic { conversation, .. }
            | Self::SetPurpose { conversation, .. }
            | Self::Invite { conversation, .. } => Some(conversation),
        }
    }
}

/// Why a channel name cannot be used as typed (the provider's rules say).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameProblem {
    Empty,
    TooLong {
        max: u32,
    },
    /// Characters the provider does not allow in a name.
    Characters,
}

/// A typed channel name as the provider would store it, and what is wrong
/// with it, if anything (`ChatProvider::check_channel_name`). The provider may
/// still refuse it on create (taken, reserved).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChannelName {
    pub name: String,
    pub problem: Option<NameProblem>,
}

/// What a batch invite did, person by person.
#[derive(Clone, Debug, Default)]
pub struct InviteReport {
    pub added: Vec<UserId>,
    /// People not added, with why.
    pub failed: Vec<(UserId, ProviderError)>,
}

/// `Input::Admin`: one provider call, when the provider has channel admin.
pub(crate) fn request(state: &WorkspaceState, call: AdminCall, fx: &mut Vec<Effect>) {
    if state.caps.channel_admin {
        fx.push(Effect::Call(ProviderCall::Admin(call)));
    }
}

/// The answer of an admin call.
pub(crate) fn call_finished(
    state: &mut WorkspaceState,
    call: &ProviderCall,
    result: &Result<CallOutput, ProviderError>,
    fx: &mut Vec<Effect>,
) {
    let (ProviderCall::Admin(call), Ok(out)) = (call, result) else {
        return;
    };
    match (call, out) {
        // A new or joined channel is in the list, as a member, shown.
        (AdminCall::Create { .. } | AdminCall::Join(_), CallOutput::Conversation(c)) => {
            // A known channel keeps its own record (mute, read and unread
            // state, mentions); the answer refreshes what it carries.
            let c = match state.conversations.get(&c.id) {
                Some(old) => {
                    let mut m = old.clone();
                    m.kind = c.kind;
                    if !c.name.is_empty() {
                        m.name = c.name.clone();
                    }
                    if !c.topic.is_empty() {
                        m.topic = c.topic.clone();
                    }
                    if !c.purpose.is_empty() {
                        m.purpose = c.purpose.clone();
                    }
                    m.member_count = c.member_count.or(m.member_count);
                    m.is_archived = c.is_archived;
                    m
                }
                None => c.clone(),
            };
            let mut c = c;
            c.is_member = true;
            c.is_open = true;
            state.conversations.insert(c.id.clone(), c.clone());
            fx.push(Effect::Persist(PersistOp::Conversations(vec![c])));
            fx.push(Effect::Changed(Changed::Conversations));
        }
        (AdminCall::Join(c), _) => apply_meta(
            state,
            ConversationMeta {
                is_member: Some(true),
                ..ConversationMeta::new(c.clone())
            },
            fx,
        ),
        (AdminCall::Leave(c), _) => apply_meta(
            state,
            ConversationMeta {
                is_member: Some(false),
                ..ConversationMeta::new(c.clone())
            },
            fx,
        ),
        (AdminCall::Archive(c), _) => apply_meta(
            state,
            ConversationMeta {
                is_archived: Some(true),
                ..ConversationMeta::new(c.clone())
            },
            fx,
        ),
        (AdminCall::Rename { conversation, name }, _) => apply_meta(
            state,
            ConversationMeta {
                name: Some(name.clone()),
                ..ConversationMeta::new(conversation.clone())
            },
            fx,
        ),
        (
            AdminCall::SetTopic {
                conversation,
                topic,
            },
            _,
        ) => apply_meta(
            state,
            ConversationMeta {
                topic: Some(topic.clone()),
                ..ConversationMeta::new(conversation.clone())
            },
            fx,
        ),
        (
            AdminCall::SetPurpose {
                conversation,
                purpose,
            },
            _,
        ) => apply_meta(
            state,
            ConversationMeta {
                purpose: Some(purpose.clone()),
                ..ConversationMeta::new(conversation.clone())
            },
            fx,
        ),
        // The count follows the people really added; members arrive through
        // the provider's own member events.
        (AdminCall::Invite { conversation, .. }, CallOutput::Invite(r)) => {
            let n = state
                .conversations
                .get(conversation)
                .and_then(|c| c.member_count)
                .map(|n| n + r.added.len() as u32);
            if let Some(n) = n.filter(|_| !r.added.is_empty()) {
                apply_meta(
                    state,
                    ConversationMeta {
                        member_count: Some(n),
                        ..ConversationMeta::new(conversation.clone())
                    },
                    fx,
                );
            }
        }
        (AdminCall::Create { .. } | AdminCall::Invite { .. }, _) => {}
    }
}

/// Applies a metadata change to a known conversation; persists and notifies
/// only when something really changed. A change of a conversation not held
/// yet (a channel the app had not loaded) asks the provider for it instead of
/// dropping it: [`conversation_loaded`] then adds it.
pub(crate) fn apply_meta(state: &mut WorkspaceState, meta: ConversationMeta, fx: &mut Vec<Effect>) {
    let Some(c) = state.conversations.get_mut(&meta.id) else {
        ask_for(state, meta, fx);
        return;
    };
    let mut changed = false;
    let mut set = |slot: &mut Arc<str>, v: Option<Arc<str>>| {
        if let Some(v) = v
            && *slot != v
        {
            *slot = v;
            changed = true;
        }
    };
    set(&mut c.name, meta.name);
    set(&mut c.topic, meta.topic);
    set(&mut c.purpose, meta.purpose);
    let mut flag = |slot: &mut bool, v: Option<bool>| {
        if let Some(v) = v
            && *slot != v
        {
            *slot = v;
            changed = true;
        }
    };
    flag(&mut c.is_archived, meta.is_archived);
    flag(&mut c.is_member, meta.is_member);
    if let Some(n) = meta.member_count
        && c.member_count != Some(n)
    {
        c.member_count = Some(n);
        changed = true;
    }
    if changed {
        fx.push(Effect::Persist(PersistOp::Conversations(vec![c.clone()])));
        fx.push(Effect::Changed(Changed::Conversations));
    }
}

/// Asks the provider for a conversation not held yet, remembering what the
/// change said (the answer may arrive before or after another change) so its
/// membership and archive state are not lost. Without the capability, or past
/// [`MAX_CONVERSATION_FETCHES`], the change is ignored as before.
fn ask_for(state: &mut WorkspaceState, meta: ConversationMeta, fx: &mut Vec<Effect>) {
    if !state.caps.conversation_lookup {
        return;
    }
    // Leaving or archiving a channel the app does not hold needs nothing.
    if meta.is_member == Some(false) || meta.is_archived == Some(true) {
        return;
    }
    if let Some(pending) = state.conversation_fetching.get_mut(&meta.id) {
        merge_hint(pending, meta);
        return;
    }
    if state.conversation_fetching.len() >= MAX_CONVERSATION_FETCHES {
        return;
    }
    let id = meta.id.clone();
    state.conversation_fetching.insert(id.clone(), meta);
    fx.push(Effect::Load(Load::Conversation(id)));
}

/// Keeps the newer `Some` fields of a second change while the first is pending.
fn merge_hint(into: &mut ConversationMeta, from: ConversationMeta) {
    if from.name.is_some() {
        into.name = from.name;
    }
    if from.topic.is_some() {
        into.topic = from.topic;
    }
    if from.is_archived.is_some() {
        into.is_archived = from.is_archived;
    }
    if from.is_member.is_some() {
        into.is_member = from.is_member;
    }
    if from.member_count.is_some() {
        into.member_count = from.member_count;
    }
    if from.purpose.is_some() {
        into.purpose = from.purpose;
    }
}

/// `Loaded::Conversation`: the answer to [`ask_for`]. A conversation not held
/// joins the list with the provider's fields and what the change said about
/// membership and archive state; a held one merges only the fields the answer
/// carries, keeping its read, mute and unread state.
pub(crate) fn conversation_loaded(
    state: &mut WorkspaceState,
    mut c: Conversation,
    fx: &mut Vec<Effect>,
) {
    let hint = state.conversation_fetching.remove(&c.id);
    if state.conversations.contains_key(&c.id) {
        let meta = ConversationMeta {
            id: c.id.clone(),
            name: (!c.name.is_empty()).then(|| c.name.clone()),
            topic: (!c.topic.is_empty()).then(|| c.topic.clone()),
            is_archived: Some(c.is_archived),
            // The held record's membership comes from its own events: an
            // answer that leaves `is_member` out must not turn it into a leave.
            is_member: None,
            member_count: c.member_count,
            purpose: (!c.purpose.is_empty()).then(|| c.purpose.clone()),
        };
        apply_meta(state, meta, fx);
        return;
    }
    if let Some(h) = hint {
        if let Some(v) = h.is_archived {
            c.is_archived = v;
        }
        if let Some(v) = h.is_member {
            c.is_member = v;
        }
        if c.name.is_empty() {
            c.name = h.name.unwrap_or_default();
        }
        if c.topic.is_empty() {
            c.topic = h.topic.unwrap_or_default();
        }
        if c.purpose.is_empty() {
            c.purpose = h.purpose.unwrap_or_default();
        }
        if c.member_count.is_none() {
            c.member_count = h.member_count;
        }
    }
    state.conversations.insert(c.id.clone(), c.clone());
    fx.push(Effect::Persist(PersistOp::Conversations(vec![c])));
    fx.push(Effect::Changed(Changed::Conversations));
}

/// Someone else's membership changed: the member list (when one is held) and
/// the member count follow. A duplicate change changes nothing. A conversation
/// not held is asked for, like any other live change (its count already
/// includes the change).
pub(crate) fn member_changed(
    state: &mut WorkspaceState,
    conversation: ConversationId,
    user: UserId,
    joined: bool,
    fx: &mut Vec<Effect>,
) {
    let Some(c) = state.conversations.get_mut(&conversation) else {
        let meta = ConversationMeta::new(conversation);
        ask_for(state, meta, fx);
        return;
    };
    let listed = !c.member_ids.is_empty();
    let was_in = c.member_ids.contains(&user);
    let list_changed = match (listed, joined, was_in) {
        (true, true, false) => {
            let mut ids: Vec<UserId> = c.member_ids.iter().cloned().collect();
            ids.push(user);
            c.member_ids = ids.into();
            true
        }
        (true, false, true) => {
            let ids: Vec<UserId> = c
                .member_ids
                .iter()
                .filter(|u| **u != user)
                .cloned()
                .collect();
            c.member_ids = ids.into();
            true
        }
        _ => false,
    };
    // Without a held list there is no way to tell a duplicate, so every event
    // counts; with one, only a real change of it does.
    let mut changed = list_changed;
    if let Some(n) = c.member_count.filter(|_| !listed || list_changed) {
        let n = if joined {
            n.saturating_add(1)
        } else {
            n.saturating_sub(1)
        };
        if c.member_count != Some(n) {
            c.member_count = Some(n);
            changed = true;
        }
    }
    if changed {
        fx.push(Effect::Persist(PersistOp::Conversations(vec![c.clone()])));
        fx.push(Effect::Changed(Changed::Conversations));
    }
}
