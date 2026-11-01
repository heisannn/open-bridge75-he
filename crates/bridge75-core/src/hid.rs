//! Usages of the HID Keyboard/Keypad page (0x07) and their names.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// HID Keyboard usage (including the modifiers 0xE0-0xE7).
///
/// As a string it is a name (`a`, `enter`, `lctrl`, etc.) or hex (`0x2c`).
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize,
)]
#[serde(try_from = "String", into = "String")]
pub struct Usage(pub u8);

impl Usage {
    pub const NONE: Usage = Usage(0);

    /// Canonical name. `None` if not in the table.
    pub fn name(self) -> Option<&'static str> {
        NAMES.iter().find(|(u, _)| *u == self.0).map(|(_, n)| *n)
    }

    /// All usages that have a name.
    pub fn all() -> impl Iterator<Item = (Usage, &'static str)> {
        NAMES.iter().map(|&(u, n)| (Usage(u), n))
    }
}

impl fmt::Display for Usage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => f.write_str(name),
            None => write!(f, "0x{:02x}", self.0),
        }
    }
}

impl FromStr for Usage {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let lower = s.trim().to_ascii_lowercase();
        if let Some(hex) = lower.strip_prefix("0x") {
            return match u8::from_str_radix(hex, 16) {
                Ok(v) => Ok(Usage(v)),
                Err(_) => Err(Error::UnknownUsage(s.to_string())),
            };
        }
        NAMES
            .iter()
            .find(|(_, n)| *n == lower)
            .map(|&(u, _)| u)
            .or_else(|| ALIASES.iter().find(|(n, _)| *n == lower).map(|&(_, u)| u))
            .map(Usage)
            .ok_or_else(|| Error::UnknownUsage(s.to_string()))
    }
}

impl TryFrom<String> for Usage {
    type Error = Error;

    fn try_from(s: String) -> Result<Self> {
        s.parse()
    }
}

impl From<Usage> for String {
    fn from(u: Usage) -> String {
        u.to_string()
    }
}

/// Whether this is a modifier.
pub fn is_modifier(u: Usage) -> bool {
    (0xE0..=0xE7).contains(&u.0)
}

/// The names match the key matrix IDs ([`crate::matrix::Key::id`]).
const NAMES: &[(u8, &str)] = &[
    (0x04, "a"),
    (0x05, "b"),
    (0x06, "c"),
    (0x07, "d"),
    (0x08, "e"),
    (0x09, "f"),
    (0x0A, "g"),
    (0x0B, "h"),
    (0x0C, "i"),
    (0x0D, "j"),
    (0x0E, "k"),
    (0x0F, "l"),
    (0x10, "m"),
    (0x11, "n"),
    (0x12, "o"),
    (0x13, "p"),
    (0x14, "q"),
    (0x15, "r"),
    (0x16, "s"),
    (0x17, "t"),
    (0x18, "u"),
    (0x19, "v"),
    (0x1A, "w"),
    (0x1B, "x"),
    (0x1C, "y"),
    (0x1D, "z"),
    (0x1E, "1"),
    (0x1F, "2"),
    (0x20, "3"),
    (0x21, "4"),
    (0x22, "5"),
    (0x23, "6"),
    (0x24, "7"),
    (0x25, "8"),
    (0x26, "9"),
    (0x27, "0"),
    (0x28, "enter"),
    (0x29, "esc"),
    (0x2A, "backspace"),
    (0x2B, "tab"),
    (0x2C, "space"),
    (0x2D, "minus"),
    (0x2E, "equal"),
    (0x2F, "lbracket"),
    (0x30, "rbracket"),
    (0x31, "backslash"),
    (0x32, "nonus-hash"),
    (0x33, "semicolon"),
    (0x34, "quote"),
    (0x35, "grave"),
    (0x36, "comma"),
    (0x37, "dot"),
    (0x38, "slash"),
    (0x39, "caps"),
    (0x3A, "f1"),
    (0x3B, "f2"),
    (0x3C, "f3"),
    (0x3D, "f4"),
    (0x3E, "f5"),
    (0x3F, "f6"),
    (0x40, "f7"),
    (0x41, "f8"),
    (0x42, "f9"),
    (0x43, "f10"),
    (0x44, "f11"),
    (0x45, "f12"),
    (0x46, "printscreen"),
    (0x47, "scrolllock"),
    (0x48, "pause"),
    (0x49, "insert"),
    (0x4A, "home"),
    (0x4B, "pgup"),
    (0x4C, "del"),
    (0x4D, "end"),
    (0x4E, "pgdn"),
    (0x4F, "right"),
    (0x50, "left"),
    (0x51, "down"),
    (0x52, "up"),
    (0x53, "numlock"),
    (0x54, "kp-slash"),
    (0x55, "kp-asterisk"),
    (0x56, "kp-minus"),
    (0x57, "kp-plus"),
    (0x58, "kp-enter"),
    (0x59, "kp-1"),
    (0x5A, "kp-2"),
    (0x5B, "kp-3"),
    (0x5C, "kp-4"),
    (0x5D, "kp-5"),
    (0x5E, "kp-6"),
    (0x5F, "kp-7"),
    (0x60, "kp-8"),
    (0x61, "kp-9"),
    (0x62, "kp-0"),
    (0x63, "kp-dot"),
    (0x64, "nonus-backslash"),
    (0x65, "menu"),
    (0x66, "power"),
    (0x67, "kp-equal"),
    (0x68, "f13"),
    (0x69, "f14"),
    (0x6A, "f15"),
    (0x6B, "f16"),
    (0x6C, "f17"),
    (0x6D, "f18"),
    (0x6E, "f19"),
    (0x6F, "f20"),
    (0x70, "f21"),
    (0x71, "f22"),
    (0x72, "f23"),
    (0x73, "f24"),
    (0x87, "ro"),
    (0x88, "kana"),
    (0x89, "yen"),
    (0x8A, "henkan"),
    (0x8B, "muhenkan"),
    (0x90, "lang1"),
    (0x91, "lang2"),
    (0xE0, "lctrl"),
    (0xE1, "lshift"),
    (0xE2, "lalt"),
    (0xE3, "lwin"),
    (0xE4, "rctrl"),
    (0xE5, "rshift"),
    (0xE6, "ralt"),
    (0xE7, "rwin"),
];

const ALIASES: &[(&str, u8)] = &[
    ("escape", 0x29),
    ("return", 0x28),
    ("bksp", 0x2A),
    ("period", 0x37),
    ("capslock", 0x39),
    ("prtsc", 0x46),
    ("ins", 0x49),
    ("delete", 0x4C),
    ("pageup", 0x4B),
    ("pagedown", 0x4E),
    ("app", 0x65),
    ("ctrl", 0xE0),
    ("shift", 0xE1),
    ("alt", 0xE2),
    ("win", 0xE3),
    ("lgui", 0xE3),
    ("rgui", 0xE7),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_are_unique() {
        for (i, (u, n)) in NAMES.iter().enumerate() {
            assert!(
                NAMES[i + 1..].iter().all(|(u2, n2)| u2 != u && n2 != n),
                "{n}"
            );
            assert!(ALIASES.iter().all(|(a, _)| a != n), "{n}");
        }
    }

    #[test]
    fn parse_and_display() {
        assert_eq!("A".parse::<Usage>().unwrap(), Usage(0x04));
        assert_eq!("escape".parse::<Usage>().unwrap(), Usage(0x29));
        assert_eq!("0x2C".parse::<Usage>().unwrap(), Usage(0x2C));
        assert_eq!(Usage(0x2C).to_string(), "space");
        assert_eq!(Usage(0xA5).to_string(), "0xa5");
        assert_eq!("0x00".parse::<Usage>().unwrap(), Usage::NONE);
        assert!("nope".parse::<Usage>().is_err());
    }
}
