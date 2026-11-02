//! SOCD pairs and the switch as last read from or written to the keyboard, and the changes
//! that can be undone. No GPUI.

use std::collections::VecDeque;

use bridge75_core::hid::Usage;
use bridge75_core::keymap::SOCD_PAIRS;
use bridge75_core::socd::{SocdBehavior, SocdMode, SocdPair, SocdPairs, SocdStatus};

/// How many changes are kept for undo. Older ones are forgotten.
const UNDO_LIMIT: usize = 100;

/// Enabled flags of every pair, as the switch sets them (bit(19 - i) is pair i).
const ALL_FLAGS: u32 = (1 << SOCD_PAIRS) - 1;

/// The keys a new pair takes, the first that no pair has yet: the pairs SOCD is most often set
/// up for.
const NEW_PAIR_KEYS: [(Usage, Usage); 4] = [
    (Usage(0x04), Usage(0x07)), // A, D
    (Usage(0x1A), Usage(0x16)), // W, S
    (Usage(0x50), Usage(0x4F)), // Left, Right
    (Usage(0x52), Usage(0x51)), // Up, Down
];

/// A pair with `a` and `b`, as a new pair starts.
const fn new_pair(a: Usage, b: Usage) -> SocdPair {
    SocdPair {
        mode: SocdMode::Always,
        behavior: SocdBehavior::Last,
        a,
        b,
    }
}

/// One change, as what it replaced.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Pairs {
        previous: SocdPairs,
        /// Document revision of the pairs after the change.
        revision: u64,
    },
    Switch {
        previous: bool,
        /// Identifies the write of the change.
        write: u64,
    },
}

/// What the page shows: the pairs and the status. Small enough to copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SocdState {
    pairs: SocdPairs,
    status: SocdStatus,
}

impl SocdState {
    pub fn pairs(&self) -> &SocdPairs {
        &self.pairs
    }

    pub fn pair(&self, index: usize) -> SocdPair {
        self.pairs[index]
    }

    /// Indices of the slots that hold a pair, in order.
    pub fn used(&self) -> impl Iterator<Item = usize> + '_ {
        (0..self.pairs.len()).filter(|i| !self.pairs[*i].is_empty())
    }

    /// The first slot without a pair.
    pub fn first_free(&self) -> Option<usize> {
        self.pairs.iter().position(SocdPair::is_empty)
    }

    /// What a new pair starts as: the first usual pair of keys no pair has yet, or no keys at
    /// all once every usual pair is taken, so no two pairs are the same.
    pub fn next_new_pair(&self) -> SocdPair {
        let taken = |a: Usage, b: Usage| {
            self.pairs
                .iter()
                .any(|p| (p.a, p.b) == (a, b) || (p.a, p.b) == (b, a))
        };
        NEW_PAIR_KEYS
            .into_iter()
            .find(|(a, b)| !taken(*a, *b))
            .map_or(new_pair(Usage::NONE, Usage::NONE), |(a, b)| new_pair(a, b))
    }

    /// Whether the switch is on.
    pub fn switch(&self) -> bool {
        self.status.global
    }

    /// Whether `index` takes effect when its keys are pressed together.
    pub fn is_active(&self, index: usize) -> bool {
        self.status.is_active(index as u8, self.pairs[index].mode)
    }

    /// Whether every enabled flag is what the switch sets it to. Keys that toggle one pair
    /// make them differ.
    fn flags_follow_switch(&self) -> bool {
        self.status.enabled_mask == if self.status.global { ALL_FLAGS } else { 0 }
    }
}

/// The SOCD settings of one connected keyboard.
#[derive(Clone, Debug)]
pub struct SocdSettings {
    state: SocdState,
    undo: VecDeque<Change>,
}

impl SocdSettings {
    pub fn new(pairs: SocdPairs, status: SocdStatus) -> Self {
        SocdSettings {
            state: SocdState { pairs, status },
            undo: VecDeque::new(),
        }
    }

    pub fn state(&self) -> &SocdState {
        &self.state
    }

    /// Takes the status read from the keyboard: the switch and the enabled flags.
    ///
    /// Undoing a switch change sets every flag, so once keys have toggled single pairs it
    /// would not bring back what the change replaced; such changes are forgotten.
    pub fn set_status(&mut self, status: SocdStatus) {
        self.state.status = status;
        if !self.state.flags_follow_switch() {
            self.undo.retain(|c| !matches!(c, Change::Switch { .. }));
        }
    }

    /// Takes the pairs read back after a failed write, and forgets the changes of the pairs
    /// after `written`, the last revision the keyboard took: those never reached it, and the
    /// changes after them recorded what they replaced on top of them.
    pub fn recover_pairs(&mut self, pairs: SocdPairs, written: u64) {
        self.state.pairs = pairs;
        self.undo
            .retain(|c| !matches!(c, Change::Pairs { revision, .. } if *revision > written));
    }

    /// Forgets the switch change written by `write`, which failed.
    pub fn forget_switch_write(&mut self, write: u64) {
        self.undo
            .retain(|c| !matches!(c, Change::Switch { write: w, .. } if *w == write));
    }

    /// Puts `pair` in slot `index` as change `revision`. Returns whether anything changed.
    pub fn set_pair(&mut self, index: usize, pair: SocdPair, revision: u64) -> bool {
        if self.state.pairs[index] == pair {
            return false;
        }
        self.push(Change::Pairs {
            previous: self.state.pairs,
            revision,
        });
        self.state.pairs[index] = pair;
        true
    }

    /// Turns the switch on or off as switch write `write`, which sets the enabled flag of
    /// every pair too. Returns whether anything changed.
    pub fn set_switch(&mut self, on: bool, write: u64) -> bool {
        if self.state.status.global == on {
            return false;
        }
        self.push(Change::Switch {
            previous: self.state.status.global,
            write,
        });
        self.apply_switch(on);
        true
    }

    fn apply_switch(&mut self, on: bool) {
        self.state.status = SocdStatus {
            global: on,
            enabled_mask: if on { ALL_FLAGS } else { 0 },
        };
    }

    fn push(&mut self, change: Change) {
        if self.undo.len() == UNDO_LIMIT {
            self.undo.pop_front();
        }
        self.undo.push_back(change);
    }

    pub fn has_undo_history(&self) -> bool {
        !self.undo.is_empty()
    }

    /// Puts back what the newest change replaced. Returns that change, so it can be written.
    /// A switch change whose setting the switch has again (keys on the keyboard turned it back)
    /// would change nothing, so it is skipped.
    pub fn undo(&mut self) -> Option<Change> {
        loop {
            let change = self.undo.pop_back()?;
            match change {
                Change::Pairs { previous, .. } => self.state.pairs = previous,
                Change::Switch { previous, .. } if previous == self.state.status.global => {
                    continue;
                }
                Change::Switch { previous, .. } => self.apply_switch(previous),
            }
            return Some(change);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(name: &str) -> Usage {
        name.parse().unwrap()
    }

    fn off() -> SocdStatus {
        SocdStatus {
            global: false,
            enabled_mask: 0,
        }
    }

    fn settings() -> SocdSettings {
        SocdSettings::new(SocdPairs::default(), off())
    }

    fn a_d() -> SocdPair {
        new_pair(usage("a"), usage("d"))
    }

    #[test]
    fn pairs_fill_the_first_free_slot() {
        let mut settings = settings();
        assert_eq!(settings.state().first_free(), Some(0));
        settings.set_pair(0, a_d(), 1);
        settings.set_pair(2, a_d(), 2);
        assert_eq!(settings.state().first_free(), Some(1));
        assert_eq!(settings.state().used().collect::<Vec<_>>(), [0, 2]);
        for i in 0..SOCD_PAIRS as usize {
            settings.set_pair(i, a_d(), 3);
        }
        assert_eq!(settings.state().first_free(), None);
    }

    #[test]
    fn a_new_pair_takes_keys_no_pair_has() {
        let mut settings = settings();
        assert_eq!(settings.state().next_new_pair(), a_d());
        // Either order counts as taken.
        settings.set_pair(0, new_pair(usage("d"), usage("a")), 1);
        let next = settings.state().next_new_pair();
        assert_eq!((next.a, next.b), (usage("w"), usage("s")));
        for (i, (a, b)) in NEW_PAIR_KEYS.into_iter().enumerate() {
            settings.set_pair(i, new_pair(a, b), 2);
        }
        let blank = settings.state().next_new_pair();
        assert_eq!((blank.a, blank.b), (Usage::NONE, Usage::NONE));
        assert!(!blank.is_empty());
    }

    #[test]
    fn the_switch_turns_on_the_pairs_that_follow_it() {
        let mut settings = settings();
        let follows = SocdPair {
            mode: SocdMode::Global,
            ..a_d()
        };
        settings.set_pair(0, follows, 1);
        settings.set_pair(1, a_d(), 2);
        settings.set_pair(
            2,
            SocdPair {
                mode: SocdMode::Off,
                ..a_d()
            },
            3,
        );
        assert!(!settings.state().is_active(0));
        assert!(settings.state().is_active(1));
        assert!(settings.set_switch(true, 1));
        assert!(settings.state().is_active(0));
        assert!(!settings.state().is_active(2));
        assert!(!settings.set_switch(true, 2));
    }

    #[test]
    fn undo_puts_back_pairs_and_the_switch_newest_first() {
        let mut settings = settings();
        assert!(!settings.set_pair(0, SocdPair::default(), 1));
        assert!(!settings.has_undo_history());

        settings.set_pair(0, a_d(), 1);
        settings.set_switch(true, 1);
        let ws = new_pair(usage("w"), usage("s"));
        settings.set_pair(0, ws, 2);

        assert!(matches!(settings.undo(), Some(Change::Pairs { .. })));
        assert_eq!(settings.state().pair(0), a_d());
        assert!(matches!(
            settings.undo(),
            Some(Change::Switch {
                previous: false,
                ..
            })
        ));
        assert!(!settings.state().switch());
        assert_eq!(settings.state().status.enabled_mask, 0);
        settings.undo();
        assert!(settings.state().pair(0).is_empty());
        assert_eq!(settings.undo(), None);
    }

    #[test]
    fn switch_changes_are_forgotten_once_keys_toggle_single_pairs() {
        let mut settings = settings();
        settings.set_switch(true, 1);
        settings.set_pair(0, a_d(), 1);
        // A key turned pair 3 off on its own.
        settings.set_status(SocdStatus {
            global: true,
            enabled_mask: ALL_FLAGS & !(1 << 16),
        });
        assert!(matches!(settings.undo(), Some(Change::Pairs { .. })));
        assert_eq!(settings.undo(), None);
    }

    #[test]
    fn undo_skips_a_switch_change_the_keyboard_already_undid() {
        let mut settings = settings();
        settings.set_pair(0, a_d(), 1);
        settings.set_switch(true, 1);
        // The key that toggles SOCD turned the switch off again.
        settings.set_status(off());
        assert!(matches!(settings.undo(), Some(Change::Pairs { .. })));
        assert!(!settings.has_undo_history());
    }

    #[test]
    fn a_failed_write_forgets_only_its_own_changes() {
        let mut settings = settings();
        settings.set_pair(0, a_d(), 1);
        settings.set_switch(true, 7);
        settings.set_pair(1, a_d(), 2);
        settings.set_pair(2, a_d(), 3);
        // Revision 1 reached the keyboard; 2 failed, and 3 had not been written yet.
        let mut actual = SocdPairs::default();
        actual[0] = a_d();
        settings.recover_pairs(actual, 1);
        assert!(settings.state().pair(1).is_empty());
        assert_eq!(settings.undo.len(), 2);

        settings.forget_switch_write(7);
        assert!(matches!(
            settings.undo(),
            Some(Change::Pairs { revision: 1, .. })
        ));
        assert_eq!(settings.undo(), None);
    }

    #[test]
    fn undo_keeps_a_bounded_number_of_changes() {
        let mut settings = settings();
        for i in 0..UNDO_LIMIT + 5 {
            settings.set_switch(i % 2 == 0, i as u64);
        }
        assert_eq!(settings.undo.len(), UNDO_LIMIT);
    }
}
