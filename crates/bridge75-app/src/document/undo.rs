//! The changes that can be undone, newest last. No GPUI.

use std::collections::VecDeque;

use bridge75_core::he::TriggerConfig;
use bridge75_core::matrix::KeyPos;

/// How many changes are kept. Older ones are forgotten.
const LIMIT: usize = 100;

/// One change: the settings its keys had before it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndoEntry {
    /// Document revision of the change.
    pub revision: u64,
    pub previous: Vec<(KeyPos, TriggerConfig)>,
}

impl UndoEntry {
    fn touches(&self, keys: &[KeyPos]) -> bool {
        self.previous.iter().any(|(pos, _)| keys.contains(pos))
    }
}

#[derive(Clone, Debug, Default)]
pub struct UndoStack {
    entries: VecDeque<UndoEntry>,
}

impl UndoStack {
    pub fn push(&mut self, revision: u64, previous: Vec<(KeyPos, TriggerConfig)>) {
        if previous.is_empty() {
            return;
        }
        if self.entries.len() == LIMIT {
            self.entries.pop_front();
        }
        self.entries.push_back(UndoEntry { revision, previous });
    }

    /// Takes the newest change.
    pub fn pop(&mut self) -> Option<UndoEntry> {
        self.entries.pop_back()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Forgets change `revision`, whose write failed, and the later changes of the same keys:
    /// the settings those recorded as "before" came from the change that never reached the
    /// keyboard.
    pub fn discard(&mut self, revision: u64) {
        let Some(failed) = self.entries.iter().find(|e| e.revision == revision) else {
            return;
        };
        let keys: Vec<KeyPos> = failed.previous.iter().map(|(pos, _)| *pos).collect();
        self.entries
            .retain(|e| e.revision < revision || (e.revision > revision && !e.touches(&keys)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use bridge75_core::he::{Travel, TriggerMode};

    fn pos(id: &str) -> KeyPos {
        id.parse().unwrap()
    }

    fn config(tenths: u8) -> TriggerConfig {
        let t = Travel::new(tenths).unwrap();
        TriggerConfig {
            mode: TriggerMode::Normal,
            actuation: t,
            release: t,
            ..TriggerConfig::DEFAULT
        }
    }

    fn revisions(stack: &UndoStack) -> Vec<u64> {
        stack.entries.iter().map(|e| e.revision).collect()
    }

    #[test]
    fn pops_the_newest_change_first() {
        let mut stack = UndoStack::default();
        stack.push(1, vec![(pos("w"), config(18))]);
        stack.push(2, vec![(pos("w"), config(15))]);
        assert_eq!(stack.pop().unwrap().previous, [(pos("w"), config(15))]);
        assert_eq!(stack.pop().unwrap().revision, 1);
        assert!(stack.pop().is_none());
        assert!(stack.is_empty());
    }

    #[test]
    fn changes_that_changed_nothing_are_not_kept() {
        let mut stack = UndoStack::default();
        stack.push(1, Vec::new());
        assert!(stack.is_empty());
    }

    #[test]
    fn keeps_a_bounded_number_of_changes() {
        let mut stack = UndoStack::default();
        for revision in 1..=LIMIT as u64 + 5 {
            stack.push(revision, vec![(pos("w"), config(18))]);
        }
        assert_eq!(stack.entries.len(), LIMIT);
        assert_eq!(stack.entries.front().unwrap().revision, 6);
        stack.clear();
        assert!(stack.is_empty());
    }

    #[test]
    fn a_failed_change_takes_later_changes_of_its_keys_with_it() {
        let mut stack = UndoStack::default();
        stack.push(1, vec![(pos("w"), config(18))]);
        stack.push(2, vec![(pos("w"), config(15)), (pos("a"), config(18))]);
        stack.push(3, vec![(pos("a"), config(12))]);
        stack.push(4, vec![(pos("s"), config(18))]);
        stack.discard(2);
        assert_eq!(revisions(&stack), [1, 4]);
        // A change that is not kept any more is ignored.
        stack.discard(2);
        assert_eq!(revisions(&stack), [1, 4]);
    }
}
