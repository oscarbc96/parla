use std::collections::VecDeque;

use crate::ids::{ClientMsgId, MessageId, MessageTs};
use crate::model::{DeliveryState, Message};

/// Rows the provider has not confirmed (pending or failed) carry a local ts,
/// which a busy server may stamp later than: they sort after every confirmed
/// row, in send order, so a confirmed send never lands above a queued one.
fn local(m: &Message) -> bool {
    m.state != DeliveryState::Delivered
}

fn order_key(m: &Message) -> (bool, &MessageTs) {
    (local(m), &m.id.ts)
}

/// Bounded view of one conversation's newest messages: confirmed rows in ts
/// order, then the unconfirmed ones in ts order.
#[derive(Clone, Debug)]
pub struct MessageWindow {
    msgs: VecDeque<Message>,
    capacity: usize,
    has_more_before: bool,
    /// Newer messages exist beyond the held ones (a detached window).
    has_more_after: bool,
    /// A history page has arrived (even an empty one), so an empty window
    /// means an empty conversation, not one still loading.
    loaded: bool,
}

impl MessageWindow {
    pub fn new(capacity: usize) -> Self {
        Self {
            msgs: VecDeque::new(),
            capacity: capacity.max(1),
            has_more_before: false,
            has_more_after: false,
            loaded: false,
        }
    }

    pub fn messages(&self) -> &[Message] {
        // Kept contiguous by `insert` (make_contiguous on every mutation).
        self.msgs.as_slices().0
    }

    pub fn has_more_before(&self) -> bool {
        self.has_more_before
    }

    pub fn has_more_after(&self) -> bool {
        self.has_more_after
    }

    /// Showing the newest messages of the conversation (not detached).
    pub fn is_live(&self) -> bool {
        !self.has_more_after
    }

    #[cfg(not(feature = "test-support"))]
    pub(crate) fn set_has_more_after(&mut self, v: bool) {
        self.has_more_after = v;
    }

    /// Test helper: detach the window from the live end.
    #[cfg(feature = "test-support")]
    pub fn set_has_more_after(&mut self, v: bool) {
        self.has_more_after = v;
    }

    /// History has been fetched at least once.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub(crate) fn mark_loaded(&mut self) {
        self.loaded = true;
    }

    pub fn is_empty(&self) -> bool {
        self.msgs.is_empty()
    }

    /// The highest ts held, confirmed or local.
    pub fn newest_ts(&self) -> Option<&MessageTs> {
        let last = self.msgs.back().map(|m| &m.id.ts);
        last.max(self.newest_delivered_ts())
    }

    pub(crate) fn set_has_more_before(&mut self, v: bool) {
        self.has_more_before = v;
    }

    /// Replaces the contents with `msgs` (ascending), keeping at most
    /// `capacity` of them: the ones nearest `keep` when it is held.
    pub(crate) fn replace(
        &mut self,
        mut msgs: Vec<Message>,
        keep: Option<&MessageId>,
        before: bool,
        after: bool,
    ) {
        msgs.sort_by(|a, b| order_key(a).cmp(&order_key(b)));
        let pos = keep
            .and_then(|k| msgs.iter().position(|m| &m.id == k))
            .unwrap_or(msgs.len().saturating_sub(1));
        let extra = msgs.len().saturating_sub(self.capacity);
        let (mut before, mut after) = (before, after);
        if extra > 0 {
            // Trim evenly around `keep`, taking from the farther side first.
            let mut left = pos;
            let mut right = msgs.len() - 1 - pos;
            let (mut drop_l, mut drop_r) = (0, 0);
            for _ in 0..extra {
                if left >= right {
                    left -= 1;
                    drop_l += 1;
                } else {
                    right -= 1;
                    drop_r += 1;
                }
            }
            msgs.truncate(msgs.len() - drop_r);
            msgs.drain(..drop_l);
            before |= drop_l > 0;
            after |= drop_r > 0;
        }
        self.msgs = msgs.into();
        self.msgs.make_contiguous();
        self.has_more_before = before;
        self.has_more_after = after;
    }

    /// Index of the message with this id. Each part (confirmed, then local)
    /// is ts-sorted, so only the runs of equal ts values are scanned.
    fn index_of(&self, id: &MessageId) -> Option<usize> {
        [false, true].into_iter().find_map(|l| {
            let start = self
                .msgs
                .partition_point(|x| (local(x), &x.id.ts) < (l, &id.ts));
            self.msgs
                .iter()
                .enumerate()
                .skip(start)
                .take_while(|(_, x)| local(x) == l && x.id.ts == id.ts)
                .find(|(_, x)| x.id == *id)
                .map(|(i, _)| i)
        })
    }

    pub(crate) fn get_mut(&mut self, id: &MessageId) -> Option<&mut Message> {
        let i = self.index_of(id)?;
        self.msgs.get_mut(i)
    }

    pub(crate) fn remove(&mut self, id: &MessageId) -> bool {
        match self.index_of(id) {
            Some(i) => {
                self.msgs.remove(i);
                self.msgs.make_contiguous();
                true
            }
            None => false,
        }
    }

    /// A confirmed (non-pending) message with this id is held.
    pub(crate) fn has_delivered(&self, id: &MessageId) -> bool {
        self.msgs
            .iter()
            .any(|m| &m.id == id && m.state == DeliveryState::Delivered)
    }

    /// Newest message the provider has confirmed (pending ones carry local ts).
    pub fn newest_delivered_ts(&self) -> Option<&MessageTs> {
        self.msgs
            .iter()
            .rev()
            .find(|m| m.state == DeliveryState::Delivered)
            .map(|m| &m.id.ts)
    }

    pub(crate) fn find_client(&self, id: &ClientMsgId) -> Option<MessageId> {
        self.msgs
            .iter()
            .find(|m| m.client_msg_id.as_ref() == Some(id))
            .map(|m| m.id.clone())
    }

    /// Insert in ts order, replacing a message with the same id or the same
    /// client id. Drops the oldest past capacity and flags `has_more_before`.
    pub(crate) fn insert(&mut self, m: Message) {
        if let Some(c) = &m.client_msg_id
            && let Some(old) = self.find_client(c)
        {
            self.remove(&old);
        }
        self.remove(&m.id);
        let key = order_key(&m);
        let pos = self.msgs.partition_point(|x| order_key(x) <= key);
        self.msgs.insert(pos, m);
        while self.msgs.len() > self.capacity {
            self.msgs.pop_front();
            self.has_more_before = true;
        }
        self.msgs.make_contiguous();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{cmid, msg};

    fn local(t: &str, id: &str) -> Message {
        let mut m = msg("c1", t, id);
        m.client_msg_id = Some(cmid(id));
        m.state = DeliveryState::Pending;
        m
    }

    /// A local row and a confirmed row with the same ts share an id (the
    /// parts are searched separately): the confirmed one replaces the local
    /// one instead of sitting next to it.
    #[test]
    fn index_of_finds_a_local_row_with_the_ts_of_a_confirmed_one() {
        let mut w = MessageWindow::new(10);
        w.insert(msg("c1", "9.000000", "before"));
        w.insert(local("10.000000", "p"));
        w.insert(msg("c1", "11.000000", "after"));
        let id = w.find_client(&cmid("p")).unwrap();
        assert!(w.get_mut(&id).is_some(), "found in the local part");
        w.insert(msg("c1", "10.000000", "x"));
        let texts: Vec<&str> = w.messages().iter().map(|m| &*m.text).collect();
        assert_eq!(texts, ["before", "x", "after"]);
        assert!(w.has_delivered(&id));
        assert!(w.remove(&id));
        assert_eq!(w.messages().len(), 2);
    }

    #[test]
    fn newest_delivered_ignores_local_rows_at_the_bottom() {
        let mut w = MessageWindow::new(10);
        w.insert(msg("c1", "10.000000", "x"));
        w.insert(local("5.000000", "p"));
        assert_eq!(w.messages().last().map(|m| &*m.text), Some("p"));
        assert_eq!(w.newest_delivered_ts().map(|t| &*t.0), Some("10.000000"));
        assert_eq!(w.newest_ts().map(|t| &*t.0), Some("10.000000"));
    }
}
