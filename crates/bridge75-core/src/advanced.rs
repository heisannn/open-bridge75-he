//! Advanced keys (wire 0x22). 40 slots x 40 bytes. Referenced from the keymap as `adv:<slot>`.

use serde::{Deserialize, Serialize};

use crate::he::Travel;
use crate::keymap::{KeyAction, KeyEntry};
use crate::{Error, Result};

/// Bytes per slot.
pub const SLOT_LEN: usize = 40;
/// Number of keys that can be assigned to a DKS.
pub const DKS_BINDINGS: usize = 4;
/// Minimum MT hold time (ms).
pub const MIN_HOLD_MS: u16 = 10;
/// Maximum MT hold time (ms). The FW counter stops at 250 (2500ms).
pub const MAX_HOLD_MS: u16 = 2500;

/// Advanced key. Profiles distinguish them with `type = "dks" | "mod-tap" | "toggle"`.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum AdvancedKey {
    Dks(Dks),
    ModTap(ModTap),
    Toggle(Toggle),
}

/// DKS (Dynamic Keystroke). For each of the 4 stages, sets what each key does.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Dks {
    /// Stage 1 press point, stage 2 press point (bottom side), stage 3 return point, stage 4 return point.
    pub points: [Travel; 4],
    /// Up to 4 keys.
    pub bindings: Vec<DksBinding>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DksBinding {
    pub key: KeyAction,
    /// Action in stages 1 to 4.
    pub stages: [DksAction; 4],
}

/// Action in each DKS stage (partly inferred).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum DksAction {
    #[default]
    Release = 0,
    /// Release -> press -> release.
    Tap = 1,
    /// Stay pressed.
    Hold = 2,
    /// Release -> press.
    Repress = 3,
}

impl DksAction {
    fn from_raw(raw: u8) -> Self {
        match raw {
            1 => DksAction::Tap,
            2 => DksAction::Hold,
            3 => DksAction::Repress,
            _ => DksAction::Release,
        }
    }
}

/// MT (hold / tap). Press / release are judged using the key's HE actuation point and release point.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModTap {
    pub hold: KeyAction,
    pub tap: KeyAction,
    /// Time until a hold is recognized (ms). 10 to 2500ms, in 10ms steps.
    ///
    /// Stored at slot offset 9 as ms / 10. While pressed, the FW increments a counter every 10ms (stopping at 250),
    /// and right after incrementing, it becomes a hold if the counter equals the value at offset 9. 0 and 251 or more never become a hold.
    pub hold_time: u16,
}

/// TGL (toggle). Toggles between press / release each time the actuation point is crossed.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Toggle {
    pub key: KeyAction,
}

fn put_entry(slot: &mut [u8; SLOT_LEN], at: usize, action: KeyAction) {
    slot[at..at + 4].copy_from_slice(&KeyEntry::from(action).0);
}

fn entry_at(slot: &[u8; SLOT_LEN], at: usize) -> KeyAction {
    KeyEntry(slot[at..at + 4].try_into().unwrap()).into()
}

impl AdvancedKey {
    pub fn validate(&self) -> Result<()> {
        match self {
            AdvancedKey::Dks(dks) => {
                if dks.bindings.len() > DKS_BINDINGS {
                    return Err(Error::OutOfRange(format!(
                        "a DKS can have at most {DKS_BINDINGS} keys"
                    )));
                }
                for p in dks.points {
                    Travel::new(p.raw())?;
                }
            }
            AdvancedKey::ModTap(mt) => {
                if mt.hold_time % 10 != 0 || !(MIN_HOLD_MS..=MAX_HOLD_MS).contains(&mt.hold_time) {
                    return Err(Error::OutOfRange(format!(
                        "hold time must be {MIN_HOLD_MS} to {MAX_HOLD_MS}ms in 10ms steps (got {}ms)",
                        mt.hold_time
                    )));
                }
            }
            AdvancedKey::Toggle(_) => {}
        }
        Ok(())
    }

    pub fn encode(&self) -> Result<[u8; SLOT_LEN]> {
        self.validate()?;
        let mut slot = [0; SLOT_LEN];
        match self {
            AdvancedKey::Dks(dks) => {
                slot[0] = 1;
                for (i, p) in dks.points.iter().enumerate() {
                    slot[1 + i] = p.raw();
                }
                for (i, binding) in dks.bindings.iter().enumerate() {
                    put_entry(&mut slot, 5 + 8 * i, binding.key);
                    for (stage, action) in binding.stages.iter().enumerate() {
                        slot[9 + 8 * i + stage] = *action as u8;
                    }
                }
            }
            AdvancedKey::ModTap(mt) => {
                slot[0] = 2;
                put_entry(&mut slot, 1, mt.hold);
                put_entry(&mut slot, 5, mt.tap);
                slot[9] = (mt.hold_time / 10) as u8;
            }
            AdvancedKey::Toggle(tgl) => {
                slot[0] = 3;
                put_entry(&mut slot, 1, tgl.key);
            }
        }
        Ok(slot)
    }

    /// Actions of the keys used in the assignment (to check references such as `macro:N`).
    pub fn actions(&self) -> Vec<KeyAction> {
        match self {
            AdvancedKey::Dks(dks) => dks.bindings.iter().map(|b| b.key).collect(),
            AdvancedKey::ModTap(mt) => vec![mt.hold, mt.tap],
            AdvancedKey::Toggle(tgl) => vec![tgl.key],
        }
    }

    /// Reads a 40-byte slot. `None` if empty or of an unknown type.
    pub fn decode(slot: &[u8; SLOT_LEN]) -> Option<Self> {
        match slot[0] {
            1 => {
                let points = [1, 2, 3, 4].map(|i| Travel::from_raw(slot[i]));
                let bindings = (0..DKS_BINDINGS)
                    .map(|i| DksBinding {
                        key: entry_at(slot, 5 + 8 * i),
                        stages: [0, 1, 2, 3].map(|s| DksAction::from_raw(slot[9 + 8 * i + s])),
                    })
                    .filter(|b| b.key != KeyAction::None)
                    .collect();
                Some(AdvancedKey::Dks(Dks { points, bindings }))
            }
            2 => Some(AdvancedKey::ModTap(ModTap {
                hold: entry_at(slot, 1),
                tap: entry_at(slot, 5),
                hold_time: slot[9] as u16 * 10,
            })),
            3 => Some(AdvancedKey::Toggle(Toggle {
                key: entry_at(slot, 1),
            })),
            _ => None,
        }
    }
}

/// Unused slot.
pub const EMPTY_SLOT: [u8; SLOT_LEN] = [0; SLOT_LEN];

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &str) -> KeyAction {
        s.parse().unwrap()
    }

    #[test]
    fn dks_layout() {
        let dks = AdvancedKey::Dks(Dks {
            points: [10, 34, 30, 5].map(Travel::from_raw),
            bindings: vec![
                DksBinding {
                    key: a("a"),
                    stages: [
                        DksAction::Tap,
                        DksAction::Hold,
                        DksAction::Hold,
                        DksAction::Release,
                    ],
                },
                DksBinding {
                    key: a("lshift"),
                    stages: [
                        DksAction::Release,
                        DksAction::Repress,
                        DksAction::Release,
                        DksAction::Release,
                    ],
                },
            ],
        });
        let slot = dks.encode().unwrap();
        assert_eq!(&slot[..5], &[1, 10, 34, 30, 5]);
        assert_eq!(&slot[5..13], &[0, 0x04, 0, 0, 1, 2, 2, 0]);
        assert_eq!(&slot[13..21], &[0, 0xE1, 0, 0, 0, 3, 0, 0]);
        assert!(slot[21..].iter().all(|&b| b == 0));
        assert_eq!(AdvancedKey::decode(&slot), Some(dks));
    }

    #[test]
    fn mod_tap_and_toggle() {
        let mt = AdvancedKey::ModTap(ModTap {
            hold: a("lctrl"),
            tap: a("esc"),
            hold_time: 200,
        });
        let slot = mt.encode().unwrap();
        assert_eq!(&slot[..10], &[2, 0, 0xE0, 0, 0, 0, 0x29, 0, 0, 20]);
        assert_eq!(AdvancedKey::decode(&slot), Some(mt));

        let tgl = AdvancedKey::Toggle(Toggle { key: a("lshift") });
        assert_eq!(&tgl.encode().unwrap()[..5], &[3, 0, 0xE1, 0, 0]);

        assert_eq!(AdvancedKey::decode(&EMPTY_SLOT), None);
    }

    #[test]
    fn hold_time_range() {
        let encode = |hold_time| {
            AdvancedKey::ModTap(ModTap {
                hold: a("a"),
                tap: a("b"),
                hold_time,
            })
            .encode()
        };
        assert_eq!(encode(10).unwrap()[9], 1);
        assert_eq!(encode(2500).unwrap()[9], 250);
        for bad in [0, 5, 205, 2510, 2550, 2560] {
            assert!(encode(bad).is_err(), "{bad}");
        }
    }
}
