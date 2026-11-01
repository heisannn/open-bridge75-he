//! SOCD (simultaneous opposing cardinal directions) pairs (wire 0x24). Not configurable in the official configurator; based only on FW analysis.
//!
//! In byte 0 of each pair, bit0-2 is the [`SocdMode`] and bit7 is the enabled flag.
//!
//! - bit7 cannot be changed by a pair write (`b[2]=0`). The FW writes `existing & 0xF8 | sent & 7`.
//! - bit7 cannot be read by a pair query (`b[2]=1`) either. The FW returns byte 0 with `& 7` applied.
//! - bit7 is rewritten by the global on/off (`b[2]=3`), which sets all 20 pairs to the same value.
//! - bit7 can be read only through the enabled mask of the status query (`b[2]=2`) ([`SocdStatus::enabled_mask`]).
//!
//! On press, SOCD takes effect when mode is 2 (always), or when mode is 1 and bit7 is set.

use serde::{Deserialize, Serialize};

use crate::hid::Usage;
use crate::keymap::SOCD_PAIRS;
use crate::{Error, Result};

/// When SOCD takes effect (bit0-2 of byte 0).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SocdMode {
    /// Never.
    #[default]
    Off = 0,
    /// Only while the enabled flag (bit7) is set. The flag is toggled by the global on/off.
    Global = 1,
    /// Always.
    Always = 2,
}

impl SocdMode {
    /// 3 to 7 are not matched by the FW (never take effect), so they are read as [`SocdMode::Off`].
    fn from_raw(raw: u8) -> Self {
        match raw & 0x07 {
            1 => SocdMode::Global,
            2 => SocdMode::Always,
            _ => SocdMode::Off,
        }
    }

    /// Name in profiles.
    pub fn name(self) -> &'static str {
        match self {
            SocdMode::Off => "off",
            SocdMode::Global => "global",
            SocdMode::Always => "always",
        }
    }
}

/// Behavior when both keys are pressed.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SocdBehavior {
    /// Last input wins (releasing it presses the earlier key again).
    #[default]
    Last = 0,
    /// A wins.
    A = 1,
    /// B wins.
    B = 2,
    /// Pressing both releases both.
    Neutral = 3,
}

impl SocdBehavior {
    fn from_raw(raw: u8) -> Self {
        match raw {
            1 => SocdBehavior::A,
            2 => SocdBehavior::B,
            3 => SocdBehavior::Neutral,
            _ => SocdBehavior::Last,
        }
    }

    /// Name in profiles.
    pub fn name(self) -> &'static str {
        match self {
            SocdBehavior::Last => "last",
            SocdBehavior::A => "a",
            SocdBehavior::B => "b",
            SocdBehavior::Neutral => "neutral",
        }
    }
}

/// One pair (4 bytes).
///
/// Bit7 of byte 0 (the enabled flag) cannot be written and cannot be read by a pair query either, so it is not stored.
/// Read the current value with [`SocdStatus`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct SocdPair {
    pub mode: SocdMode,
    pub behavior: SocdBehavior,
    pub a: Usage,
    pub b: Usage,
}

impl SocdPair {
    pub fn is_empty(&self) -> bool {
        self.encode() == [0; 4]
    }

    /// Whether keys A / B are nonzero and different from each other.
    pub fn validate(&self) -> Result<()> {
        if self.a == Usage::NONE || self.b == Usage::NONE {
            return Err(Error::Invalid(
                "SOCD pair keys a / b must be nonzero".into(),
            ));
        }
        if self.a == self.b {
            return Err(Error::Invalid(format!(
                "SOCD pair keys a / b are the same ({})",
                self.a
            )));
        }
        Ok(())
    }

    /// Bit7 of byte 0 is sent as 0 (the FW ignores the sent bit7 and keeps the old value).
    pub fn encode(&self) -> [u8; 4] {
        [self.mode as u8, self.behavior as u8, self.a.0, self.b.0]
    }

    pub fn decode(bytes: [u8; 4]) -> Self {
        let [flags, behavior, a, b] = bytes;
        SocdPair {
            mode: SocdMode::from_raw(flags),
            behavior: SocdBehavior::from_raw(behavior),
            a: Usage(a),
            b: Usage(b),
        }
    }
}

/// All 20 pairs.
pub type SocdPairs = [SocdPair; SOCD_PAIRS as usize];

/// Result of the status query (`b[2]=2`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct SocdStatus {
    /// Global on/off (`b[6]`).
    pub global: bool,
    /// 24 bits (BE) of `b[7..=9]`. bit(19 - i) is the enabled flag of pair i (bit7 of byte 0).
    pub enabled_mask: u32,
}

impl SocdStatus {
    /// Enabled flag of the pair (bit7 of byte 0).
    pub fn is_enabled(&self, pair: u8) -> bool {
        pair < SOCD_PAIRS && self.enabled_mask & (1 << (19 - pair)) != 0
    }

    /// Whether SOCD currently takes effect for this pair.
    pub fn is_active(&self, pair: u8, mode: SocdMode) -> bool {
        match mode {
            SocdMode::Always => true,
            SocdMode::Global => self.is_enabled(pair),
            SocdMode::Off => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_round_trip() {
        let pair = SocdPair {
            mode: SocdMode::Always,
            behavior: SocdBehavior::Neutral,
            a: Usage(0x04),
            b: Usage(0x07),
        };
        assert_eq!(pair.encode(), [0x02, 3, 0x04, 0x07]);
        assert_eq!(SocdPair::decode(pair.encode()), pair);
        assert!(SocdPair::default().is_empty());
    }

    #[test]
    fn decode_ignores_enabled_bit() {
        // The query does not return bit7, but if it comes, it is not mixed into mode.
        let pair = SocdPair::decode([0x81, 0, 0x04, 0x07]);
        assert_eq!(pair.mode, SocdMode::Global);
        assert_eq!(pair.encode()[0], 0x01);
        // A mode the FW does not match never takes effect, so it is off.
        assert_eq!(SocdPair::decode([0x05, 0, 0x04, 0x07]).mode, SocdMode::Off);
    }

    #[test]
    fn validate_keys() {
        let pair = SocdPair {
            mode: SocdMode::Always,
            behavior: SocdBehavior::Last,
            a: Usage(0x04),
            b: Usage(0x07),
        };
        assert!(pair.validate().is_ok());
        assert!(SocdPair { b: pair.a, ..pair }.validate().is_err());
        assert!(
            SocdPair {
                a: Usage::NONE,
                ..pair
            }
            .validate()
            .is_err()
        );
    }

    #[test]
    fn status_mask() {
        let status = SocdStatus {
            global: true,
            enabled_mask: 1 << 19 | 1 << 0,
        };
        assert!(status.is_enabled(0));
        assert!(!status.is_enabled(1));
        assert!(status.is_enabled(19));
        assert!(!status.is_enabled(20));
        assert!(status.is_active(1, SocdMode::Always));
        assert!(!status.is_active(1, SocdMode::Global));
        assert!(status.is_active(0, SocdMode::Global));
        assert!(!status.is_active(0, SocdMode::Off));
    }
}
