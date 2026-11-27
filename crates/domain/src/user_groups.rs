//! User groups: named sets of people one mention reaches (`@eng`).

use std::collections::HashMap;
use std::sync::Arc;

use crate::model::PersistOp;
use crate::reduce::{Changed, Effect};
use crate::rich::Mention;
use crate::state::WorkspaceState;

/// One user group of the workspace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UserGroup {
    /// Provider id, what a group mention carries.
    pub id: Arc<str>,
    /// What follows the `@` (`eng`).
    pub handle: Arc<str>,
    /// Display name (`Engineering`).
    pub name: Arc<str>,
    pub member_count: u32,
    /// The signed-in user is in the group, so its mentions are theirs.
    pub is_member: bool,
}

/// The workspace's groups by id.
pub type UserGroups = HashMap<Arc<str>, UserGroup>;

/// Most groups kept per workspace, in memory and in the store.
pub const MAX_USER_GROUPS: usize = 2000;

/// Keeps the first [`MAX_USER_GROUPS`] by handle, so the cut is stable.
fn bounded(mut all: Vec<UserGroup>) -> Vec<UserGroup> {
    if all.len() > MAX_USER_GROUPS {
        all.sort_by(|a, b| a.handle.cmp(&b.handle).then_with(|| a.id.cmp(&b.id)));
        all.truncate(MAX_USER_GROUPS);
    }
    all
}

fn replace(state: &mut WorkspaceState, all: &[UserGroup]) {
    state.user_groups = all.iter().map(|g| (g.id.clone(), g.clone())).collect();
}

/// The provider's answer: replaces the set and persists it.
pub(crate) fn loaded(state: &mut WorkspaceState, all: Vec<UserGroup>, fx: &mut Vec<Effect>) {
    let all = bounded(all);
    replace(state, &all);
    state.user_groups_fresh = true;
    recount(state, fx);
    fx.push(Effect::Persist(PersistOp::UserGroups(all)));
    fx.push(Effect::Changed(Changed::Members));
}

/// Mentions of a group the user is in only count once the group is known:
/// recounts the conversations whose messages are held. A count never drops
/// below the provider's, which may cover messages the window does not hold.
fn recount(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    let held: Vec<_> = state.windows.keys().cloned().collect();
    let mut moved = false;
    for c in held {
        let Some(before) = state.conversations.get(&c).map(|x| (x.unread, x.mentions)) else {
            continue;
        };
        crate::unread::recompute(state, &c);
        if let Some(x) = state.conversations.get_mut(&c) {
            x.unread = before.0 || x.unread;
            x.mentions = x.mentions.max(before.1);
            moved |= x.mentions != before.1;
        }
    }
    if moved {
        fx.push(Effect::Changed(Changed::Conversations));
    }
}

/// What the store held: painted until the provider answers, never written back.
pub(crate) fn cached(state: &mut WorkspaceState, all: Vec<UserGroup>, fx: &mut Vec<Effect>) {
    if state.user_groups_fresh {
        return;
    }
    replace(state, &bounded(all));
    recount(state, fx);
    fx.push(Effect::Changed(Changed::Members));
}

/// `m` names a group the user belongs to.
pub(crate) fn mentions_my_group(mentions: &[Mention], groups: Option<&UserGroups>) -> bool {
    let Some(groups) = groups else {
        return false;
    };
    mentions.iter().any(|x| match x {
        Mention::Group(id) => groups.get(id).is_some_and(|g| g.is_member),
        _ => false,
    })
}

/// `text` with each `@<id>` of a group `mentions` names replaced by `@handle`
/// when the group is known (a label-less group token reads as its id).
pub fn resolve_group_text(text: &str, mentions: &[Mention], groups: &UserGroups) -> String {
    let mut out = text.to_string();
    for m in mentions {
        let Mention::Group(id) = m else { continue };
        let Some(g) = groups.get(id).filter(|g| !g.handle.is_empty()) else {
            continue;
        };
        let needle = format!("@{id}");
        let mut next = String::with_capacity(out.len());
        let mut rest = out.as_str();
        while let Some(i) = rest.find(&needle) {
            let end = i + needle.len();
            // A whole token only: `@S1x` is not `@S1`.
            let whole = !rest[end..]
                .chars()
                .next()
                .is_some_and(|c| c.is_alphanumeric() || c == '_');
            next.push_str(&rest[..i]);
            if whole {
                next.push('@');
                next.push_str(&g.handle);
            } else {
                next.push_str(&needle);
            }
            rest = &rest[end..];
        }
        next.push_str(rest);
        out = next;
    }
    out
}
