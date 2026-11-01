//! Keymap entries (wire 0x18) and layers.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::hid::Usage;
use crate::matrix::{CELLS, KeyPos, LAYOUT};
use crate::{Error, Result};

/// Number of macro slots.
pub const MACRO_SLOTS: u8 = 10;
/// Number of advanced key slots.
pub const ADVANCED_SLOTS: u8 = 40;
/// Number of SOCD pairs.
pub const SOCD_PAIRS: u8 = 20;
/// Upper bound of media key bit numbers (0..63).
pub const MEDIA_BITS: u8 = 64;

/// A 4-byte keymap entry `[type, p1, p2, p3]`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Debug)]
pub struct KeyEntry(pub [u8; 4]);

impl KeyEntry {
    /// Sends nothing.
    pub const NONE: KeyEntry = KeyEntry([0; 4]);
    pub const FN: KeyEntry = KeyEntry([0x01, 0, 0, 0]);

    /// Regular key.
    pub const fn key(usage: u8) -> Self {
        KeyEntry([0x00, usage, 0, 0])
    }
}

/// A keymap entry split up by meaning.
///
/// String notation ([`fmt::Display`] / [`FromStr`]):
///
/// | Notation | Meaning |
/// |---|---|
/// | `none` | Sends nothing |
/// | `a`, `lctrl`, `0x2c` | Regular key (HID usage) |
/// | `fn` | Fn |
/// | `media:16`, `media:volup` | Media key (FW-internal bit number 0..63) |
/// | `macro:0`, `macro:0:a` | Macro 0..9 (the suffix is the fallback HID sent when that slot is empty) |
/// | `sys:nkro`, etc. | System function ([`SystemFn`]) |
/// | `mouse:left`, `mouse:left+right` | Mouse buttons |
/// | `wheel:up`, `wheel:down` | Wheel |
/// | `combo:lctrl+c` | Combo (up to 3 keys) |
/// | `event:f13` | Custom event key (notifies the PC while the driver is connected, otherwise acts as that key) |
/// | `adv:0` | Advanced key 0..39 |
/// | `socd:0`, `socd:all` | Toggle an SOCD pair / toggle globally |
/// | `raw:12000100` | Raw 4 bytes (the index range is checked when it refers to a macro, advanced key, or SOCD pair) |
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum KeyAction {
    None,
    Key(Usage),
    Fn,
    Media(u8),
    Macro { index: u8, fallback: Usage },
    System(SystemFn),
    Mouse(MouseButtons),
    Wheel(Wheel),
    Combo([Usage; 3]),
    CustomEvent(Usage),
    Advanced(u8),
    SocdToggle(u8),
    SocdToggleAll,
    Raw(KeyEntry),
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Wheel {
    Up,
    Down,
}

/// Type 0x11 system functions.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum SystemFn {
    /// Toggle all-key lock.
    LockAll,
    /// Toggle between 6KRO / NKRO.
    KroToggle,
    Kro6,
    Nkro,
    /// Toggle Win lock.
    WinLockToggle,
    /// Win lock (p2=1 in the official configurator).
    WinLockOn,
    /// Win unlock (p2=2 in the official configurator).
    WinLockOff,
    OsWin,
    OsMac,
}

const SYSTEM_FNS: &[(SystemFn, &str, [u8; 2])] = &[
    (SystemFn::LockAll, "lock", [0, 0]),
    (SystemFn::KroToggle, "kro-toggle", [1, 0]),
    (SystemFn::Kro6, "6kro", [1, 1]),
    (SystemFn::Nkro, "nkro", [1, 2]),
    (SystemFn::WinLockToggle, "winlock", [2, 0]),
    (SystemFn::WinLockOn, "winlock-on", [2, 1]),
    (SystemFn::WinLockOff, "winlock-off", [2, 2]),
    (SystemFn::OsWin, "os-win", [9, 1]),
    (SystemFn::OsMac, "os-mac", [9, 3]),
];

impl SystemFn {
    fn entry(self) -> &'static (SystemFn, &'static str, [u8; 2]) {
        SYSTEM_FNS.iter().find(|(f, _, _)| *f == self).unwrap()
    }

    pub fn name(self) -> &'static str {
        self.entry().1
    }

    pub fn params(self) -> [u8; 2] {
        self.entry().2
    }

    fn from_params(params: [u8; 2]) -> Option<Self> {
        SYSTEM_FNS
            .iter()
            .find(|(_, _, p)| *p == params)
            .map(|(f, _, _)| *f)
    }

    fn from_name(name: &str) -> Option<Self> {
        SYSTEM_FNS
            .iter()
            .find(|(_, n, _)| *n == name)
            .map(|(f, _, _)| *f)
    }

    pub fn all() -> impl Iterator<Item = SystemFn> {
        SYSTEM_FNS.iter().map(|(f, _, _)| *f)
    }
}

/// Mouse button mask (1=left 2=right 4=middle 8=back 16=forward).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct MouseButtons(pub u8);

const MOUSE_BUTTONS: &[(u8, &str)] = &[
    (1, "left"),
    (2, "right"),
    (4, "middle"),
    (8, "back"),
    (16, "forward"),
];

impl fmt::Display for MouseButtons {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0 & !0x1F != 0 || self.0 == 0 {
            return write!(f, "0x{:02x}", self.0);
        }
        let names: Vec<_> = MOUSE_BUTTONS
            .iter()
            .filter(|(bit, _)| self.0 & bit != 0)
            .map(|(_, n)| *n)
            .collect();
        f.write_str(&names.join("+"))
    }
}

impl FromStr for MouseButtons {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let lower = s.trim().to_ascii_lowercase();
        if let Some(hex) = lower.strip_prefix("0x") {
            return u8::from_str_radix(hex, 16)
                .ok()
                .filter(|&v| v != 0)
                .map(MouseButtons)
                .ok_or_else(|| Error::InvalidAction(s.to_string()));
        }
        let mut mask = 0;
        for name in lower.split('+') {
            let bit = MOUSE_BUTTONS
                .iter()
                .find(|(_, n)| *n == name)
                .map(|(bit, _)| *bit)
                .ok_or_else(|| Error::InvalidAction(format!("mouse button {name}")))?;
            mask |= bit;
        }
        Ok(MouseButtons(mask))
    }
}

impl TryFrom<String> for MouseButtons {
    type Error = Error;

    fn try_from(s: String) -> Result<Self> {
        s.parse()
    }
}

impl From<MouseButtons> for String {
    fn from(m: MouseButtons) -> String {
        m.to_string()
    }
}

/// Media key bit numbers known from the official configurator's assignments.
const MEDIA_NAMES: &[(u8, &str)] = &[(16, "volup"), (17, "voldown"), (18, "mute")];

impl From<KeyAction> for KeyEntry {
    fn from(action: KeyAction) -> KeyEntry {
        use KeyAction as A;
        KeyEntry(match action {
            A::None => [0, 0, 0, 0],
            A::Key(u) => [0x00, u.0, 0, 0],
            A::Fn => [0x01, 0, 0, 0],
            A::Media(bit) => [0x07, bit, 0, 0],
            A::Macro { index, fallback } => [0x10, fallback.0, index, 0],
            A::System(f) => {
                let [p1, p2] = f.params();
                [0x11, p1, p2, 0]
            }
            A::Mouse(m) => [0x12, 0, m.0, 0],
            A::Wheel(Wheel::Up) => [0x12, 1, 0x01, 0],
            A::Wheel(Wheel::Down) => [0x12, 1, 0xFF, 0],
            A::Combo([a, b, c]) => [0x13, a.0, b.0, c.0],
            A::CustomEvent(u) => [0x16, 0, 0, u.0],
            A::Advanced(slot) => [0x17, slot, 0, 0],
            A::SocdToggle(pair) => [0x1A, pair, 0, 0],
            A::SocdToggleAll => [0x1A, 0, 1, 0],
            A::Raw(entry) => entry.0,
        })
    }
}

impl From<KeyEntry> for KeyAction {
    /// Becomes [`KeyAction::Raw`] unless it can be written back exactly as notated.
    fn from(entry: KeyEntry) -> KeyAction {
        use KeyAction as A;
        match entry.0 {
            [0, 0, 0, 0] => A::None,
            [0x00, u, 0, 0] => A::Key(Usage(u)),
            [0x01, 0, 0, 0] => A::Fn,
            [0x07, bit, 0, 0] if bit < MEDIA_BITS => A::Media(bit),
            [0x10, fallback, index, 0] if index < MACRO_SLOTS => A::Macro {
                index,
                fallback: Usage(fallback),
            },
            [0x11, p1, p2, 0] if SystemFn::from_params([p1, p2]).is_some() => {
                A::System(SystemFn::from_params([p1, p2]).unwrap())
            }
            [0x12, 0, mask, 0] if mask != 0 => A::Mouse(MouseButtons(mask)),
            [0x12, 1, 0x01, 0] => A::Wheel(Wheel::Up),
            [0x12, 1, 0xFF, 0] => A::Wheel(Wheel::Down),
            [0x13, a, b, c] if a != 0 && (b != 0 || c == 0) => {
                A::Combo([Usage(a), Usage(b), Usage(c)])
            }
            [0x16, 0, 0, u] if u != 0 => A::CustomEvent(Usage(u)),
            [0x17, slot, 0, 0] if slot < ADVANCED_SLOTS => A::Advanced(slot),
            [0x1A, 0, 1, 0] => A::SocdToggleAll,
            [0x1A, pair, 0, 0] if pair < SOCD_PAIRS => A::SocdToggle(pair),
            _ => A::Raw(entry),
        }
    }
}

impl fmt::Display for KeyAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use KeyAction as A;
        match self {
            A::None => f.write_str("none"),
            A::Key(u) => write!(f, "{u}"),
            A::Fn => f.write_str("fn"),
            A::Media(bit) => match MEDIA_NAMES.iter().find(|(b, _)| b == bit) {
                Some((_, name)) => write!(f, "media:{name}"),
                None => write!(f, "media:{bit}"),
            },
            A::Macro { index, fallback } if *fallback == Usage::NONE => write!(f, "macro:{index}"),
            A::Macro { index, fallback } => write!(f, "macro:{index}:{fallback}"),
            A::System(s) => write!(f, "sys:{}", s.name()),
            A::Mouse(m) => write!(f, "mouse:{m}"),
            A::Wheel(Wheel::Up) => f.write_str("wheel:up"),
            A::Wheel(Wheel::Down) => f.write_str("wheel:down"),
            A::Combo(keys) => {
                let keys: Vec<_> = keys
                    .iter()
                    .filter(|u| **u != Usage::NONE)
                    .map(|u| u.to_string())
                    .collect();
                write!(f, "combo:{}", keys.join("+"))
            }
            A::CustomEvent(u) => write!(f, "event:{u}"),
            A::Advanced(slot) => write!(f, "adv:{slot}"),
            A::SocdToggle(pair) => write!(f, "socd:{pair}"),
            A::SocdToggleAll => f.write_str("socd:all"),
            A::Raw(KeyEntry([t, a, b, c])) => write!(f, "raw:{t:02x}{a:02x}{b:02x}{c:02x}"),
        }
    }
}

impl FromStr for KeyAction {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        use KeyAction as A;
        let lower = s.trim().to_ascii_lowercase();
        let invalid = || Error::InvalidAction(s.to_string());
        let index = |v: &str, max: u8| {
            v.parse::<u8>()
                .ok()
                .filter(|&v| v < max)
                .ok_or_else(invalid)
        };

        let Some((kind, arg)) = lower.split_once(':') else {
            return match lower.as_str() {
                "none" => Ok(A::None),
                "fn" => Ok(A::Fn),
                _ => match lower.parse() {
                    Ok(Usage::NONE) => Ok(A::None),
                    Ok(u) => Ok(A::Key(u)),
                    Err(_) => Err(invalid()),
                },
            };
        };
        let usage = |v: &str| match v.parse::<Usage>()? {
            Usage::NONE => Err(invalid()),
            u => Ok(u),
        };
        match kind {
            "media" => match MEDIA_NAMES.iter().find(|(_, n)| *n == arg) {
                Some((bit, _)) => Ok(A::Media(*bit)),
                None => index(arg, MEDIA_BITS).map(A::Media),
            },
            "macro" => {
                let (slot, fallback) = match arg.split_once(':') {
                    Some((slot, fallback)) => (slot, fallback.parse()?),
                    None => (arg, Usage::NONE),
                };
                Ok(A::Macro {
                    index: index(slot, MACRO_SLOTS)?,
                    fallback,
                })
            }
            "sys" => SystemFn::from_name(arg).map(A::System).ok_or_else(invalid),
            "mouse" => arg.parse().map(A::Mouse),
            "wheel" => match arg {
                "up" => Ok(A::Wheel(Wheel::Up)),
                "down" => Ok(A::Wheel(Wheel::Down)),
                _ => Err(invalid()),
            },
            "combo" => {
                let keys: Vec<Usage> = arg.split('+').map(usage).collect::<Result<_>>()?;
                if keys.is_empty() || keys.len() > 3 {
                    return Err(Error::InvalidAction(format!(
                        "{s}: a combo has 1 to 3 keys"
                    )));
                }
                let mut combo = [Usage::NONE; 3];
                combo[..keys.len()].copy_from_slice(&keys);
                Ok(A::Combo(combo))
            }
            "event" => usage(arg).map(A::CustomEvent),
            "adv" => index(arg, ADVANCED_SLOTS).map(A::Advanced),
            "socd" if arg == "all" => Ok(A::SocdToggleAll),
            "socd" => index(arg, SOCD_PAIRS).map(A::SocdToggle),
            "raw" => {
                if arg.len() != 8 || !arg.bytes().all(|b| b.is_ascii_hexdigit()) {
                    return Err(invalid());
                }
                let entry = KeyEntry(u32::from_str_radix(arg, 16).unwrap().to_be_bytes());
                check_raw_slot(entry).map_err(|e| Error::InvalidAction(format!("{s}: {e}")))?;
                Ok(A::Raw(entry))
            }
            _ => Err(invalid()),
        }
    }
}

/// Checks the slot index a raw entry refers to, because the FW reads RAM without checking the range.
fn check_raw_slot(entry: KeyEntry) -> std::result::Result<(), String> {
    let [t, p1, p2, _] = entry.0;
    match t {
        0x10 if p2 >= MACRO_SLOTS => Err(format!(
            "macro index (byte 3) must be 0 to {} (got {p2})",
            MACRO_SLOTS - 1
        )),
        0x17 if p1 >= ADVANCED_SLOTS => Err(format!(
            "advanced key index (byte 2) must be 0 to {} (got {p1})",
            ADVANCED_SLOTS - 1
        )),
        0x1A if p2 != 1 && p1 >= SOCD_PAIRS => Err(format!(
            "SOCD pair index (byte 2) must be 0 to {} (got {p1})",
            SOCD_PAIRS - 1
        )),
        _ => Ok(()),
    }
}

impl TryFrom<String> for KeyAction {
    type Error = Error;

    fn try_from(s: String) -> Result<Self> {
        s.parse()
    }
}

impl From<KeyAction> for String {
    fn from(action: KeyAction) -> String {
        action.to_string()
    }
}

/// Keymap layer (`b[2]`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Layer {
    Base = 0,
    /// Layer used while Fn is held.
    Fn = 2,
}

/// Keymap for one layer (132 cells, row-major).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Keymap(pub [KeyEntry; CELLS]);

impl Keymap {
    /// The default layout. Empty cells are `00 00 00 00`.
    ///
    /// The Fn layer default is the same (keys other than the fixed-function keys work as their default HID code).
    /// The FW refuses to override fixed-function keys (the 0x0E mask), so whatever is sent, the FW's own function remains.
    pub fn default_layer() -> Self {
        let mut map = Keymap([KeyEntry::NONE; CELLS]);
        for key in &LAYOUT {
            map.set(key.pos, key.default);
        }
        map
    }

    pub fn get(&self, pos: KeyPos) -> KeyEntry {
        self.0[pos.index()]
    }

    pub fn set(&mut self, pos: KeyPos, entry: KeyEntry) {
        self.0[pos.index()] = entry;
    }
}

impl Default for Keymap {
    fn default() -> Self {
        Self::default_layer()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trip() {
        for text in [
            "none",
            "a",
            "lctrl",
            "fn",
            "media:volup",
            "media:48",
            "macro:3",
            "macro:9:enter",
            "sys:nkro",
            "sys:os-mac",
            "mouse:left+middle",
            "wheel:down",
            "combo:lctrl+c",
            "combo:lctrl+lshift+esc",
            "event:f13",
            "adv:39",
            "socd:19",
            "socd:all",
            "raw:05000000",
        ] {
            let action: KeyAction = text.parse().unwrap();
            assert_eq!(action.to_string(), text);
            let entry = KeyEntry::from(action);
            assert_eq!(KeyAction::from(entry), action, "{text}");
        }
    }

    #[test]
    fn encodes_like_official_app() {
        let e = |s: &str| KeyEntry::from(s.parse::<KeyAction>().unwrap()).0;
        assert_eq!(e("a"), [0x00, 0x04, 0, 0]);
        assert_eq!(e("macro:2:a"), [0x10, 0x04, 2, 0]);
        assert_eq!(e("combo:lalt+f4"), [0x13, 0xE2, 0x3D, 0]);
        assert_eq!(e("event:a"), [0x16, 0, 0, 0x04]);
        assert_eq!(e("wheel:down"), [0x12, 1, 0xFF, 0]);
        assert_eq!(e("socd:all"), [0x1A, 0, 1, 0]);
    }

    #[test]
    fn rejects_out_of_range() {
        for text in [
            "macro:10",
            "adv:40",
            "socd:20",
            "media:64",
            "combo:a+b+c+d",
            "raw:123",
            "raw:+1234567",
            "x:1",
        ] {
            assert!(text.parse::<KeyAction>().is_err(), "{text}");
        }
    }

    #[test]
    fn raw_slot_range() {
        let raw = |s: &str| s.parse::<KeyAction>();
        // Check the indices of macros (p2), advanced keys (p1), and SOCD pairs (p1 when p2 != 1).
        assert!(raw("raw:10040a00").is_err());
        assert!(raw("raw:1004ff00").is_err());
        assert!(raw("raw:10040900").is_ok());
        assert!(raw("raw:10000901").is_ok());
        assert!(raw("raw:17280000").is_err());
        assert!(raw("raw:17270001").is_ok());
        assert!(raw("raw:1a140000").is_err());
        assert!(raw("raw:1a140200").is_err());
        assert!(raw("raw:1a130001").is_ok());
        // p2 = 1 toggles globally and does not use p1.
        assert!(raw("raw:1a140100").is_ok());
        // Other types are not checked.
        assert!(raw("raw:11ff0000").is_ok());
        // Values received from the keyboard are kept as is.
        let entry = KeyEntry([0x17, 0x50, 0, 0]);
        assert_eq!(KeyAction::from(entry), KeyAction::Raw(entry));
    }

    #[test]
    fn unknown_entries_stay_raw() {
        let entry = KeyEntry([0x11, 12, 0, 0]);
        assert_eq!(KeyAction::from(entry), KeyAction::Raw(entry));
        assert_eq!(KeyEntry::from(KeyAction::from(entry)), entry);
    }

    #[test]
    fn default_layer() {
        let map = Keymap::default_layer();
        assert_eq!(map.get(KeyPos::new(0, 0)), KeyEntry::key(0x29));
        assert_eq!(map.get(KeyPos::new(0, 1)), KeyEntry::NONE);
        assert_eq!(map.get(KeyPos::new(5, 11)), KeyEntry::FN);
        assert_eq!(map.0.iter().filter(|e| **e != KeyEntry::NONE).count(), 81);
    }
}
