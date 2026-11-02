//! The assignments of both layers as the app knows them, and the changes that can be undone.
//! No GPUI.

use std::collections::{BTreeMap, VecDeque};

use bridge75_core::keymap::{KeyAction, KeyEntry, Keymap, Layer};
use bridge75_core::matrix::KeyPos;
use bridge75_core::profile::KeymapProfile;

/// How many changes are kept for undo. Older ones are forgotten.
const UNDO_LIMIT: usize = 100;

/// What `pos` sends by default. The default Fn layer is the same as the base layer, apart from
/// the keys whose Fn function the keyboard fixes.
pub fn default_action(pos: KeyPos) -> KeyAction {
    pos.key()
        .map_or(KeyAction::None, |key| KeyAction::from(key.default))
}

fn is_default(pos: KeyPos, action: KeyAction) -> bool {
    pos.key()
        .is_some_and(|key| KeyEntry::from(action) == key.default)
}

/// Both layers, kept as the keys that differ from the default.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KeyLayers {
    changes: KeymapProfile,
}

impl KeyLayers {
    /// Takes the changes of a stored profile. Entries that match the default are dropped.
    pub fn from_profile(mut profile: KeymapProfile) -> Self {
        for layer in [&mut profile.base, &mut profile.fn_layer] {
            layer.retain(|pos, action| !is_default(*pos, *action));
        }
        KeyLayers { changes: profile }
    }

    /// The keys that differ from the default, as a profile stores them.
    pub fn profile(&self) -> &KeymapProfile {
        &self.changes
    }

    fn layer(&self, layer: Layer) -> &BTreeMap<KeyPos, KeyAction> {
        match layer {
            Layer::Base => &self.changes.base,
            Layer::Fn => &self.changes.fn_layer,
        }
    }

    fn layer_mut(&mut self, layer: Layer) -> &mut BTreeMap<KeyPos, KeyAction> {
        match layer {
            Layer::Base => &mut self.changes.base,
            Layer::Fn => &mut self.changes.fn_layer,
        }
    }

    pub fn get(&self, layer: Layer, pos: KeyPos) -> KeyAction {
        self.layer(layer)
            .get(&pos)
            .copied()
            .unwrap_or_else(|| default_action(pos))
    }

    /// Whether `pos` sends its default on `layer`.
    pub fn is_default(&self, layer: Layer, pos: KeyPos) -> bool {
        !self.layer(layer).contains_key(&pos)
    }

    /// Whether any key differs from the default.
    pub fn is_changed(&self) -> bool {
        !self.changes.base.is_empty() || !self.changes.fn_layer.is_empty()
    }

    /// Assigns `action` to `pos` on `layer`. Returns what the key had before, or `None` if
    /// nothing changed.
    pub fn set(&mut self, layer: Layer, pos: KeyPos, action: KeyAction) -> Option<KeyAction> {
        let previous = self.get(layer, pos);
        if KeyEntry::from(previous) == KeyEntry::from(action) {
            return None;
        }
        let changes = self.layer_mut(layer);
        if is_default(pos, action) {
            changes.remove(&pos);
        } else {
            changes.insert(pos, action);
        }
        Some(previous)
    }

    /// Both layers in full, as they are written.
    pub fn build(&self) -> (Keymap, Keymap) {
        self.changes.build()
    }

    /// Whether some key of the base layer is Fn. Without one, the Fn layer can't be used.
    pub fn has_fn_key(&self) -> bool {
        let (base, _) = self.build();
        base.0.contains(&KeyEntry::FN)
    }
}

/// One key's assignment before a change.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Change {
    pub layer: Layer,
    pub pos: KeyPos,
    pub previous: KeyAction,
}

/// Changes that can be undone, newest last. One entry may change several keys.
#[derive(Clone, Debug, Default)]
pub struct UndoStack {
    entries: VecDeque<Vec<Change>>,
}

impl UndoStack {
    pub fn push(&mut self, changes: Vec<Change>) {
        if changes.is_empty() {
            return;
        }
        if self.entries.len() == UNDO_LIMIT {
            self.entries.pop_front();
        }
        self.entries.push_back(changes);
    }

    /// Takes the newest change.
    pub fn pop(&mut self) -> Option<Vec<Change>> {
        self.entries.pop_back()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use bridge75_core::hid::Usage;

    use super::*;

    fn pos(id: &str) -> KeyPos {
        id.parse().unwrap()
    }

    fn key(name: &str) -> KeyAction {
        KeyAction::Key(name.parse::<Usage>().unwrap())
    }

    #[test]
    fn unchanged_keys_send_their_default_on_both_layers() {
        let layers = KeyLayers::default();
        assert_eq!(layers.get(Layer::Base, pos("caps")), key("caps"));
        assert_eq!(layers.get(Layer::Fn, pos("caps")), key("caps"));
        assert_eq!(layers.get(Layer::Base, pos("fn")), KeyAction::Fn);
        assert!(layers.has_fn_key());
        assert!(!layers.is_changed());
    }

    #[test]
    fn setting_reports_the_previous_assignment_once() {
        let mut layers = KeyLayers::default();
        assert_eq!(layers.set(Layer::Fn, pos("h"), key("left")), Some(key("h")));
        assert_eq!(layers.set(Layer::Fn, pos("h"), key("left")), None);
        assert_eq!(layers.get(Layer::Fn, pos("h")), key("left"));
        // The base layer is a separate assignment.
        assert_eq!(layers.get(Layer::Base, pos("h")), key("h"));
        assert!(!layers.is_default(Layer::Fn, pos("h")));
    }

    #[test]
    fn going_back_to_the_default_leaves_nothing_stored() {
        let mut layers = KeyLayers::default();
        layers.set(Layer::Base, pos("caps"), key("lctrl"));
        layers.set(Layer::Base, pos("caps"), key("caps"));
        assert!(layers.is_default(Layer::Base, pos("caps")));
        assert_eq!(*layers.profile(), KeymapProfile::default());
    }

    #[test]
    fn stored_entries_equal_to_the_default_are_dropped() {
        let mut profile = KeymapProfile::default();
        profile.base.insert(pos("a"), key("a"));
        profile.base.insert(pos("b"), key("c"));
        // The raw form of the default is the default too.
        let raw = KeyAction::Raw(KeyEntry::key(0x06));
        profile.fn_layer.insert(pos("c"), raw);
        let layers = KeyLayers::from_profile(profile);
        assert_eq!(layers.profile().base.len(), 1);
        assert!(layers.profile().fn_layer.is_empty());
    }

    #[test]
    fn the_fn_layer_needs_an_fn_key_on_the_base_layer() {
        let mut layers = KeyLayers::default();
        layers.set(Layer::Base, pos("fn"), key("ralt"));
        assert!(!layers.has_fn_key());
        layers.set(Layer::Base, pos("ralt"), KeyAction::Fn);
        assert!(layers.has_fn_key());
    }

    #[test]
    fn the_written_layers_hold_every_key() {
        let mut layers = KeyLayers::default();
        layers.set(Layer::Fn, pos("j"), key("down"));
        let (base, fn_layer) = layers.build();
        assert_eq!(base, Keymap::default_layer());
        assert_eq!(fn_layer.get(pos("j")), KeyEntry::key(0x51));
        assert_eq!(fn_layer.get(pos("k")), KeyEntry::key(0x0E));
    }

    #[test]
    fn undo_pops_the_newest_change_and_keeps_a_bounded_number() {
        let change = |previous| Change {
            layer: Layer::Base,
            pos: pos("a"),
            previous,
        };
        let mut stack = UndoStack::default();
        stack.push(Vec::new());
        assert!(stack.is_empty());
        for _ in 0..UNDO_LIMIT + 5 {
            stack.push(vec![change(key("a"))]);
        }
        stack.push(vec![change(key("b"))]);
        assert_eq!(stack.entries.len(), UNDO_LIMIT);
        assert_eq!(stack.pop(), Some(vec![change(key("b"))]));
    }
}
