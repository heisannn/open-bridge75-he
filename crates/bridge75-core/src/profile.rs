//! Profile stored on the PC (TOML).
//!
//! The keymap, macros, and advanced keys cannot be read from the keyboard, so the app keeps their contents.
//! Every item is optional. Omitted items are not written to the keyboard.
//! Unknown keys, and the same key written twice under different names, are errors.
//!
//! ```toml
//! debounce = 0
//! polling_rate = 8
//!
//! [sleep]
//! light = 60
//! keyboard = 1800
//!
//! # Later groups take precedence.
//! [[trigger]]
//! keys = ["all"]
//! mode = "rapid"
//! actuation = 1.6
//! rt_press = 0.8
//!
//! [keymap.base]   # keys not listed keep their defaults
//! caps = "adv:0"
//! [keymap.fn]
//! h = "left"
//! m = "macro:0"
//!
//! [[macro]]
//! slot = 0
//! events = [{ press = "a", delay = 20 }, { release = "a", delay = 20 }]
//!
//! [[advanced]]
//! slot = 0
//! type = "mod-tap"
//! hold = "lctrl"
//! tap = "esc"
//! hold_time = 200
//!
//! [socd]
//! global = true
//! pairs = [{ index = 0, mode = "always", behavior = "last", a = "a", b = "d" }]
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crate::advanced::{AdvancedKey, Dks, DksBinding, ModTap, Toggle};
use crate::he::{Debounce, PollingRate, Travel, TriggerConfig, TriggerMode};
use crate::hid::Usage;
use crate::info::SleepTimes;
use crate::keymap::{ADVANCED_SLOTS, KeyAction, KeyEntry, Keymap, MACRO_SLOTS, SOCD_PAIRS};
use crate::macros::{Macro, MacroEvent, RepeatMode};
use crate::matrix::{KeyBitmap, KeyPos, KeySel, LAYOUT};
use crate::socd::{SocdBehavior, SocdMode, SocdPair, SocdPairs};
use crate::{Error, Result};

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub debounce: Option<Debounce>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub polling_rate: Option<PollingRate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sleep: Option<SleepTimes>,
    /// Trigger settings. Later groups take precedence.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trigger: Vec<TriggerGroup>,
    /// If present, both layers are written (an empty table restores the default layout).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keymap: Option<KeymapProfile>,
    /// If present, all 10 slots are written (slots not listed are empty).
    #[serde(rename = "macro", skip_serializing_if = "Option::is_none")]
    pub macros: Option<Vec<MacroSlot>>,
    /// If present, all 40 slots are written (slots not listed are empty).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advanced: Option<Vec<AdvancedSlot>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub socd: Option<SocdProfile>,
}

/// A group of keys with the same trigger settings.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerGroup {
    pub keys: Vec<KeySel>,
    pub mode: TriggerMode,
    pub actuation: Travel,
    /// Release point / RT exit point. Defaults to the actuation point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<Travel>,
    /// RT press sensitivity. Defaults to 0.5mm.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rt_press: Option<Travel>,
    /// RT release sensitivity. Defaults to the RT press sensitivity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rt_release: Option<Travel>,
}

impl TriggerGroup {
    pub fn config(&self) -> TriggerConfig {
        let rt_press = self.rt_press.unwrap_or(TriggerConfig::DEFAULT_RT);
        TriggerConfig {
            mode: self.mode,
            actuation: self.actuation,
            release: self.release.unwrap_or(self.actuation),
            rt_press,
            rt_release: self.rt_release.unwrap_or(rt_press),
        }
    }

    /// Builds with optional values omitted.
    pub fn new(keys: Vec<KeySel>, config: &TriggerConfig) -> Self {
        TriggerGroup {
            keys,
            mode: config.mode,
            actuation: config.actuation,
            release: (config.release != config.actuation).then_some(config.release),
            rt_press: (config.rt_press != TriggerConfig::DEFAULT_RT).then_some(config.rt_press),
            rt_release: (config.rt_release != config.rt_press).then_some(config.rt_release),
        }
    }

    pub fn bitmap(&self) -> KeyBitmap {
        KeySel::to_bitmap(&self.keys)
    }

    /// Groups per-key settings into groups of identical settings. Groups with more keys come first.
    pub fn group(configs: &[(KeyPos, TriggerConfig)]) -> Vec<TriggerGroup> {
        let mut groups: Vec<(TriggerConfig, Vec<KeyPos>)> = Vec::new();
        for (pos, config) in configs {
            match groups.iter_mut().find(|(c, _)| c == config) {
                Some((_, keys)) => keys.push(*pos),
                None => groups.push((*config, vec![*pos])),
            }
        }
        groups.sort_by_key(|(_, keys)| std::cmp::Reverse(keys.len()));
        groups
            .into_iter()
            .map(|(config, keys)| {
                let keys = if keys.len() == LAYOUT.len() {
                    vec![KeySel::All]
                } else {
                    keys.into_iter().map(KeySel::Key).collect()
                };
                TriggerGroup::new(keys, &config)
            })
            .collect()
    }

    /// Same as [`Self::group`], but if the settings cover all keys, the largest group becomes `all`.
    /// Later groups take precedence, so the written result is the same.
    pub fn compact(configs: &[(KeyPos, TriggerConfig)]) -> Vec<TriggerGroup> {
        let mut groups = Self::group(configs);
        let covers_all = LAYOUT
            .iter()
            .all(|k| configs.iter().any(|(pos, _)| *pos == k.pos));
        if covers_all && let Some(first) = groups.first_mut() {
            first.keys = vec![KeySel::All];
        }
        groups
    }
}

/// Keymap changes. Keys not listed get the default assignment (the Fn layer also gets the default HID code).
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct KeymapProfile {
    #[serde(
        default,
        deserialize_with = "layer_changes",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub base: BTreeMap<KeyPos, KeyAction>,
    #[serde(
        default,
        rename = "fn",
        deserialize_with = "layer_changes",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub fn_layer: BTreeMap<KeyPos, KeyAction>,
}

/// Reads the changes for one layer. Writing the same key twice under different names (such as `caps` and `r3c0`) is an error.
fn layer_changes<'de, D: Deserializer<'de>>(
    d: D,
) -> std::result::Result<BTreeMap<KeyPos, KeyAction>, D::Error> {
    struct Changes;

    impl<'de> Visitor<'de> for Changes {
        type Value = BTreeMap<KeyPos, KeyAction>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a table of key names and actions")
        }

        fn visit_map<A: MapAccess<'de>>(
            self,
            mut map: A,
        ) -> std::result::Result<Self::Value, A::Error> {
            let mut changes = BTreeMap::new();
            while let Some(name) = map.next_key::<String>()? {
                let pos: KeyPos = name.parse().map_err(de::Error::custom)?;
                let action: KeyAction = map.next_value()?;
                if changes.insert(pos, action).is_some() {
                    return Err(de::Error::custom(format!(
                        "{name}: {pos} is already set (the same key is written twice under different names)"
                    )));
                }
            }
            Ok(changes)
        }
    }

    d.deserialize_map(Changes)
}

impl KeymapProfile {
    /// Builds the base layer and the Fn layer.
    pub fn build(&self) -> (Keymap, Keymap) {
        let layer = |changes: &BTreeMap<KeyPos, KeyAction>| {
            let mut map = Keymap::default_layer();
            for (pos, action) in changes {
                map.set(*pos, KeyEntry::from(*action));
            }
            map
        };
        (layer(&self.base), layer(&self.fn_layer))
    }
}

/// One `[[macro]]` entry. In TOML, `slot` and the macro contents (`repeat` / `count` / `events`) go in the same table.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(try_from = "MacroSlotToml", into = "MacroSlotToml")]
pub struct MacroSlot {
    pub slot: u8,
    pub r#macro: Macro,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MacroSlotToml {
    slot: u8,
    #[serde(default)]
    repeat: RepeatMode,
    /// Only allowed when `repeat = "count"`. Defaults to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    count: Option<u16>,
    events: Vec<MacroEvent>,
}

impl TryFrom<MacroSlotToml> for MacroSlot {
    type Error = Error;

    fn try_from(t: MacroSlotToml) -> Result<Self> {
        if t.count.is_some() && t.repeat != RepeatMode::Count {
            return Err(Error::Invalid(format!(
                "macro {}: count is only allowed when repeat = \"count\"",
                t.slot
            )));
        }
        Ok(MacroSlot {
            slot: t.slot,
            r#macro: Macro {
                repeat: t.repeat,
                count: t.count.unwrap_or(1),
                events: t.events,
            },
        })
    }
}

impl From<MacroSlot> for MacroSlotToml {
    fn from(m: MacroSlot) -> Self {
        let Macro {
            repeat,
            count,
            events,
        } = m.r#macro;
        MacroSlotToml {
            slot: m.slot,
            repeat,
            count: (repeat == RepeatMode::Count).then_some(count),
            events,
        }
    }
}

/// One `[[advanced]]` entry. In TOML, `slot` and the advanced key contents (`type`, etc.) go in the same table.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(from = "AdvancedSlotToml")]
pub struct AdvancedSlot {
    pub slot: u8,
    // Used only for serialization. Reading goes through `AdvancedSlotToml`, which rejects unknown keys per type.
    #[serde(flatten)]
    pub key: AdvancedKey,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum AdvancedSlotToml {
    Dks {
        slot: u8,
        points: [Travel; 4],
        bindings: Vec<DksBinding>,
    },
    ModTap {
        slot: u8,
        hold: KeyAction,
        tap: KeyAction,
        hold_time: u16,
    },
    Toggle {
        slot: u8,
        key: KeyAction,
    },
}

impl From<AdvancedSlotToml> for AdvancedSlot {
    fn from(t: AdvancedSlotToml) -> Self {
        let (slot, key) = match t {
            AdvancedSlotToml::Dks {
                slot,
                points,
                bindings,
            } => (slot, AdvancedKey::Dks(Dks { points, bindings })),
            AdvancedSlotToml::ModTap {
                slot,
                hold,
                tap,
                hold_time,
            } => (
                slot,
                AdvancedKey::ModTap(ModTap {
                    hold,
                    tap,
                    hold_time,
                }),
            ),
            AdvancedSlotToml::Toggle { slot, key } => (slot, AdvancedKey::Toggle(Toggle { key })),
        };
        AdvancedSlot { slot, key }
    }
}

#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SocdProfile {
    /// Global on/off. The enabled flags of all pairs are also set to this value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global: Option<bool>,
    /// Pairs not listed are empty.
    #[serde(default)]
    pub pairs: Vec<SocdSlot>,
}

/// One `pairs` entry. In TOML, `{ index = 0, mode = "always", behavior = "last", a = "a", b = "d" }`.
#[derive(Clone, PartialEq, Debug, Serialize, Deserialize)]
#[serde(from = "SocdSlotToml", into = "SocdSlotToml")]
pub struct SocdSlot {
    pub index: u8,
    pub pair: SocdPair,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SocdSlotToml {
    index: u8,
    /// `off` / `global` (only while globally on) / `always`.
    mode: SocdMode,
    behavior: SocdBehavior,
    a: Usage,
    b: Usage,
}

impl From<SocdSlotToml> for SocdSlot {
    fn from(t: SocdSlotToml) -> Self {
        SocdSlot {
            index: t.index,
            pair: SocdPair {
                mode: t.mode,
                behavior: t.behavior,
                a: t.a,
                b: t.b,
            },
        }
    }
}

impl From<SocdSlot> for SocdSlotToml {
    fn from(s: SocdSlot) -> Self {
        let SocdPair {
            mode,
            behavior,
            a,
            b,
        } = s.pair;
        SocdSlotToml {
            index: s.index,
            mode,
            behavior,
            a,
            b,
        }
    }
}

fn check_slots(what: &str, slots: impl Iterator<Item = u8>, max: u8) -> Result<()> {
    let mut seen = BTreeSet::new();
    for slot in slots {
        if slot >= max {
            return Err(Error::OutOfRange(format!(
                "{what} index must be 0 to {} (got {slot})",
                max - 1
            )));
        }
        if !seen.insert(slot) {
            return Err(Error::Invalid(format!("duplicate {what} {slot}")));
        }
    }
    Ok(())
}

/// The kind of slot referenced from the keymap and elsewhere.
struct SlotKind {
    prefix: &'static str,
    table: &'static str,
    /// What gets written when the referenced slot is not listed.
    when_missing: &'static str,
    /// What gets used when the table is missing.
    existing: &'static str,
}

const MACRO_KIND: SlotKind = SlotKind {
    prefix: "macro",
    table: "[[macro]]",
    when_missing: "it will be written as an empty macro (the fallback HID is sent, if any)",
    existing: "the macros currently on the keyboard",
};

const ADVANCED_KIND: SlotKind = SlotKind {
    prefix: "adv",
    table: "[[advanced]]",
    when_missing: "it will be written as an empty slot",
    existing: "the advanced keys currently on the keyboard",
};

impl Profile {
    pub fn from_toml(text: &str) -> Result<Self> {
        let profile: Profile = toml::from_str(text)
            .map_err(|e| Error::Invalid(format!("cannot read profile: {e}")))?;
        profile.validate()?;
        Ok(profile)
    }

    pub fn to_toml(&self) -> String {
        toml::to_string_pretty(self).expect("a profile can always be serialized to TOML")
    }

    /// Checks all values before writing.
    pub fn validate(&self) -> Result<()> {
        if let Some(sleep) = &self.sleep {
            sleep.validate()?;
        }
        for group in &self.trigger {
            if group.keys.is_empty() {
                return Err(Error::Invalid("trigger keys is empty".into()));
            }
            group.config().validate()?;
        }
        if let Some(macros) = &self.macros {
            check_slots("macro", macros.iter().map(|m| m.slot), MACRO_SLOTS)?;
            for m in macros {
                m.r#macro
                    .validate()
                    .map_err(|e| Error::Invalid(format!("macro {}: {e}", m.slot)))?;
            }
        }
        if let Some(advanced) = &self.advanced {
            check_slots(
                "advanced key",
                advanced.iter().map(|a| a.slot),
                ADVANCED_SLOTS,
            )?;
            for a in advanced {
                a.key
                    .validate()
                    .map_err(|e| Error::Invalid(format!("advanced key {}: {e}", a.slot)))?;
            }
        }
        if let Some(socd) = &self.socd {
            check_slots("SOCD pair", socd.pairs.iter().map(|p| p.index), SOCD_PAIRS)?;
            for p in &socd.pairs {
                p.pair
                    .validate()
                    .map_err(|e| Error::Invalid(format!("SOCD pair {}: {e}", p.index)))?;
            }
        }
        Ok(())
    }

    /// Points that might cause trouble when written. These are not errors.
    pub fn warnings(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        if let Some(keymap) = &self.keymap {
            let (base, _) = keymap.build();
            if !base.0.contains(&KeyEntry::FN) {
                warnings
                    .push("the base layer has no fn key; the Fn layer will be unreachable".into());
            }
            for (pos, action) in &keymap.fn_layer {
                if *action == KeyAction::None {
                    warnings.push(format!(
                        "{pos} on the Fn layer is none; this key sends nothing while Fn is held"
                    ));
                }
            }
        }
        let macros = self
            .macros
            .as_ref()
            .map(|m| m.iter().map(|s| s.slot).collect());
        let advanced = self
            .advanced
            .as_ref()
            .map(|a| a.iter().map(|s| s.slot).collect());
        let refs = self.slot_references();
        warnings.extend(slot_warnings(&MACRO_KIND, &refs.0, macros.as_ref()));
        warnings.extend(slot_warnings(&ADVANCED_KIND, &refs.1, advanced.as_ref()));
        warnings
    }

    /// Where `macro:N` and `adv:N` are used (keymap and advanced keys), per slot number.
    fn slot_references(&self) -> (BTreeMap<u8, Vec<String>>, BTreeMap<u8, Vec<String>>) {
        let mut macros: BTreeMap<u8, Vec<String>> = BTreeMap::new();
        let mut advanced: BTreeMap<u8, Vec<String>> = BTreeMap::new();
        let mut note = |place: String, action: KeyAction| match action {
            KeyAction::Macro { index, .. } => macros.entry(index).or_default().push(place),
            KeyAction::Advanced(slot) => advanced.entry(slot).or_default().push(place),
            _ => {}
        };
        if let Some(keymap) = &self.keymap {
            for (pos, action) in &keymap.base {
                note(format!("base layer {pos}"), *action);
            }
            for (pos, action) in &keymap.fn_layer {
                note(format!("Fn layer {pos}"), *action);
            }
        }
        for a in self.advanced.iter().flatten() {
            for action in a.key.actions() {
                note(format!("advanced key {}", a.slot), action);
            }
        }
        (macros, advanced)
    }

    /// Warns if fixed-function keys (the 0x0E mask) are assigned on the Fn layer. The FW silently drops those assignments.
    pub fn fixed_fn_warnings(&self, mask: &KeyBitmap) -> Vec<String> {
        let Some(keymap) = &self.keymap else {
            return Vec::new();
        };
        let fixed: Vec<String> = keymap
            .fn_layer
            .keys()
            .filter(|pos| mask.contains(**pos))
            .map(|pos| pos.to_string())
            .collect();
        if fixed.is_empty() {
            return Vec::new();
        }
        vec![format!(
            "{} on the Fn layer are fixed-function keys; the keyboard ignores these assignments",
            fixed.join(" ")
        )]
    }

    pub fn macro_slots(&self) -> Option<[Option<Macro>; MACRO_SLOTS as usize]> {
        let macros = self.macros.as_ref()?;
        let mut slots: [Option<Macro>; MACRO_SLOTS as usize] = Default::default();
        for m in macros {
            slots[m.slot as usize] = Some(m.r#macro.clone());
        }
        Some(slots)
    }

    pub fn advanced_slots(&self) -> Option<[Option<AdvancedKey>; ADVANCED_SLOTS as usize]> {
        let advanced = self.advanced.as_ref()?;
        let mut slots: [Option<AdvancedKey>; ADVANCED_SLOTS as usize] =
            std::array::from_fn(|_| None);
        for a in advanced {
            slots[a.slot as usize] = Some(a.key.clone());
        }
        Some(slots)
    }
}

/// Warning for when a referenced slot is not written (the table is missing, or the slot is not listed).
fn slot_warnings(
    kind: &SlotKind,
    refs: &BTreeMap<u8, Vec<String>>,
    defined: Option<&BTreeSet<u8>>,
) -> Vec<String> {
    if refs.is_empty() {
        return Vec::new();
    }
    let Some(defined) = defined else {
        let used: Vec<String> = refs
            .keys()
            .map(|slot| format!("{}:{slot}", kind.prefix))
            .collect();
        return vec![format!(
            "{} is used, but there is no {}; {} will be used as is",
            used.join(", "),
            kind.table,
            kind.existing
        )];
    };
    refs.iter()
        .filter(|(slot, _)| !defined.contains(slot))
        .map(|(slot, places)| {
            format!(
                "{}:{slot} is used ({}), but {} has no slot {slot}; {}",
                kind.prefix,
                places.join(", "),
                kind.table,
                kind.when_missing
            )
        })
        .collect()
}

impl SocdProfile {
    pub fn pairs(&self) -> SocdPairs {
        let mut pairs = SocdPairs::default();
        for slot in &self.pairs {
            pairs[slot.index as usize] = slot.pair;
        }
        pairs
    }

    /// Builds a profile containing only the non-empty pairs.
    ///
    /// Pairs that have no effect (mode off) and also have invalid keys cannot be written back, so they are left out.
    pub fn from_pairs(pairs: &SocdPairs, global: Option<bool>) -> Self {
        let pairs = pairs
            .iter()
            .enumerate()
            .filter(|(_, p)| !p.is_empty())
            .filter(|(_, p)| p.mode != SocdMode::Off || p.validate().is_ok())
            .map(|(i, p)| SocdSlot {
                index: i as u8,
                pair: *p,
            })
            .collect();
        SocdProfile { global, pairs }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::advanced::DksAction;
    use crate::macros::{MacroAction, MacroTarget};

    const SAMPLE: &str = r#"
debounce = 0
polling_rate = 8

[sleep]
light = 60
keyboard = 1800

[[trigger]]
keys = ["all"]
mode = "rapid"
actuation = 1.6
rt_press = 0.8

[[trigger]]
keys = ["a", "x", "quote", "dot"]
mode = "rapid"
actuation = 1.2
rt_press = 0.4

[keymap.base]
caps = "lctrl"
lctrl = "caps"

[keymap.fn]
h = "left"
r3c8 = "down"

[[macro]]
slot = 0
repeat = "count"
count = 2
events = [{ press = "a", delay = 20 }, { release = "a", delay = 20 }, { press = "mouse:left", delay = 8 }]

[[macro]]
slot = 1
events = []

[[advanced]]
slot = 0
type = "mod-tap"
hold = "lctrl"
tap = "esc"
hold_time = 200

[[advanced]]
slot = 1
type = "dks"
points = [1.0, 3.4, 3.0, 0.5]
bindings = [{ key = "a", stages = ["tap", "hold", "hold", "release"] }]

[[advanced]]
slot = 2
type = "toggle"
key = "lshift"

[socd]
global = true
pairs = [
    { index = 0, mode = "always", behavior = "last", a = "a", b = "d" },
    { index = 3, mode = "global", behavior = "neutral", a = "w", b = "s" },
]
"#;

    #[test]
    fn parse_sample() {
        let p = Profile::from_toml(SAMPLE).unwrap();
        assert_eq!(p.debounce, Some(Debounce::new(0).unwrap()));
        assert_eq!(p.trigger.len(), 2);
        assert_eq!(p.trigger[1].config().rt_release, Travel::from_raw(4));
        assert_eq!(p.trigger[1].bitmap().len(), 4);

        let (base, fn_layer) = p.keymap.as_ref().unwrap().build();
        assert_eq!(base.get("caps".parse().unwrap()), KeyEntry::key(0xE0));
        assert_eq!(fn_layer.get("j".parse().unwrap()), KeyEntry::key(0x51));
        assert_eq!(fn_layer.get("k".parse().unwrap()), KeyEntry::key(0x0E));

        let macros = p.macro_slots().unwrap();
        let m = macros[0].as_ref().unwrap();
        assert_eq!(m.repeat, RepeatMode::Count);
        assert_eq!(m.count, 2);
        assert_eq!(
            m.events[2].action,
            MacroAction::Press(MacroTarget::Mouse(1))
        );
        assert!(macros[1].as_ref().unwrap().events.is_empty());
        assert!(macros[2].is_none());

        let advanced = p.advanced_slots().unwrap();
        assert_eq!(
            advanced[1],
            Some(AdvancedKey::Dks(Dks {
                points: [10, 34, 30, 5].map(Travel::from_raw),
                bindings: vec![DksBinding {
                    key: KeyAction::Key(Usage(0x04)),
                    stages: [
                        DksAction::Tap,
                        DksAction::Hold,
                        DksAction::Hold,
                        DksAction::Release
                    ],
                }],
            }))
        );
        assert_eq!(
            advanced[2],
            Some(AdvancedKey::Toggle(Toggle {
                key: KeyAction::Key(Usage(0xE1))
            }))
        );
        let pairs = p.socd.as_ref().unwrap().pairs();
        assert_eq!(pairs[0].encode(), [0x02, 0, 0x04, 0x07]);
        assert_eq!(pairs[3].encode(), [0x01, 3, 0x1A, 0x16]);
        assert!(pairs[1].is_empty());
        assert!(p.warnings().is_empty(), "{:?}", p.warnings());
    }

    #[test]
    fn toml_round_trip() {
        let p = Profile::from_toml(SAMPLE).unwrap();
        let text = p.to_toml();
        assert_eq!(Profile::from_toml(&text).unwrap(), p, "{text}");
        // Serialization also puts slot first.
        assert!(
            text.contains("[[advanced]]\nslot = 0\ntype = \"mod-tap\""),
            "{text}"
        );
        // count is serialized only when repeat = "count".
        assert_eq!(text.matches("count = ").count(), 1, "{text}");
    }

    #[test]
    fn doc_example_parses() {
        let example: String = include_str!("profile.rs")
            .lines()
            .skip_while(|l| *l != "//! ```toml")
            .skip(1)
            .take_while(|l| *l != "//! ```")
            .map(|l| {
                let l = l.strip_prefix("//!").unwrap();
                format!("{}\n", l.strip_prefix(' ').unwrap_or(l))
            })
            .collect();
        assert!(example.contains("[[advanced]]"), "{example}");
        let p = Profile::from_toml(&example).unwrap();
        assert!(p.warnings().is_empty(), "{:?}", p.warnings());
        assert_eq!(Profile::from_toml(&p.to_toml()).unwrap(), p);
    }

    #[test]
    fn rejects_bad_profiles() {
        for text in [
            "debounce = 252",
            "polling_rate = 3",
            "unknown = 1",
            "[[trigger]]\nkeys = []\nmode = \"rapid\"\nactuation = 1.0",
            "[[trigger]]\nkeys = [\"nokey\"]\nmode = \"rapid\"\nactuation = 1.0",
            "[[trigger]]\nkeys = [\"a\"]\nmode = \"rapid\"\nactuation = 4.0",
            "[[trigger]]\nkeys = [\"a\"]\nmode = \"rapid\"\nactuation = 1.0\nrt = 0.3",
            "[keymap.base]\na = \"macro:10\"",
            "[keymap.base]\na = \"raw:10040a00\"",
            "[keymap.bse]\na = \"b\"",
            "[[macro]]\nslot = 10\nevents = []",
            "[[macro]]\nslot = 0\nevents = []\n[[macro]]\nslot = 0\nevents = []",
            "[sleep]\nlight = 60\nkeyboard = 100",
            "[sleep]\nlight = 0\nkeyboard = 1800",
            "[sleep]\nlight = 60\nkeyboard = 1800\nlihgt = 30",
        ] {
            assert!(Profile::from_toml(text).is_err(), "{text}");
        }
    }

    #[test]
    fn rejects_unknown_keys() {
        // Formats found in review that were silently ignored.
        for text in [
            "[[macro]]\nslot = 0\nrepaet = \"hold\"\nevents = []",
            "[[macro]]\nslot = 0\nrepeat = \"count\"\ncuont = 3\nevents = []",
            "[[macro]]\nslot = 0\nevents = [{ press = \"a\", dealy = 20 }]",
            "[[macro]]\nslot = 0\nevents = [{ press = \"a\", release = \"b\", delay = 4 }]",
            "[[macro]]\nslot = 0\nevents = [{ delay = 4 }]",
            "[[macro]]\nslot = 0\ncount = 3\nevents = []",
            "[[macro]]\nslot = 0\nrepeat = \"hold\"\ncount = 3\nevents = []",
            "[[advanced]]\nslot = 0\ntype = \"toggle\"\nkey = \"a\"\nhold_time = 200",
            "[[advanced]]\nslot = 0\ntype = \"mod-tap\"\nhold = \"a\"\ntap = \"b\"\nhold_time = 200\nhold_tme = 100",
            "[[advanced]]\nslot = 0\ntype = \"mod-tap\"\nhold = \"a\"\ntap = \"b\"",
            "[[advanced]]\nslot = 0\nkey = \"a\"",
            "[[advanced]]\nslot = 0\ntype = \"dks\"\npoints = [1.0, 2.0, 2.0, 1.0]\nbindings = [{ key = \"a\", stages = [\"tap\", \"tap\", \"tap\", \"tap\"], stage = 1 }]",
            "[socd]\nglobal = true\nglobl = false",
            "[socd]\npairs = [{ index = 0, enabled = true, mode = \"always\", behavior = \"last\", a = \"a\", b = \"d\" }]",
            "[keymap]\nbase = {}\nfn = {}\nextra = {}",
        ] {
            assert!(Profile::from_toml(text).is_err(), "{text}");
        }
    }

    #[test]
    fn rejects_same_key_twice() {
        assert!(Profile::from_toml("[keymap.base]\ncaps = \"a\"\nr3c0 = \"b\"").is_err());
        assert!(Profile::from_toml("[keymap.fn]\nesc = \"a\"\nR0C0 = \"b\"").is_err());
        assert!(Profile::from_toml("[keymap.fn]\nr0c1 = \"a\"").is_err());
        // Different layers are fine.
        assert!(
            Profile::from_toml("[keymap.base]\ncaps = \"a\"\n[keymap.fn]\nr3c0 = \"b\"").is_ok()
        );
    }

    #[test]
    fn rejects_bad_socd() {
        let pair = |body: &str| format!("[socd]\npairs = [{{ index = 0, {body} }}]");
        for body in [
            "mode = 3, behavior = \"last\", a = \"a\", b = \"d\"",
            "mode = 2, behavior = \"last\", a = \"a\", b = \"d\"",
            "mode = \"sometimes\", behavior = \"last\", a = \"a\", b = \"d\"",
            "mode = \"always\", behavior = \"last\", a = \"a\", b = \"a\"",
            "mode = \"always\", behavior = \"last\", a = \"0x00\", b = \"d\"",
            "mode = \"off\", behavior = \"last\", a = \"a\", b = \"0x00\"",
            "mode = \"always\", behavior = \"first\", a = \"a\", b = \"d\"",
            "mode = \"always\", a = \"a\", b = \"d\"",
        ] {
            assert!(Profile::from_toml(&pair(body)).is_err(), "{body}");
        }
        assert!(
            Profile::from_toml(&pair(
                "mode = \"off\", behavior = \"b\", a = \"left\", b = \"right\""
            ))
            .is_ok()
        );
    }

    #[test]
    fn socd_dump_round_trip() {
        let mut pairs = SocdPairs::default();
        pairs[0] = SocdPair::decode([0x02, 0, 0x04, 0x07]);
        pairs[5] = SocdPair::decode([0x01, 3, 0x1A, 0x16]);
        // Pairs that the FW does not match and that would fail when written back are left out.
        pairs[7] = SocdPair::decode([0x07, 0xFF, 0xFF, 0xFF]);
        let socd = SocdProfile::from_pairs(&pairs, Some(true));
        assert_eq!(socd.pairs.len(), 2);
        let profile = Profile {
            socd: Some(socd),
            ..Profile::default()
        };
        let text = profile.to_toml();
        assert!(text.contains("mode = \"global\""), "{text}");
        let back = Profile::from_toml(&text).unwrap();
        let written = back.socd.as_ref().unwrap().pairs();
        assert_eq!(written[0].encode(), [0x02, 0, 0x04, 0x07]);
        assert_eq!(written[5].encode(), [0x01, 3, 0x1A, 0x16]);
        assert!(written[7].is_empty());
    }

    #[test]
    fn group_triggers() {
        let rapid = TriggerConfig {
            mode: TriggerMode::Rapid,
            ..TriggerConfig::DEFAULT
        };
        let mut configs: Vec<_> = LAYOUT.iter().map(|k| (k.pos, rapid)).collect();
        assert_eq!(TriggerGroup::group(&configs)[0].keys, [KeySel::All]);

        configs[0].1 = TriggerConfig::DEFAULT;
        let groups = TriggerGroup::group(&configs);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].keys.len(), 80);
        assert_eq!(groups[1].keys, [KeySel::Key("esc".parse().unwrap())]);
        assert_eq!(groups[1].config(), TriggerConfig::DEFAULT);

        let compact = TriggerGroup::compact(&configs);
        assert_eq!(compact[0].keys, [KeySel::All]);
        assert_eq!(compact[1], groups[1]);
        assert_eq!(TriggerGroup::compact(&configs[..10])[0].keys.len(), 9);
    }

    #[test]
    fn warns_about_fn() {
        let p =
            Profile::from_toml("[keymap.base]\nfn = \"ralt\"\n[keymap.fn]\na = \"none\"").unwrap();
        assert_eq!(p.warnings().len(), 2);
    }

    #[test]
    fn warns_about_missing_slots() {
        let keymap =
            "[keymap.base]\ncaps = \"macro:2:esc\"\nj = \"adv:1\"\n[keymap.fn]\nh = \"macro:2\"\n";
        // No table: the current contents of the keyboard are used.
        let p = Profile::from_toml(keymap).unwrap();
        let w = p.warnings();
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w[0].contains("macro:2") && w[0].contains("there is no [[macro]]"));
        assert!(w[1].contains("adv:1") && w[1].contains("there is no [[advanced]]"));

        // The table exists but the slot does not: an empty slot is written.
        let text = format!(
            "{keymap}[[macro]]\nslot = 0\nevents = []\n\
             [[advanced]]\nslot = 0\ntype = \"toggle\"\nkey = \"macro:3\"\n"
        );
        let w = Profile::from_toml(&text).unwrap().warnings();
        assert_eq!(w.len(), 3, "{w:?}");
        assert!(w[0].contains("macro:2") && w[0].contains("base layer caps, Fn layer h"));
        assert!(w[1].contains("macro:3") && w[1].contains("advanced key 0"));
        assert!(w[2].contains("adv:1") && w[2].contains("has no slot 1"));

        // No warning when the slot is listed.
        let text = format!(
            "{keymap}[[macro]]\nslot = 2\nevents = []\n\
             [[advanced]]\nslot = 1\ntype = \"toggle\"\nkey = \"a\"\n"
        );
        assert!(Profile::from_toml(&text).unwrap().warnings().is_empty());
    }

    #[test]
    fn warns_about_fixed_fn_keys() {
        let p = Profile::from_toml("[keymap.fn]\nh = \"left\"\nf1 = \"a\"\nf2 = \"b\"").unwrap();
        let mask: KeyBitmap = ["f1", "f2", "f3"]
            .iter()
            .map(|k| k.parse::<KeyPos>().unwrap())
            .collect();
        let w = p.fixed_fn_warnings(&mask);
        assert_eq!(w.len(), 1);
        assert!(w[0].contains("f1 f2"), "{w:?}");
        assert!(p.fixed_fn_warnings(&KeyBitmap::EMPTY).is_empty());
        assert!(Profile::default().fixed_fn_warnings(&mask).is_empty());
    }
}
