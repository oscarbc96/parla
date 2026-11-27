//! Lean mode: a workspace nobody is looking at keeps its counts and its
//! connection but releases every message window.

use crate::reduce::{Changed, Effect};
use crate::state::WorkspaceState;

pub(crate) fn set_lean(state: &mut WorkspaceState, lean: bool, fx: &mut Vec<Effect>) {
    if state.lean == lean {
        return;
    }
    state.lean = lean;
    if lean {
        // Conversations, members and counts stay; held messages go.
        state.windows.clear();
        state.threads.clear();
        state.referenced.clear();
        state.collections.release();
        crate::typing::clear(state, fx);
    } else {
        if let Some(f) = &state.focus {
            fx.push(Effect::Reload(f.conversation.clone()));
        }
        crate::collections::wake(state, fx);
    }
    fx.push(Effect::Changed(Changed::Lean));
}
