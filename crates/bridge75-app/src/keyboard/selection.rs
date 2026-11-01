//! Which keys are selected for editing. No GPUI.

use std::collections::BTreeSet;

use bridge75_core::matrix::{KeyPos, LAYOUT};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeySelection {
    keys: BTreeSet<KeyPos>,
}

impl KeySelection {
    /// A plain click selects only `pos`; with `extend` it adds or removes `pos`.
    pub fn click(&mut self, pos: KeyPos, extend: bool) {
        if extend {
            self.toggle(pos);
        } else {
            self.select_only(pos);
        }
    }

    pub fn select_only(&mut self, pos: KeyPos) {
        self.keys.clear();
        self.keys.insert(pos);
    }

    /// Adds `pos`, or removes it if it is selected.
    pub fn toggle(&mut self, pos: KeyPos) {
        if !self.keys.remove(&pos) {
            self.keys.insert(pos);
        }
    }

    pub fn select_all(&mut self) {
        self.keys = LAYOUT.iter().map(|k| k.pos).collect();
    }

    pub fn clear(&mut self) {
        self.keys.clear();
    }

    pub fn contains(&self, pos: KeyPos) -> bool {
        self.keys.contains(&pos)
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// The first selected key in matrix order.
    pub fn first(&self) -> Option<KeyPos> {
        self.keys.first().copied()
    }

    /// Selected keys in matrix order.
    pub fn keys(&self) -> Vec<KeyPos> {
        self.keys.iter().copied().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(id: &str) -> KeyPos {
        id.parse().unwrap()
    }

    #[test]
    fn plain_click_replaces_and_extended_click_toggles() {
        let mut selection = KeySelection::default();
        selection.click(pos("w"), false);
        selection.click(pos("a"), true);
        assert_eq!(selection.keys(), [pos("w"), pos("a")]);
        selection.click(pos("w"), true);
        assert_eq!(selection.keys(), [pos("a")]);
        selection.click(pos("d"), false);
        assert_eq!(selection.keys(), [pos("d")]);
    }

    #[test]
    fn select_all_covers_every_key() {
        let mut selection = KeySelection::default();
        selection.select_all();
        assert_eq!(selection.len(), LAYOUT.len());
        selection.clear();
        assert!(selection.is_empty());
    }
}
