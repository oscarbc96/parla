//! Back and forward through the conversations the user opened, across workspaces.

use std::collections::VecDeque;

use domain::{ConversationId, MessageId};
use settings::WorkspaceKey;

/// Most entries kept.
pub const CAPACITY: usize = 100;
/// Most recently opened conversations kept for the quick switcher.
pub const RECENT: usize = 20;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavEntry {
    pub workspace: WorkspaceKey,
    pub conversation: ConversationId,
    pub message: Option<MessageId>,
}

#[derive(Default)]
pub struct NavHistory {
    entries: Vec<NavEntry>,
    cursor: usize,
    recent: VecDeque<(WorkspaceKey, ConversationId)>,
}

impl NavHistory {
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that `e` is now shown. Showing the current entry again only
    /// refreshes its jump target; anything else drops the forward entries.
    pub fn visit(&mut self, e: NavEntry) {
        self.recent
            .retain(|(w, c)| !(*w == e.workspace && *c == e.conversation));
        self.recent
            .push_front((e.workspace.clone(), e.conversation.clone()));
        self.recent.truncate(RECENT);
        if let Some(cur) = self.entries.get_mut(self.cursor)
            && cur.workspace == e.workspace
            && cur.conversation == e.conversation
        {
            if e.message.is_some() {
                cur.message = e.message;
            }
            return;
        }
        if !self.entries.is_empty() {
            self.entries.truncate(self.cursor + 1);
        }
        self.entries.push(e);
        if self.entries.len() > CAPACITY {
            self.entries.remove(0);
        }
        self.cursor = self.entries.len() - 1;
    }

    pub fn current(&self) -> Option<&NavEntry> {
        self.entries.get(self.cursor)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether an earlier entry exists (the titlebar's back button state).
    pub fn can_back(&self) -> bool {
        self.cursor > 0
    }

    /// Whether back would land on an entry `valid` accepts: the same rule
    /// [`back`](Self::back) applies, so an enabled button always does something.
    pub fn can_back_to(&self, valid: impl Fn(&NavEntry) -> bool) -> bool {
        self.entries[..self.cursor.min(self.entries.len())]
            .iter()
            .any(valid)
    }

    /// Whether forward would land on an entry `valid` accepts.
    pub fn can_forward_to(&self, valid: impl Fn(&NavEntry) -> bool) -> bool {
        self.entries
            .get(self.cursor + 1..)
            .is_some_and(|e| e.iter().any(valid))
    }

    /// Whether a later entry exists.
    pub fn can_forward(&self) -> bool {
        self.cursor + 1 < self.entries.len()
    }

    /// Steps back to the nearest earlier entry `valid` accepts; the position
    /// stays when there is none.
    pub fn back(&mut self, valid: impl Fn(&NavEntry) -> bool) -> Option<NavEntry> {
        let at = (0..self.cursor).rev().find(|i| valid(&self.entries[*i]))?;
        self.cursor = at;
        Some(self.entries[at].clone())
    }

    /// Steps forward to the nearest later entry `valid` accepts.
    pub fn forward(&mut self, valid: impl Fn(&NavEntry) -> bool) -> Option<NavEntry> {
        let at = (self.cursor + 1..self.entries.len()).find(|i| valid(&self.entries[*i]))?;
        self.cursor = at;
        Some(self.entries[at].clone())
    }

    /// The conversations opened lately, newest first (at most [`RECENT`]).
    pub fn recent(&self) -> Vec<(WorkspaceKey, ConversationId)> {
        self.recent.iter().cloned().collect()
    }
}
