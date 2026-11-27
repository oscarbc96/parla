//! Thread windows: replies live in their own window, keyed by the root.

use std::sync::Arc;

use crate::ids::{ConversationId, MessageId, MessageTs};
use crate::model::{HistoryPage, Message, PersistOp};
use crate::reduce::{Changed, Effect, window};
use crate::state::WorkspaceState;
use crate::window::MessageWindow;

/// One open thread: the root (when known) and its replies in ts order.
#[derive(Clone, Debug)]
pub struct ThreadWindow {
    pub root: Option<Message>,
    pub replies: MessageWindow,
    /// The reply to scroll to and highlight once (set by a jump).
    pub jump: Option<MessageId>,
    /// Open viewers (main panel and pop-outs); the thread is dropped when the
    /// last one closes.
    pub viewers: u32,
}

pub(crate) fn root_id(conversation: &ConversationId, root: &MessageTs) -> MessageId {
    MessageId {
        conversation: conversation.clone(),
        ts: root.clone(),
    }
}

/// Get or create the window for `root`, seeding the root from the channel.
pub(crate) fn entry<'a>(state: &'a mut WorkspaceState, root: &MessageId) -> &'a mut ThreadWindow {
    if !state.threads.contains_key(root) {
        let held = state
            .windows
            .get(&root.conversation)
            .and_then(|w| w.messages().iter().find(|m| m.id == *root).cloned());
        let cap = state.window_capacity;
        state.threads.insert(
            root.clone(),
            ThreadWindow {
                root: held,
                replies: MessageWindow::new(cap),
                jump: None,
                viewers: 0,
            },
        );
    }
    state.threads.get_mut(root).expect("inserted above")
}

/// What a walk over the copies of one message reached.
#[derive(Default)]
pub(crate) struct Touched {
    /// Any copy of the message is held (changed or not).
    pub held: bool,
    /// The channel window's copy changed.
    pub timeline: bool,
    /// Roots of the open threads whose copy (reply or root) changed.
    pub threads: Vec<MessageId>,
    /// The `referenced` copy changed.
    pub referenced: bool,
}

impl Touched {
    pub(crate) fn merge(&mut self, t: Touched) {
        self.held |= t.held;
        self.timeline |= t.timeline;
        self.referenced |= t.referenced;
        for r in t.threads {
            if !self.threads.contains(&r) {
                self.threads.push(r);
            }
        }
    }

    /// Whether any copy in a view (window or thread) changed.
    pub(crate) fn in_view(&self) -> bool {
        self.timeline || !self.threads.is_empty()
    }

    /// Repaint what changed: the channel timeline and the touched threads.
    pub(crate) fn emit(&self, id: &MessageId, fx: &mut Vec<Effect>) {
        if self.timeline {
            fx.push(Effect::Changed(Changed::Timeline(id.conversation.clone())));
        }
        for r in &self.threads {
            fx.push(Effect::Changed(Changed::Thread(r.clone())));
        }
    }
}

/// Runs `f` on every held copy of `id`: the channel window's, each open
/// thread's reply or root, and the `referenced` one. `f` returns whether it
/// changed the copy. The single walker for edits, reactions, flags and file
/// changes, so no copy is missed.
pub(crate) fn for_each_copy(
    state: &mut WorkspaceState,
    id: &MessageId,
    mut f: impl FnMut(&mut Message) -> bool,
) -> Touched {
    let mut t = Touched::default();
    if let Some(m) = state
        .windows
        .get_mut(&id.conversation)
        .and_then(|w| w.get_mut(id))
    {
        t.held = true;
        t.timeline = f(m);
    }
    for (key, th) in state.threads.iter_mut() {
        let mut hit = false;
        if let Some(m) = th.root.as_mut().filter(|m| m.id == *id) {
            t.held = true;
            hit |= f(m);
        }
        if let Some(m) = th.replies.get_mut(id) {
            t.held = true;
            hit |= f(m);
        }
        if hit {
            t.threads.push(key.clone());
        }
    }
    if let Some(m) = state.referenced.get_mut(id) {
        t.held = true;
        t.referenced = f(m);
    }
    t
}

/// Puts a pending or delivered message where it belongs: replies in the
/// thread, plus the channel only when broadcast.
pub(crate) fn place(state: &mut WorkspaceState, m: Message) {
    let c = m.id.conversation.clone();
    match m.thread_root.clone() {
        Some(r) => {
            entry(state, &root_id(&c, &r)).replies.insert(m.clone());
            if m.broadcast {
                window(state, &c).insert(m);
            }
        }
        None => window(state, &c).insert(m),
    }
}

/// Removes the pending copy of a client id from every window it may be in.
pub(crate) fn remove_client(
    state: &mut WorkspaceState,
    conv: &ConversationId,
    thread: Option<&MessageTs>,
    id: &crate::ids::ClientMsgId,
) {
    let w = window(state, conv);
    if let Some(mid) = w.find_client(id) {
        w.remove(&mid);
    }
    if let Some(r) = thread
        && let Some(t) = state.threads.get_mut(&root_id(conv, r))
        && let Some(mid) = t.replies.find_client(id)
    {
        t.replies.remove(&mid);
    }
}

/// Sets the delivery state on every copy of a pending message.
pub(crate) fn set_state(
    state: &mut WorkspaceState,
    conv: &ConversationId,
    thread: Option<&MessageTs>,
    id: &crate::ids::ClientMsgId,
    ds: crate::model::DeliveryState,
) {
    let w = window(state, conv);
    if let Some(mid) = w.find_client(id)
        && let Some(m) = w.get_mut(&mid)
    {
        m.state = ds.clone();
    }
    if let Some(r) = thread
        && let Some(t) = state.threads.get_mut(&root_id(conv, r))
        && let Some(mid) = t.replies.find_client(id)
        && let Some(m) = t.replies.get_mut(&mid)
    {
        m.state = ds;
    }
}

fn bump(root: &mut Message, reply: &Message) {
    root.reply_count += 1;
    if root.latest_reply.as_ref().is_none_or(|l| *l < reply.id.ts) {
        root.latest_reply = Some(reply.id.ts.clone());
    }
    if let Some(a) = &reply.author
        && !root.reply_users.contains(a)
    {
        let mut u = root.reply_users.to_vec();
        u.push(a.clone());
        root.reply_users = Arc::from(u);
    }
}

/// A delivered reply arrived (live post or send result): file it in its
/// thread window and update the root summary wherever the root is held.
/// `create` makes the window when absent (own sends).
pub(crate) fn deliver_reply(
    state: &mut WorkspaceState,
    m: &Message,
    create: bool,
    fx: &mut Vec<Effect>,
) {
    let Some(r) = m.thread_root.clone() else {
        return;
    };
    let c = m.id.conversation.clone();
    let tid = root_id(&c, &r);
    let already = state
        .threads
        .get(&tid)
        .is_some_and(|t| t.replies.has_delivered(&m.id))
        || (m.broadcast
            && state
                .windows
                .get(&c)
                .is_some_and(|w| w.has_delivered(&m.id)));
    if create {
        entry(state, &tid);
    }
    let has_window = state.threads.contains_key(&tid);
    if let Some(t) = state.threads.get_mut(&tid) {
        t.replies.insert(m.clone());
    }
    if already {
        if has_window {
            fx.push(Effect::Changed(Changed::Thread(tid)));
        }
        return;
    }
    let mut summary = None;
    let mut in_channel = false;
    if let Some(root) = state.windows.get_mut(&c).and_then(|w| w.get_mut(&tid)) {
        bump(root, m);
        summary = Some((root.reply_count, root.latest_reply.clone()));
        in_channel = true;
    }
    if let Some(root) = state.threads.get_mut(&tid).and_then(|t| t.root.as_mut()) {
        // Keep the thread's copy in step when the channel copy is absent.
        if summary.is_none() {
            bump(root, m);
            summary = Some((root.reply_count, root.latest_reply.clone()));
        } else if let Some((n, l)) = &summary {
            root.reply_count = *n;
            root.latest_reply = l.clone();
            root.reply_users = state
                .windows
                .get(&c)
                .and_then(|w| w.messages().iter().find(|x| x.id == tid))
                .map(|x| x.reply_users.clone())
                .unwrap_or_else(|| root.reply_users.clone());
        }
    }
    if let Some((reply_count, latest_reply)) = summary {
        fx.push(Effect::Persist(PersistOp::ThreadSummary {
            root: tid.clone(),
            reply_count,
            latest_reply,
        }));
    }
    if in_channel {
        fx.push(Effect::Changed(Changed::Timeline(c)));
    }
    if has_window {
        if create && release_unviewed(state, &tid) {
            return;
        }
        fx.push(Effect::Changed(Changed::Thread(tid)));
    }
}

/// Drops a window nobody views once none of its replies is still pending, so
/// a send made with the panel closed does not leave a window behind. Returns
/// whether it was dropped.
fn release_unviewed(state: &mut WorkspaceState, root: &MessageId) -> bool {
    let idle = state.threads.get(root).is_some_and(|t| {
        t.viewers == 0
            && t.replies
                .messages()
                .iter()
                .all(|m| m.state == crate::model::DeliveryState::Delivered)
    });
    if idle {
        state.threads.remove(root);
    }
    idle
}

pub(crate) fn open(state: &mut WorkspaceState, root: MessageId, fx: &mut Vec<Effect>) {
    open_as(state, root, true, fx);
}

/// Opens the thread; `viewer` is false when it is already shown and this is
/// only a refresh (a jump to one of its replies), so one close still ends it.
pub(crate) fn open_as(
    state: &mut WorkspaceState,
    root: MessageId,
    viewer: bool,
    fx: &mut Vec<Effect>,
) {
    let t = entry(state, &root);
    if viewer || t.viewers == 0 {
        t.viewers = t.viewers.saturating_add(1);
    }
    let after = t.replies.newest_delivered_ts().cloned();
    fx.push(Effect::FetchReplies {
        root: root.clone(),
        after: after.clone(),
    });
    if let Some(ts) = after {
        fx.push(Effect::MarkThreadRead {
            root: root.clone(),
            ts,
        });
    }
    fx.push(Effect::Changed(Changed::Thread(root)));
}

pub(crate) fn close(state: &mut WorkspaceState, root: &MessageId, fx: &mut Vec<Effect>) {
    let drop = if let Some(t) = state.threads.get_mut(root) {
        t.viewers = t.viewers.saturating_sub(1);
        t.viewers == 0
    } else {
        false
    };
    if drop {
        state.threads.remove(root);
        fx.push(Effect::Changed(Changed::Thread(root.clone())));
    }
}

pub(crate) fn loaded(
    state: &mut WorkspaceState,
    root: MessageId,
    page: HistoryPage,
    fx: &mut Vec<Effect>,
) {
    // A page that lands after the thread was closed has no window to fill.
    if !state.threads.contains_key(&root) {
        return;
    }
    let t = entry(state, &root);
    t.replies.mark_loaded();
    let before = t.replies.newest_delivered_ts().cloned();
    let mut replies = Vec::new();
    for m in page.messages {
        if m.id == root {
            t.root = Some(m);
        } else {
            t.replies.insert(m.clone());
            replies.push(m);
        }
    }
    let after = t.replies.newest_delivered_ts().cloned();
    fx.push(Effect::Persist(PersistOp::Messages {
        conversation: root.conversation.clone(),
        messages: replies,
    }));
    if let Some(ts) = after
        && before.as_ref() != Some(&ts)
    {
        fx.push(Effect::MarkThreadRead {
            root: root.clone(),
            ts,
        });
    }
    fx.push(Effect::Changed(Changed::Thread(root)));
}
