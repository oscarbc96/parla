//! Provider events that change an existing message: edits, full replacements,
//! thread summaries and deleted files. Each applies to every held copy
//! (`threads::for_each_copy`) and persists once.

use std::sync::Arc;

use crate::files::FileRef;
use crate::ids::{FileId, MessageId, MessageTs, UserId};
use crate::model::{Message, PersistOp};
use crate::reduce::Effect;
use crate::state::WorkspaceState;
use crate::threads;

pub(crate) fn edited(
    state: &mut WorkspaceState,
    id: MessageId,
    text: Arc<str>,
    plain: Arc<str>,
    edited: MessageTs,
    fx: &mut Vec<Effect>,
) {
    threads::for_each_copy(state, &id, |m| {
        m.text = text.clone();
        m.plain = plain.clone();
        m.edited = Some(edited.clone());
        true
    })
    .emit(&id, fx);
    fx.push(Effect::Persist(PersistOp::EditMessage {
        id,
        text,
        plain,
        edited,
    }));
}

/// A full replacement: Parla's own flags (pinned, saved) stay.
pub(crate) fn updated(state: &mut WorkspaceState, new: Message, fx: &mut Vec<Effect>) {
    let id = new.id.clone();
    threads::for_each_copy(state, &id, |m| {
        let (pinned, saved) = (m.pinned, m.saved);
        *m = new.clone();
        m.pinned = pinned;
        m.saved = saved;
        true
    })
    .emit(&id, fx);
    fx.push(Effect::Persist(PersistOp::ReplaceMessage(new)));
}

pub(crate) fn thread_updated(
    state: &mut WorkspaceState,
    root: MessageId,
    reply_count: u32,
    latest_reply: Option<MessageTs>,
    reply_users: Arc<[UserId]>,
    fx: &mut Vec<Effect>,
) {
    threads::for_each_copy(state, &root, |m| {
        m.reply_count = reply_count;
        m.latest_reply = latest_reply.clone();
        m.reply_users = reply_users.clone();
        true
    })
    .emit(&root, fx);
    fx.push(Effect::Persist(PersistOp::ThreadSummary {
        root,
        reply_count,
        latest_reply,
    }));
}

fn holds(m: &Message, file: &FileId) -> bool {
    m.files.iter().any(|f| f.id == *file && !f.tombstone)
}

/// Marks the file deleted on every held message that carries it.
pub(crate) fn file_deleted(state: &mut WorkspaceState, file: FileId, fx: &mut Vec<Effect>) {
    let mut ids: Vec<MessageId> = Vec::new();
    let mut note = |m: &Message| {
        if holds(m, &file) && !ids.contains(&m.id) {
            ids.push(m.id.clone());
        }
    };
    state
        .windows
        .values()
        .flat_map(|w| w.messages())
        .for_each(&mut note);
    for t in state.threads.values() {
        t.root.iter().for_each(&mut note);
        t.replies.messages().iter().for_each(&mut note);
    }
    state.referenced.values().for_each(&mut note);
    for id in ids {
        threads::for_each_copy(state, &id, |m| {
            m.files = m
                .files
                .iter()
                .map(|f| {
                    let mut f: FileRef = f.clone();
                    if f.id == file {
                        f.tombstone = true;
                    }
                    f
                })
                .collect();
            true
        })
        .emit(&id, fx);
    }
    fx.push(Effect::Persist(PersistOp::FileDeleted(file)));
}
