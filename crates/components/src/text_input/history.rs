//! Undo and redo for the text input: whole-text snapshots (composer text is small),
//! typing runs coalesced into one step, at most [`CAP`] steps.

use std::collections::VecDeque;

use domain::ComposedSpan;

use super::buffer::Selection;

/// The most undo steps kept.
pub const CAP: usize = 100;

#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub text: String,
    pub sel: Selection,
    pub spans: Vec<ComposedSpan>,
}

#[derive(Debug, Default)]
pub struct History {
    undo: VecDeque<Snapshot>,
    redo: Vec<Snapshot>,
    /// The last recorded edit was typing and nothing has interrupted the run.
    run: bool,
}

impl History {
    /// Records the state before an edit. A typing edit right after another one joins its step.
    pub fn record(&mut self, before: Snapshot, typing: bool) {
        if typing && self.run {
            return;
        }
        self.undo.push_back(before);
        if self.undo.len() > CAP {
            self.undo.pop_front();
        }
        self.redo.clear();
        self.run = typing;
    }

    /// A caret move or another non-edit interrupts the typing run.
    pub fn break_run(&mut self) {
        self.run = false;
    }

    pub fn undo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let s = self.undo.pop_back()?;
        self.redo.push(current);
        self.run = false;
        Some(s)
    }

    pub fn redo(&mut self, current: Snapshot) -> Option<Snapshot> {
        let s = self.redo.pop()?;
        self.undo.push_back(current);
        self.run = false;
        Some(s)
    }

    pub fn clear(&mut self) {
        self.undo.clear();
        self.redo.clear();
        self.run = false;
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.undo.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snap(t: &str) -> Snapshot {
        Snapshot {
            text: t.into(),
            sel: Selection::caret(t.len()),
            spans: Vec::new(),
        }
    }

    #[test]
    fn typing_runs_join_and_other_edits_do_not() {
        let mut h = History::default();
        h.record(snap(""), true);
        h.record(snap("a"), true);
        h.record(snap("ab"), true);
        assert_eq!(h.len(), 1);
        h.record(snap("abc"), false);
        h.record(snap("abc!"), true);
        assert_eq!(h.len(), 3);
        h.break_run();
        h.record(snap("abc!d"), true);
        assert_eq!(h.len(), 4);
    }

    #[test]
    fn capped_at_one_hundred() {
        let mut h = History::default();
        for i in 0..250 {
            h.record(snap(&i.to_string()), false);
        }
        assert_eq!(h.len(), CAP);
    }

    #[test]
    fn undo_then_redo_and_new_edit_clears_redo() {
        let mut h = History::default();
        h.record(snap(""), false);
        assert_eq!(h.undo(snap("a")), Some(snap("")));
        assert_eq!(h.redo(snap("")), Some(snap("a")));
        h.undo(snap("a"));
        h.record(snap(""), false);
        assert_eq!(h.redo(snap("b")), None);
    }
}
