//! Trigger settings of every key, as last read from or written to the keyboard. No GPUI.

use std::collections::BTreeMap;

use bridge75_core::he::{MAX_TRAVEL, Travel, TriggerConfig, TriggerMode};
use bridge75_core::matrix::{KeyBitmap, KeyPos};

/// Smallest travel the app writes. 0 mm would keep the key pressed.
pub const MIN_TRAVEL: Travel = Travel::from_raw(1);

/// Whether the app may write `travel` as it is.
fn is_writable(travel: Travel) -> bool {
    (MIN_TRAVEL..=Travel::MAX).contains(&travel)
}

/// One travel setting of a key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TriggerField {
    Actuation,
    Release,
    RtPress,
    RtRelease,
}

impl TriggerField {
    pub const ALL: [TriggerField; 4] = [
        TriggerField::Actuation,
        TriggerField::Release,
        TriggerField::RtPress,
        TriggerField::RtRelease,
    ];

    /// Position in [`TriggerField::ALL`].
    pub fn ix(self) -> usize {
        self as usize
    }

    pub fn get(self, config: &TriggerConfig) -> Travel {
        match self {
            TriggerField::Actuation => config.actuation,
            TriggerField::Release => config.release,
            TriggerField::RtPress => config.rt_press,
            TriggerField::RtRelease => config.rt_release,
        }
    }

    fn set(self, config: &mut TriggerConfig, travel: Travel) {
        match self {
            TriggerField::Actuation => config.actuation = travel,
            TriggerField::Release => config.release = travel,
            TriggerField::RtPress => config.rt_press = travel,
            TriggerField::RtRelease => config.rt_release = travel,
        }
    }

    /// Whether a key in `mode` acts on this field.
    pub fn is_used_in(self, mode: TriggerMode) -> bool {
        match self {
            TriggerField::Actuation => true,
            TriggerField::Release => mode.uses_release_point(),
            TriggerField::RtPress | TriggerField::RtRelease => mode.is_rapid(),
        }
    }
}

/// `travel` moved by `delta` × 0.1 mm, kept within the range the app writes.
pub fn stepped(travel: Travel, delta: i8) -> Travel {
    let raw =
        (travel.raw() as i16 + delta as i16).clamp(MIN_TRAVEL.raw() as i16, MAX_TRAVEL as i16);
    Travel::from_raw(raw as u8)
}

/// `config` with every value the app may not write replaced, ready to be written.
///
/// The keyboard can report values outside 0.1..=3.6 mm: the FW does not initialize the RT
/// sensitivity, so it may be 0 or anything up to 25.5 mm. Writing such a value back would either
/// be rejected or keep the key pressed. The release point of a mode without one follows the
/// actuation point, as the official configurator sends it.
pub fn normalized(mut config: TriggerConfig) -> TriggerConfig {
    if !is_writable(config.actuation) {
        config.actuation = TriggerConfig::DEFAULT.actuation;
    }
    if !config.mode.uses_release_point() || !is_writable(config.release) {
        config.release = config.actuation;
    }
    for rt in [&mut config.rt_press, &mut config.rt_release] {
        if !is_writable(*rt) {
            *rt = TriggerConfig::DEFAULT_RT;
        }
    }
    config
}

/// A change to the trigger settings, applied to each selected key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TriggerEdit {
    Mode(TriggerMode),
    /// Sets the field on each key whose mode uses it.
    Set(TriggerField, Travel),
    /// Moves the field by the given number of 0.1 mm steps on each key whose mode uses it, so
    /// keys with different values keep their differences.
    Step(TriggerField, i8),
}

impl TriggerEdit {
    /// The settings after the edit, ready to be written. A key the edit does not apply to keeps
    /// its settings unchanged.
    pub fn apply(self, config: TriggerConfig) -> TriggerConfig {
        let mut next = config;
        match self {
            TriggerEdit::Mode(mode) => next.mode = mode,
            TriggerEdit::Set(field, travel) if field.is_used_in(config.mode) => {
                field.set(&mut next, travel)
            }
            TriggerEdit::Step(field, delta) if field.is_used_in(config.mode) => {
                field.set(&mut next, stepped(field.get(&config), delta))
            }
            TriggerEdit::Set(..) | TriggerEdit::Step(..) => return config,
        }
        normalized(next)
    }
}

/// A change applied to the table that still has to be written to the keyboard.
#[derive(Debug)]
pub struct PendingWrite {
    /// Document revision of the change.
    pub revision: u64,
    /// Settings the changed keys had before, in key order.
    pub previous: Vec<(KeyPos, TriggerConfig)>,
    /// One write per distinct resulting setting.
    pub writes: Vec<(KeyBitmap, TriggerConfig)>,
}

impl PendingWrite {
    pub fn is_empty(&self) -> bool {
        self.writes.is_empty()
    }

    /// The keys the change writes.
    pub fn keys(&self) -> Vec<KeyPos> {
        self.previous.iter().map(|(pos, _)| *pos).collect()
    }
}

#[derive(Clone, Debug, Default)]
pub struct TriggerTable {
    configs: BTreeMap<KeyPos, TriggerConfig>,
    /// Document revision of the latest change of each key.
    changed: BTreeMap<KeyPos, u64>,
    /// Document revision when the settings were read. Writes queued up to then are already in
    /// them, because the keyboard runs requests in order.
    read_at: u64,
}

impl TriggerTable {
    pub fn new(configs: impl IntoIterator<Item = (KeyPos, TriggerConfig)>, read_at: u64) -> Self {
        TriggerTable {
            configs: configs.into_iter().collect(),
            changed: BTreeMap::new(),
            read_at,
        }
    }

    pub fn get(&self, pos: KeyPos) -> Option<&TriggerConfig> {
        self.configs.get(&pos)
    }

    /// Applies `edit` to `keys` as change `revision` and returns the writes that make the
    /// keyboard match.
    ///
    /// Keys whose settings differ keep their other fields, so a mixed selection may need
    /// several writes. Keys the edit does not change are not written.
    pub fn apply(
        &mut self,
        keys: impl IntoIterator<Item = KeyPos>,
        edit: TriggerEdit,
        revision: u64,
    ) -> PendingWrite {
        let changes: Vec<(KeyPos, TriggerConfig)> = keys
            .into_iter()
            .filter_map(|pos| Some((pos, edit.apply(*self.configs.get(&pos)?))))
            .collect();
        self.change(changes, revision)
    }

    /// Puts back settings an earlier change replaced, as change `revision`.
    pub fn restore(&mut self, configs: &[(KeyPos, TriggerConfig)], revision: u64) -> PendingWrite {
        let changes = configs
            .iter()
            .map(|(pos, config)| (*pos, normalized(*config)))
            .collect();
        self.change(changes, revision)
    }

    fn change(&mut self, changes: Vec<(KeyPos, TriggerConfig)>, revision: u64) -> PendingWrite {
        let mut previous = Vec::new();
        let mut writes: Vec<(KeyBitmap, TriggerConfig)> = Vec::new();
        for (pos, next) in changes {
            let Some(config) = self.configs.get_mut(&pos) else {
                continue;
            };
            if next == *config {
                continue;
            }
            previous.push((pos, *config));
            *config = next;
            self.changed.insert(pos, revision);
            match writes.iter_mut().find(|(_, c)| *c == next) {
                Some((bitmap, _)) => bitmap.insert(pos),
                None => writes.push(([pos].into_iter().collect(), next)),
            }
        }
        PendingWrite {
            revision,
            previous,
            writes,
        }
    }

    /// Takes the settings read back from the keyboard after the write of change `revision`
    /// failed. Keys changed again since keep their newer settings, which are written after it.
    pub fn recover(
        &mut self,
        revision: u64,
        actual: impl IntoIterator<Item = (KeyPos, TriggerConfig)>,
    ) {
        if revision <= self.read_at {
            return;
        }
        for (pos, config) in actual {
            let newer = self.changed.get(&pos).is_some_and(|r| *r > revision);
            if !newer && let Some(current) = self.configs.get_mut(&pos) {
                *current = config;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(id: &str) -> KeyPos {
        id.parse().unwrap()
    }

    fn mm(tenths: u8) -> Travel {
        Travel::new(tenths).unwrap()
    }

    fn rapid() -> TriggerConfig {
        TriggerConfig {
            mode: TriggerMode::Rapid,
            actuation: mm(10),
            release: mm(10),
            rt_press: mm(3),
            rt_release: mm(3),
        }
    }

    fn table() -> TriggerTable {
        TriggerTable::new(
            [
                (pos("w"), TriggerConfig::DEFAULT),
                (pos("a"), TriggerConfig::DEFAULT),
                (pos("s"), rapid()),
            ],
            0,
        )
    }

    #[test]
    fn edit_keeps_other_fields_of_mixed_keys() {
        let mut table = table();
        let pending = table.apply(
            [pos("w"), pos("a"), pos("s")],
            TriggerEdit::Set(TriggerField::Actuation, mm(15)),
            1,
        );
        assert_eq!(pending.writes.len(), 2);
        let (keys, normal) = pending.writes[0];
        assert_eq!(keys, [pos("w"), pos("a")].into_iter().collect());
        assert_eq!(
            (normal.mode, normal.actuation),
            (TriggerMode::Normal, mm(15))
        );
        let (keys, rapid) = pending.writes[1];
        assert_eq!(keys, [pos("s")].into_iter().collect());
        assert_eq!((rapid.mode, rapid.rt_press), (TriggerMode::Rapid, mm(3)));
        assert_eq!(table.get(pos("s")).unwrap().actuation, mm(15));
    }

    #[test]
    fn release_follows_actuation_without_a_release_point() {
        let mut table = table();
        let w = [pos("w")];
        table.apply(w, TriggerEdit::Set(TriggerField::Actuation, mm(12)), 1);
        assert_eq!(table.get(pos("w")).unwrap().release, mm(12));

        table.apply(w, TriggerEdit::Mode(TriggerMode::Split), 2);
        table.apply(w, TriggerEdit::Set(TriggerField::Release, mm(8)), 3);
        table.apply(w, TriggerEdit::Set(TriggerField::Actuation, mm(20)), 4);
        let w = table.get(pos("w")).unwrap();
        assert_eq!((w.actuation, w.release), (mm(20), mm(8)));
    }

    #[test]
    fn unchanged_keys_are_not_written() {
        let mut table = table();
        let pending = table.apply(
            [pos("w"), pos("s")],
            TriggerEdit::Mode(TriggerMode::Rapid),
            1,
        );
        assert_eq!(pending.writes.len(), 1);
        assert_eq!(pending.writes[0].0, [pos("w")].into_iter().collect());
        assert_eq!(pending.keys(), [pos("w")]);
    }

    #[test]
    fn fields_apply_only_to_keys_whose_mode_uses_them() {
        let mut table = table();
        let pending = table.apply(
            [pos("w"), pos("s")],
            TriggerEdit::Set(TriggerField::RtPress, mm(7)),
            1,
        );
        assert_eq!(pending.keys(), [pos("s")]);
        assert_eq!(
            table.get(pos("w")).unwrap().rt_press,
            TriggerConfig::DEFAULT_RT
        );
        assert_eq!(table.get(pos("s")).unwrap().rt_press, mm(7));
    }

    #[test]
    fn steps_move_each_key_from_its_own_value() {
        let mut table = table();
        let keys = [pos("w"), pos("s")];
        table.apply(keys, TriggerEdit::Step(TriggerField::Actuation, 1), 1);
        assert_eq!(table.get(pos("w")).unwrap().actuation, mm(19));
        assert_eq!(table.get(pos("s")).unwrap().actuation, mm(11));

        // Only the Rapid Trigger key has a press sensitivity to change.
        let pending = table.apply(keys, TriggerEdit::Step(TriggerField::RtPress, -1), 2);
        assert_eq!(pending.keys(), [pos("s")]);
        assert_eq!(table.get(pos("s")).unwrap().rt_press, mm(2));
    }

    #[test]
    fn steps_stay_within_the_writable_range() {
        assert_eq!(stepped(mm(1), -1), mm(1));
        assert_eq!(stepped(Travel::MAX, 1), Travel::MAX);
        assert_eq!(stepped(Travel::from_raw(0), -1), MIN_TRAVEL);
        assert_eq!(stepped(Travel::from_raw(0), 1), MIN_TRAVEL);
        assert_eq!(stepped(Travel::from_raw(200), 1), Travel::MAX);
    }

    #[test]
    fn values_the_keyboard_never_initialized_are_replaced_before_writing() {
        let unset = TriggerConfig {
            mode: TriggerMode::Normal,
            actuation: mm(18),
            release: mm(18),
            rt_press: Travel::from_raw(0),
            rt_release: Travel::from_raw(200),
        };
        let mut table = TriggerTable::new([(pos("q"), unset)], 0);
        // The stored values are shown as they are until the key is written.
        assert_eq!(
            table.get(pos("q")).unwrap().rt_release,
            Travel::from_raw(200)
        );

        let pending = table.apply([pos("q")], TriggerEdit::Mode(TriggerMode::Rapid), 1);
        let (_, written) = pending.writes[0];
        assert_eq!(written.rt_press, TriggerConfig::DEFAULT_RT);
        assert_eq!(written.rt_release, TriggerConfig::DEFAULT_RT);
        assert!(written.validate().is_ok());
        assert_eq!(*table.get(pos("q")).unwrap(), written);
    }

    #[test]
    fn normalizing_replaces_every_unwritable_value() {
        let broken = TriggerConfig {
            mode: TriggerMode::Split,
            actuation: Travel::from_raw(0),
            release: Travel::from_raw(40),
            rt_press: Travel::from_raw(37),
            rt_release: mm(36),
        };
        let fixed = normalized(broken);
        assert_eq!(fixed.actuation, TriggerConfig::DEFAULT.actuation);
        assert_eq!(fixed.release, fixed.actuation);
        assert_eq!(fixed.rt_press, TriggerConfig::DEFAULT_RT);
        assert_eq!(fixed.rt_release, mm(36));
        assert!(fixed.validate().is_ok());
        assert_eq!(normalized(rapid()), rapid());
    }

    #[test]
    fn recovery_keeps_keys_changed_since() {
        let mut table = table();
        let first = table.apply(
            [pos("w"), pos("a")],
            TriggerEdit::Set(TriggerField::Actuation, mm(15)),
            1,
        );
        table.apply(
            [pos("a")],
            TriggerEdit::Set(TriggerField::Actuation, mm(25)),
            2,
        );

        // The keyboard took the first write for W only.
        let w = TriggerConfig {
            actuation: mm(15),
            release: mm(15),
            ..TriggerConfig::DEFAULT
        };
        table.recover(
            first.revision,
            [(pos("w"), w), (pos("a"), TriggerConfig::DEFAULT)],
        );
        assert_eq!(table.get(pos("w")).unwrap().actuation, mm(15));
        assert_eq!(table.get(pos("a")).unwrap().actuation, mm(25));

        table.recover(first.revision, [(pos("w"), TriggerConfig::DEFAULT)]);
        assert_eq!(table.get(pos("w")).unwrap().actuation, mm(18));
    }

    #[test]
    fn recovery_ignores_writes_older_than_the_table() {
        let mut table = TriggerTable::new([(pos("w"), TriggerConfig::DEFAULT)], 5);
        table.recover(5, [(pos("w"), rapid())]);
        assert_eq!(*table.get(pos("w")).unwrap(), TriggerConfig::DEFAULT);
        table.recover(6, [(pos("w"), rapid())]);
        assert_eq!(*table.get(pos("w")).unwrap(), rapid());
    }

    #[test]
    fn restore_puts_back_previous_settings_as_a_new_change() {
        let mut table = table();
        let edit = table.apply(
            [pos("w"), pos("s")],
            TriggerEdit::Mode(TriggerMode::RapidSplit),
            1,
        );
        let undo = table.restore(&edit.previous, 2);
        assert_eq!(undo.revision, 2);
        assert_eq!(undo.writes.len(), 2);
        assert_eq!(*table.get(pos("w")).unwrap(), TriggerConfig::DEFAULT);
        assert_eq!(*table.get(pos("s")).unwrap(), rapid());
    }
}
