//! Pins, bookmarks, saved items and the Threads inbox.
//!
//! Pin and save toggles are optimistic on every held copy of the message
//! and on the loaded list. Like the DND writes they keep no prior value:
//! writes in flight are counted per message, and when the last answer is in
//! and any failed, the truth is refetched (only lists that are loaded: a
//! failure never opens a list). Everything held is bounded.
//!
//! Each list has one loader: at most one request in flight plus one
//! pending refresh. A snapshot that lands while writes touching its list
//! are in flight is dropped and re-read once they settle.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::calls::{CallOutput, Load, Loaded, ProviderCall};
use crate::flight::{Flight, settle};
use crate::ids::{ConversationId, MessageId};
use crate::model::{Message, ProviderError};
use crate::reduce::{Changed, Effect, Input};
use crate::saved::{Bookmark, BookmarkId, Pin, SavedItem};
use crate::search::Page;
use crate::state::WorkspaceState;
use crate::threads::{self, Touched};

/// Messages kept for the pin, saved and inbox views, oldest dropped first.
pub const MAX_REFERENCED: usize = 500;
/// Messages fetched per list answer.
const MAX_FETCH: usize = 50;
/// Items held in the saved list and in the inbox.
const MAX_LIST: usize = 1000;
const MAX_PINS: usize = 200;
const MAX_BOOKMARKS: usize = 100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fetch {
    First,
    More,
}

/// One list's loader: at most one request in flight; a refresh asked for
/// meanwhile (or while writes block it) is one pending first-page read.
#[derive(Clone, Copy, Debug, Default)]
struct ListSync {
    inflight: Option<Fetch>,
    stale: bool,
}

impl ListSync {
    fn idle(&self) -> bool {
        self.inflight.is_none() && !self.stale
    }

    /// Start the pending first-page read unless one is out or `blocked`.
    fn pump(&mut self, blocked: bool) -> bool {
        if self.stale && self.inflight.is_none() && !blocked {
            self.stale = false;
            self.inflight = Some(Fetch::First);
            return true;
        }
        false
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct CollectionsBook {
    pin: HashMap<MessageId, Flight>,
    save: HashMap<MessageId, Flight>,
    bookmark: HashMap<ConversationId, Flight>,
    /// The latest value written per message while a write is in flight.
    want_pin: HashMap<MessageId, bool>,
    want_save: HashMap<MessageId, bool>,
    pins_sync: HashMap<ConversationId, ListSync>,
    bookmarks_sync: HashMap<ConversationId, ListSync>,
    saved: ListSync,
    inbox: ListSync,
    /// Messages asked for and not yet answered.
    fetching: HashSet<MessageId>,
    /// Insertion order of `referenced`, oldest first.
    order: VecDeque<MessageId>,
}

impl CollectionsBook {
    /// Drop what lean mode releases.
    pub(crate) fn release(&mut self) {
        self.order.clear();
        self.fetching.clear();
    }
}

#[derive(Clone, Copy)]
enum Flag {
    Pinned,
    Saved,
}

fn apply(m: &mut Message, flag: Flag, v: bool) -> bool {
    let slot = match flag {
        Flag::Pinned => &mut m.pinned,
        Flag::Saved => &mut m.saved,
    };
    let changed = *slot != v;
    *slot = v;
    changed
}

/// Set `flag` on every held copy of `id`.
fn set_flag(state: &mut WorkspaceState, id: &MessageId, flag: Flag, v: bool) -> Touched {
    threads::for_each_copy(state, id, |m| apply(m, flag, v))
}

/// A copy of `id` is held in a window or an open thread.
fn held_in_view(state: &WorkspaceState, id: &MessageId) -> bool {
    state
        .windows
        .get(&id.conversation)
        .is_some_and(|w| w.messages().iter().any(|m| m.id == *id))
        || state.threads.iter().any(|(k, t)| {
            k == id
                || t.root.as_ref().is_some_and(|m| m.id == *id)
                || t.replies.messages().iter().any(|m| m.id == *id)
        })
}

fn held(state: &WorkspaceState, id: &MessageId) -> bool {
    state.referenced.contains_key(id) || held_in_view(state, id)
}

fn in_pins(state: &WorkspaceState, id: &MessageId) -> bool {
    state
        .pins
        .get(&id.conversation)
        .is_some_and(|l| l.iter().any(|p| p.message == *id))
}

fn in_saved(state: &WorkspaceState, id: &MessageId) -> bool {
    state
        .saved
        .as_ref()
        .is_some_and(|p| p.items.iter().any(|i| i.message == *id))
}

fn in_inbox(state: &WorkspaceState, id: &MessageId) -> bool {
    state
        .thread_inbox
        .as_ref()
        .is_some_and(|p| p.items.iter().any(|i| i.root == *id))
}

/// Some loaded list shows this message.
fn wanted(state: &WorkspaceState, id: &MessageId) -> bool {
    in_pins(state, id) || in_saved(state, id) || in_inbox(state, id)
}

/// One `Load(Message)` per message that is neither held nor already asked
/// for, at most `MAX_FETCH`. Lean mode holds no messages, so none is asked
/// for; `wake` asks again when it ends.
fn fetch_missing<'a>(
    state: &mut WorkspaceState,
    ids: impl Iterator<Item = &'a MessageId>,
    fx: &mut Vec<Effect>,
) {
    if state.lean {
        return;
    }
    let mut n = 0;
    for id in ids {
        if n >= MAX_FETCH {
            break;
        }
        if held(state, id) || state.collections.fetching.contains(id) {
            continue;
        }
        state.collections.fetching.insert(id.clone());
        fx.push(Effect::Load(Load::Message(id.clone())));
        n += 1;
    }
}

/// Leaving lean mode: the loaded lists lost their messages, fetch the
/// missing ones again (at most `MAX_FETCH`).
pub(crate) fn wake(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    let mut ids: Vec<MessageId> = Vec::new();
    let mut cs: Vec<&ConversationId> = state.pins.keys().collect();
    cs.sort();
    for c in cs {
        ids.extend(state.pins[c].iter().map(|p| p.message.clone()));
    }
    if let Some(p) = &state.saved {
        ids.extend(p.items.iter().map(|i| i.message.clone()));
    }
    if let Some(p) = &state.thread_inbox {
        ids.extend(p.items.iter().map(|i| i.root.clone()));
    }
    fetch_missing(state, ids.iter(), fx);
}

fn refetch_message(state: &mut WorkspaceState, id: &MessageId, fx: &mut Vec<Effect>) {
    if !state.lean && state.collections.fetching.insert(id.clone()) {
        fx.push(Effect::Load(Load::Message(id.clone())));
    }
}

fn pin_flight_for(state: &WorkspaceState, c: &ConversationId) -> bool {
    state.collections.pin.keys().any(|k| k.conversation == *c)
}

fn settle_key<K: std::hash::Hash + Eq + Clone>(
    map: &mut HashMap<K, Flight>,
    k: &K,
    err: bool,
) -> crate::flight::Settled {
    let mut slot = map.remove(k);
    let st = settle(&mut slot, err);
    if let Some(f) = slot {
        map.insert(k.clone(), f);
    }
    st
}

// ---- list loaders ---------------------------------------------------------

fn pump_pins(state: &mut WorkspaceState, c: &ConversationId, fx: &mut Vec<Effect>) {
    let blocked = pin_flight_for(state, c);
    let Some(s) = state.collections.pins_sync.get_mut(c) else {
        return;
    };
    if s.pump(blocked) {
        fx.push(Effect::Load(Load::Pins(c.clone())));
    }
    if s.idle() {
        state.collections.pins_sync.remove(c);
    }
}

fn request_pins(state: &mut WorkspaceState, c: &ConversationId, fx: &mut Vec<Effect>) {
    state
        .collections
        .pins_sync
        .entry(c.clone())
        .or_default()
        .stale = true;
    pump_pins(state, c, fx);
}

fn pump_bookmarks(state: &mut WorkspaceState, c: &ConversationId, fx: &mut Vec<Effect>) {
    let blocked = state.collections.bookmark.contains_key(c);
    let Some(s) = state.collections.bookmarks_sync.get_mut(c) else {
        return;
    };
    if s.pump(blocked) {
        fx.push(Effect::Load(Load::Bookmarks(c.clone())));
    }
    if s.idle() {
        state.collections.bookmarks_sync.remove(c);
    }
}

fn request_bookmarks(state: &mut WorkspaceState, c: &ConversationId, fx: &mut Vec<Effect>) {
    state
        .collections
        .bookmarks_sync
        .entry(c.clone())
        .or_default()
        .stale = true;
    pump_bookmarks(state, c, fx);
}

fn pump_saved(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    let blocked = !state.collections.save.is_empty();
    if state.collections.saved.pump(blocked) {
        fx.push(Effect::Load(Load::Saved { cursor: None }));
    }
}

fn request_saved(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    state.collections.saved.stale = true;
    pump_saved(state, fx);
}

fn pump_inbox(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    if state.collections.inbox.pump(false) {
        fx.push(Effect::Load(Load::ThreadInbox { cursor: None }));
    }
}

fn request_inbox(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    state.collections.inbox.stale = true;
    pump_inbox(state, fx);
}

// ---- inputs -------------------------------------------------------------

pub(crate) fn open_pins(state: &mut WorkspaceState, c: ConversationId, fx: &mut Vec<Effect>) {
    if !state.pins.contains_key(&c) && !state.collections.pins_sync.contains_key(&c) {
        request_pins(state, &c, fx);
    }
}

pub(crate) fn open_bookmarks(state: &mut WorkspaceState, c: ConversationId, fx: &mut Vec<Effect>) {
    if !state.bookmarks.contains_key(&c) && !state.collections.bookmarks_sync.contains_key(&c) {
        request_bookmarks(state, &c, fx);
    }
}

/// Opening a conversation also loads its bookmarks (the bar above the
/// timeline), when the provider has them.
pub(crate) fn on_open(state: &mut WorkspaceState, c: &ConversationId, fx: &mut Vec<Effect>) {
    if state.caps.bookmarks {
        open_bookmarks(state, c.clone(), fx);
    }
}

pub(crate) fn open_saved(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    if state.saved.is_none() && state.collections.saved.idle() {
        request_saved(state, fx);
    }
}

pub(crate) fn open_inbox(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    if state.thread_inbox.is_none() && state.collections.inbox.idle() {
        request_inbox(state, fx);
    }
}

pub(crate) fn more_saved(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    let Some(p) = state.saved.as_ref() else {
        return;
    };
    let Some(cursor) = p.next.clone() else { return };
    // Writes in flight or a pending refresh: the pages would be stale.
    if p.items.len() >= MAX_LIST || !state.collections.saved.idle() {
        return;
    }
    state.collections.saved.inflight = Some(Fetch::More);
    fx.push(Effect::Load(Load::Saved {
        cursor: Some(cursor),
    }));
}

pub(crate) fn more_inbox(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    let Some(p) = state.thread_inbox.as_ref() else {
        return;
    };
    let Some(cursor) = p.next.clone() else { return };
    if p.items.len() >= MAX_LIST || !state.collections.inbox.idle() {
        return;
    }
    state.collections.inbox.inflight = Some(Fetch::More);
    fx.push(Effect::Load(Load::ThreadInbox {
        cursor: Some(cursor),
    }));
}

/// Add or remove `id` in the loaded pin list; true when it changed.
fn pins_set(state: &mut WorkspaceState, id: &MessageId, pinned: bool, by: bool) -> bool {
    let by = if by { state.self_user.clone() } else { None };
    let at = state.now_unix;
    let Some(list) = state.pins.get_mut(&id.conversation) else {
        return false;
    };
    let pos = list.iter().position(|p| p.message == *id);
    match (pinned, pos) {
        (true, None) => {
            list.insert(
                0,
                Pin {
                    message: id.clone(),
                    by,
                    at,
                },
            );
            list.truncate(MAX_PINS);
            true
        }
        (false, Some(i)) => {
            list.remove(i);
            true
        }
        _ => false,
    }
}

fn saved_set(state: &mut WorkspaceState, id: &MessageId, saved: bool) -> bool {
    let at = state.now_unix;
    let Some(page) = state.saved.as_mut() else {
        return false;
    };
    let pos = page.items.iter().position(|i| i.message == *id);
    match (saved, pos) {
        (true, None) => {
            page.items.insert(
                0,
                SavedItem {
                    message: id.clone(),
                    saved_at: at,
                    due: None,
                    done: false,
                },
            );
            page.items.truncate(MAX_LIST);
            true
        }
        (false, Some(i)) => {
            page.items.remove(i);
            true
        }
        _ => false,
    }
}

fn pins_changed(state: &WorkspaceState, id: &MessageId, list: bool, t: &Touched) -> bool {
    list || (t.referenced && in_pins(state, id))
}

fn saved_changed(state: &WorkspaceState, id: &MessageId, list: bool, t: &Touched) -> bool {
    list || (t.referenced && in_saved(state, id))
}

pub(crate) fn set_pinned(
    state: &mut WorkspaceState,
    id: MessageId,
    pinned: bool,
    fx: &mut Vec<Effect>,
) {
    state.collections.pin.entry(id.clone()).or_default().begin();
    state.collections.want_pin.insert(id.clone(), pinned);
    let t = set_flag(state, &id, Flag::Pinned, pinned);
    let list = pins_set(state, &id, pinned, true);
    t.emit(&id, fx);
    if pins_changed(state, &id, list, &t) {
        fx.push(Effect::Changed(Changed::Pins(id.conversation.clone())));
    }
    fx.push(Effect::Call(ProviderCall::SetPinned { id, pinned }));
}

pub(crate) fn set_saved(
    state: &mut WorkspaceState,
    id: MessageId,
    saved: bool,
    fx: &mut Vec<Effect>,
) {
    state
        .collections
        .save
        .entry(id.clone())
        .or_default()
        .begin();
    state.collections.want_save.insert(id.clone(), saved);
    let t = set_flag(state, &id, Flag::Saved, saved);
    let list = saved_set(state, &id, saved);
    t.emit(&id, fx);
    if saved_changed(state, &id, list, &t) {
        fx.push(Effect::Changed(Changed::Saved));
    }
    fx.push(Effect::Call(ProviderCall::SetSaved { id, saved }));
}

pub(crate) fn add_bookmark(
    conversation: ConversationId,
    title: std::sync::Arc<str>,
    link: std::sync::Arc<str>,
    fx: &mut Vec<Effect>,
) {
    fx.push(Effect::Call(ProviderCall::AddBookmark {
        conversation,
        title,
        link,
    }));
}

pub(crate) fn remove_bookmark(
    state: &mut WorkspaceState,
    conversation: ConversationId,
    id: BookmarkId,
    fx: &mut Vec<Effect>,
) {
    state
        .collections
        .bookmark
        .entry(conversation.clone())
        .or_default()
        .begin();
    if let Some(list) = state.bookmarks.get_mut(&conversation)
        && let Some(i) = list.iter().position(|b| b.id == id)
    {
        list.remove(i);
        fx.push(Effect::Changed(Changed::Bookmarks(conversation.clone())));
    }
    fx.push(Effect::Call(ProviderCall::RemoveBookmark {
        conversation,
        id,
    }));
}

// ---- provider answers ---------------------------------------------------

pub(crate) fn call_finished(
    state: &mut WorkspaceState,
    call: &ProviderCall,
    result: &Result<CallOutput, ProviderError>,
    fx: &mut Vec<Effect>,
) {
    let err = result.is_err();
    match call {
        ProviderCall::SetPinned { id, .. } => {
            let st = settle_key(&mut state.collections.pin, id, err);
            let c = &id.conversation;
            if !state.collections.pin.contains_key(id) {
                state.collections.want_pin.remove(id);
            }
            if st.refetch {
                // Only a loaded list is read again; a failure opens nothing.
                if state.pins.contains_key(c) {
                    state
                        .collections
                        .pins_sync
                        .entry(c.clone())
                        .or_default()
                        .stale = true;
                }
                refetch_message(state, id, fx);
            }
            pump_pins(state, c, fx);
        }
        ProviderCall::SetSaved { id, .. } => {
            let st = settle_key(&mut state.collections.save, id, err);
            if !state.collections.save.contains_key(id) {
                state.collections.want_save.remove(id);
            }
            if st.refetch {
                if state.saved.is_some() {
                    state.collections.saved.stale = true;
                }
                refetch_message(state, id, fx);
            }
            pump_saved(state, fx);
        }
        ProviderCall::AddBookmark { conversation, .. } => match result {
            Ok(CallOutput::Bookmark(b)) => insert_bookmark(state, b.clone(), fx),
            Ok(_) => {
                // No bookmark came back: read this conversation's list, if held.
                if state.bookmarks.contains_key(conversation) {
                    request_bookmarks(state, conversation, fx);
                }
            }
            Err(_) => {}
        },
        ProviderCall::RemoveBookmark { conversation, .. } => {
            let st = settle_key(&mut state.collections.bookmark, conversation, err);
            if st.refetch && state.bookmarks.contains_key(conversation) {
                state
                    .collections
                    .bookmarks_sync
                    .entry(conversation.clone())
                    .or_default()
                    .stale = true;
            }
            pump_bookmarks(state, conversation, fx);
        }
        _ => {}
    }
}

fn insert_bookmark(state: &mut WorkspaceState, b: Bookmark, fx: &mut Vec<Effect>) {
    let c = b.conversation.clone();
    // A list that was never loaded stays unloaded: it is read on open.
    let Some(list) = state.bookmarks.get_mut(&c) else {
        return;
    };
    match list.iter_mut().find(|x| x.id == b.id) {
        Some(x) if *x == b => return,
        Some(x) => *x = b,
        None => {
            list.push(b);
            list.truncate(MAX_BOOKMARKS);
        }
    }
    fx.push(Effect::Changed(Changed::Bookmarks(c)));
}

/// `Loaded(Pins | Bookmarks | Saved | ThreadInbox | Message)` and the load
/// failures that need bookkeeping.
pub(crate) fn handle_loaded(
    state: &mut WorkspaceState,
    r: &Result<Loaded, (Load, ProviderError)>,
    fx: &mut Vec<Effect>,
) {
    match r {
        Ok(Loaded::Pins(c, list)) => pins_loaded(state, c, list, fx),
        Ok(Loaded::Bookmarks(c, list)) => bookmarks_loaded(state, c, list, fx),
        Ok(Loaded::Saved(page)) => saved_loaded(state, page.clone(), fx),
        Ok(Loaded::ThreadInbox(page)) => inbox_loaded(state, page.clone(), fx),
        Ok(Loaded::Message(m)) => message_loaded(state, m, fx),
        Err((Load::Message(id), _)) => {
            state.collections.fetching.remove(id);
        }
        Err((Load::Pins(c), _)) => {
            if let Some(s) = state.collections.pins_sync.get_mut(c) {
                s.inflight = None;
                s.stale = false;
            }
            pump_pins(state, c, fx);
        }
        Err((Load::Bookmarks(c), _)) => {
            if let Some(s) = state.collections.bookmarks_sync.get_mut(c) {
                s.inflight = None;
                s.stale = false;
            }
            pump_bookmarks(state, c, fx);
        }
        Err((Load::Saved { .. }, _)) => {
            state.collections.saved.inflight = None;
            state.collections.saved.stale = false;
        }
        Err((Load::ThreadInbox { .. }, _)) => {
            state.collections.inbox.inflight = None;
            state.collections.inbox.stale = false;
        }
        _ => {}
    }
}

fn pins_loaded(state: &mut WorkspaceState, c: &ConversationId, list: &[Pin], fx: &mut Vec<Effect>) {
    let s = state.collections.pins_sync.entry(c.clone()).or_default();
    s.inflight = None;
    // A write in flight keeps its optimistic value: the snapshot may
    // predate it, so it is dropped and read again once the writes settle.
    if pin_flight_for(state, c) {
        state
            .collections
            .pins_sync
            .entry(c.clone())
            .or_default()
            .stale = true;
        return;
    }
    let mut next = list.to_vec();
    next.truncate(MAX_PINS);
    fetch_missing(state, next.iter().map(|p| &p.message), fx);
    if state.pins.get(c) != Some(&next) {
        state.pins.insert(c.clone(), next);
        fx.push(Effect::Changed(Changed::Pins(c.clone())));
    }
    pump_pins(state, c, fx);
}

fn bookmarks_loaded(
    state: &mut WorkspaceState,
    c: &ConversationId,
    list: &[Bookmark],
    fx: &mut Vec<Effect>,
) {
    state
        .collections
        .bookmarks_sync
        .entry(c.clone())
        .or_default()
        .inflight = None;
    if state.collections.bookmark.contains_key(c) {
        state
            .collections
            .bookmarks_sync
            .entry(c.clone())
            .or_default()
            .stale = true;
        return;
    }
    let mut next = list.to_vec();
    next.truncate(MAX_BOOKMARKS);
    if state.bookmarks.get(c) != Some(&next) {
        state.bookmarks.insert(c.clone(), next);
        fx.push(Effect::Changed(Changed::Bookmarks(c.clone())));
    }
    pump_bookmarks(state, c, fx);
}

/// Replace (first page) or append (next page) `page` into `slot`, bounded
/// by `MAX_LIST`. True when `slot` changed.
fn merge_page<T: Clone + PartialEq>(
    slot: &mut Option<Page<T>>,
    page: Page<T>,
    more: bool,
    id: impl Fn(&T) -> &MessageId,
) -> bool {
    let mut page = match (more, slot.as_ref()) {
        (true, Some(cur)) => {
            let mut items = cur.items.clone();
            for it in page.items {
                if !items.iter().any(|x| id(x) == id(&it)) {
                    items.push(it);
                }
            }
            Page {
                items,
                next: page.next,
            }
        }
        _ => page,
    };
    if page.items.len() >= MAX_LIST {
        page.items.truncate(MAX_LIST);
        page.next = None;
    }
    if slot.as_ref() == Some(&page) {
        return false;
    }
    *slot = Some(page);
    true
}

fn saved_loaded(state: &mut WorkspaceState, page: Page<SavedItem>, fx: &mut Vec<Effect>) {
    let kind = state.collections.saved.inflight.take();
    if !state.collections.save.is_empty() {
        // Saves in flight: this page may predate them. Read again after.
        state.collections.saved.stale = true;
        return;
    }
    fetch_missing(state, page.items.iter().map(|i| &i.message), fx);
    if merge_page(&mut state.saved, page, kind == Some(Fetch::More), |i| {
        &i.message
    }) {
        fx.push(Effect::Changed(Changed::Saved));
    }
    pump_saved(state, fx);
}

fn inbox_loaded(
    state: &mut WorkspaceState,
    page: Page<crate::saved::InboxThread>,
    fx: &mut Vec<Effect>,
) {
    let kind = state.collections.inbox.inflight.take();
    fetch_missing(state, page.items.iter().map(|i| &i.root), fx);
    if merge_page(
        &mut state.thread_inbox,
        page,
        kind == Some(Fetch::More),
        |i| &i.root,
    ) {
        fx.push(Effect::Changed(Changed::ThreadInbox));
    }
    pump_inbox(state, fx);
}

/// The fields a view shows; a refetch that changes none repaints nothing.
fn same_view(a: &Message, b: &Message) -> bool {
    a.text == b.text
        && a.edited == b.edited
        && a.reply_count == b.reply_count
        && a.pinned == b.pinned
        && a.saved == b.saved
        && a.reactions.len() == b.reactions.len()
}

fn message_loaded(state: &mut WorkspaceState, m: &Message, fx: &mut Vec<Effect>) {
    let id = m.id.clone();
    if !state.collections.fetching.remove(&id) || state.lean {
        return;
    }
    // A flag with a write in flight keeps its optimistic value.
    let mut copy = m.clone();
    if let Some(&v) = state.collections.want_pin.get(&id) {
        copy.pinned = v;
    }
    if let Some(&v) = state.collections.want_save.get(&id) {
        copy.saved = v;
    }
    let mut touched = Touched::default();
    touched.merge(set_flag(state, &id, Flag::Pinned, copy.pinned));
    touched.merge(set_flag(state, &id, Flag::Saved, copy.saved));
    touched.emit(&id, fx);
    let mut changed = touched.in_view();
    if held_in_view(state, &id) {
        forget(state, &id);
    } else if wanted(state, &id) {
        let old = state.referenced.get(&id);
        changed |= old.is_none_or(|o| !same_view(o, &copy));
        if state.referenced.insert(id.clone(), copy).is_none() {
            state.collections.order.push_back(id.clone());
            while state.referenced.len() > MAX_REFERENCED {
                match state.collections.order.pop_front() {
                    Some(old) => {
                        state.referenced.remove(&old);
                    }
                    None => break,
                }
            }
        }
    }
    if !changed {
        return;
    }
    // The views that list this message repaint.
    if in_pins(state, &id) {
        fx.push(Effect::Changed(Changed::Pins(id.conversation.clone())));
    }
    if in_saved(state, &id) {
        fx.push(Effect::Changed(Changed::Saved));
    }
    if in_inbox(state, &id) {
        fx.push(Effect::Changed(Changed::ThreadInbox));
    }
}

/// Drop one message from `referenced`.
pub(crate) fn forget(state: &mut WorkspaceState, id: &MessageId) {
    if state.referenced.remove(id).is_some() {
        state.collections.order.retain(|x| x != id);
    }
}

/// Messages that just entered a window or a thread are held there now:
/// drop their `referenced` copies (called before the input is applied).
pub(crate) fn note_input(state: &mut WorkspaceState, input: &Input) {
    if state.referenced.is_empty() {
        return;
    }
    let msgs: &[Message] = match input {
        Input::HistoryLoaded { page, .. }
        | Input::NewerLoaded { page, .. }
        | Input::RepliesLoaded { page, .. } => &page.messages,
        Input::AroundLoaded { page, .. } => &page.messages,
        _ => return,
    };
    for m in msgs {
        forget(state, &m.id);
    }
}

// ---- live events --------------------------------------------------------

pub(crate) fn pin_event(
    state: &mut WorkspaceState,
    id: MessageId,
    pinned: bool,
    fx: &mut Vec<Effect>,
) {
    // Our own write is in flight: the optimistic state stands.
    if state.collections.pin.contains_key(&id) {
        return;
    }
    let t = set_flag(state, &id, Flag::Pinned, pinned);
    let list = pins_set(state, &id, pinned, false);
    t.emit(&id, fx);
    if pins_changed(state, &id, list, &t) {
        fx.push(Effect::Changed(Changed::Pins(id.conversation.clone())));
    }
}

pub(crate) fn bookmarks_event(state: &mut WorkspaceState, c: ConversationId, fx: &mut Vec<Effect>) {
    if state.bookmarks.contains_key(&c) {
        request_bookmarks(state, &c, fx);
    }
}

pub(crate) fn saved_event(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    if state.saved.is_some() {
        request_saved(state, fx);
    }
}

/// `ThreadInboxChanged` and `ThreadReadMarker`.
pub(crate) fn inbox_event(state: &mut WorkspaceState, fx: &mut Vec<Effect>) {
    if state.thread_inbox.is_some() {
        request_inbox(state, fx);
    }
}
