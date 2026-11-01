//! Macros (wire 0x19). 10 slots x 256 bytes, fixed.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::hid::Usage;
use crate::{Error, Result};

/// Bytes per slot.
pub const SLOT_LEN: usize = 256;
const HEADER_LEN: usize = 8;
/// Number of events that fit in one slot.
pub const MAX_EVENTS: usize = (SLOT_LEN - HEADER_LEN) / 4;
/// Minimum delay between events (ms). The FW counts in 4ms steps.
pub const MIN_DELAY_MS: u16 = 4;

/// Repeat mode (low 2 bits of slot offset 1).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RepeatMode {
    /// Once.
    #[default]
    Once = 0,
    /// `count` times.
    Count = 1,
    /// Repeat until pressed again.
    Toggle = 2,
    /// Repeat only while held.
    Hold = 3,
}

impl RepeatMode {
    fn from_raw(raw: u8) -> Self {
        match raw & 3 {
            0 => RepeatMode::Once,
            1 => RepeatMode::Count,
            2 => RepeatMode::Toggle,
            _ => RepeatMode::Hold,
        }
    }
}

/// Macro target. As a string it is a key name or `mouse:left` / `mouse:right` / `mouse:middle`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum MacroTarget {
    Key(Usage),
    /// Mouse button mask (1=left 2=right 4=middle).
    Mouse(u8),
}

const MOUSE_NAMES: &[(u8, &str)] = &[(1, "left"), (2, "right"), (4, "middle")];

impl fmt::Display for MacroTarget {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MacroTarget::Key(u) => write!(f, "{u}"),
            MacroTarget::Mouse(mask) => match MOUSE_NAMES.iter().find(|(m, _)| m == mask) {
                Some((_, name)) => write!(f, "mouse:{name}"),
                None => write!(f, "mouse:0x{mask:02x}"),
            },
        }
    }
}

impl FromStr for MacroTarget {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let lower = s.trim().to_ascii_lowercase();
        let Some(button) = lower.strip_prefix("mouse:") else {
            return lower.parse().map(MacroTarget::Key);
        };
        let mask = match MOUSE_NAMES.iter().find(|(_, n)| *n == button) {
            Some((mask, _)) => Some(*mask),
            None => button
                .strip_prefix("0x")
                .and_then(|h| u8::from_str_radix(h, 16).ok()),
        };
        mask.filter(|&m| m != 0)
            .map(MacroTarget::Mouse)
            .ok_or_else(|| Error::Invalid(format!("macro mouse button: {s}")))
    }
}

impl TryFrom<String> for MacroTarget {
    type Error = Error;

    fn try_from(s: String) -> Result<Self> {
        s.parse()
    }
}

impl From<MacroTarget> for String {
    fn from(t: MacroTarget) -> String {
        t.to_string()
    }
}

/// Press or release.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MacroAction {
    Press(MacroTarget),
    Release(MacroTarget),
}

/// Event `[code, flags, delayHi, delayLo]`.
///
/// In profiles it takes the form `{ press = "a", delay = 20 }` or `{ release = "a", delay = 20 }`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "MacroEventToml", into = "MacroEventToml")]
pub struct MacroEvent {
    pub action: MacroAction,
    /// Delay until the next event (ms, in 4ms steps, minimum 4).
    pub delay: u16,
}

/// Form of an event in profiles. Write exactly one of press and release, plus delay.
#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MacroEventToml {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    press: Option<MacroTarget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    release: Option<MacroTarget>,
    delay: u16,
}

impl TryFrom<MacroEventToml> for MacroEvent {
    type Error = Error;

    fn try_from(e: MacroEventToml) -> Result<Self> {
        let action = match (e.press, e.release) {
            (Some(t), None) => MacroAction::Press(t),
            (None, Some(t)) => MacroAction::Release(t),
            _ => {
                return Err(Error::Invalid(
                    "a macro event must have exactly one of press and release".into(),
                ));
            }
        };
        Ok(MacroEvent {
            action,
            delay: e.delay,
        })
    }
}

impl From<MacroEvent> for MacroEventToml {
    fn from(e: MacroEvent) -> Self {
        let (press, release) = match e.action {
            MacroAction::Press(t) => (Some(t), None),
            MacroAction::Release(t) => (None, Some(t)),
        };
        MacroEventToml {
            press,
            release,
            delay: e.delay,
        }
    }
}

impl MacroEvent {
    fn encode(&self) -> [u8; 4] {
        let (target, pressed) = match self.action {
            MacroAction::Press(t) => (t, true),
            MacroAction::Release(t) => (t, false),
        };
        // flags: bit4 = press; a nonzero low nibble means mouse (the official configurator sets bit0).
        let (code, mouse) = match target {
            MacroTarget::Key(u) => (u.0, 0),
            MacroTarget::Mouse(mask) => (mask, 0x01),
        };
        let flags = if pressed { 0x10 } else { 0 } | mouse;
        let [hi, lo] = self.delay.to_be_bytes();
        [code, flags, hi, lo]
    }

    fn decode(bytes: [u8; 4]) -> Self {
        let [code, flags, hi, lo] = bytes;
        let target = if flags & 0x0F != 0 {
            MacroTarget::Mouse(code)
        } else {
            MacroTarget::Key(Usage(code))
        };
        let action = if flags & 0x10 != 0 {
            MacroAction::Press(target)
        } else {
            MacroAction::Release(target)
        };
        MacroEvent {
            action,
            delay: u16::from_be_bytes([hi, lo]),
        }
    }
}

/// A macro for one slot. Its form in profiles is [`crate::profile::MacroSlot`].
///
/// If `events` is empty it is written as an empty macro ([`EMPTY_SLOT`]), so the key's fallback HID is sent.
#[derive(Clone, PartialEq, Eq, Hash, Debug, Default)]
pub struct Macro {
    pub repeat: RepeatMode,
    /// Count when `repeat = "count"`.
    pub count: u16,
    pub events: Vec<MacroEvent>,
}

impl Macro {
    pub fn validate(&self) -> Result<()> {
        if self.events.len() > MAX_EVENTS {
            return Err(Error::OutOfRange(format!(
                "a macro can have at most {MAX_EVENTS} events (got {})",
                self.events.len()
            )));
        }
        if let Some(e) = self.events.iter().find(|e| e.delay < MIN_DELAY_MS) {
            return Err(Error::OutOfRange(format!(
                "macro delay must be at least {MIN_DELAY_MS}ms (got {}ms)",
                e.delay
            )));
        }
        if self.repeat == RepeatMode::Count && self.count == 0 {
            return Err(Error::OutOfRange(
                "macro repeat count must be at least 1 (got 0)".into(),
            ));
        }
        // The code of the first event goes into offset 8. If it is 0xFF, the FW treats the macro as empty.
        if let Some(e) = self.events.first()
            && e.encode()[0] == EMPTY_MARK
        {
            return Err(Error::Invalid(
                "0xff cannot be used in the first macro event (the keyboard treats the macro as empty)"
                    .into(),
            ));
        }
        Ok(())
    }

    /// Converts to a 256-byte slot. [`EMPTY_SLOT`] if `events` is empty.
    pub fn encode(&self) -> Result<[u8; SLOT_LEN]> {
        self.validate()?;
        if self.events.is_empty() {
            return Ok(EMPTY_SLOT);
        }
        let mut slot = [0; SLOT_LEN];
        slot[1] = self.repeat as u8;
        slot[2..4].copy_from_slice(&((self.events.len() * 4) as u16).to_be_bytes());
        slot[4..6].copy_from_slice(&self.count.to_be_bytes());
        for (i, event) in self.events.iter().enumerate() {
            let at = HEADER_LEN + 4 * i;
            slot[at..at + 4].copy_from_slice(&event.encode());
        }
        Ok(slot)
    }

    /// Reads a 256-byte slot. Empty, and so `None`, if offset 8 is 0xFF.
    ///
    /// The FW treats a slot as empty only when offset 8 is 0xFF. A zero-filled slot is a zero-length macro
    /// (no fallback HID is sent), so it becomes `Some` with empty `events`.
    pub fn decode(slot: &[u8; SLOT_LEN]) -> Option<Self> {
        if slot[HEADER_LEN] == EMPTY_MARK {
            return None;
        }
        let len = u16::from_be_bytes([slot[2], slot[3]]) as usize;
        let count = (len / 4).min(MAX_EVENTS);
        let events = (0..count)
            .map(|i| {
                let at = HEADER_LEN + 4 * i;
                MacroEvent::decode(slot[at..at + 4].try_into().unwrap())
            })
            .collect();
        Some(Macro {
            repeat: RepeatMode::from_raw(slot[1]),
            count: u16::from_be_bytes([slot[4], slot[5]]),
            events,
        })
    }
}

/// An empty macro if offset 8 has this value. The FW sends the key's fallback HID (p1 of type 0x10).
const EMPTY_MARK: u8 = 0xFF;

/// Empty slot (offset 8 is 0xFF, the rest is 0).
///
/// The official configurator fills unused slots with 0, but the FW treats that as a zero-length macro
/// and does not send the fallback HID.
pub const EMPTY_SLOT: [u8; SLOT_LEN] = {
    let mut slot = [0; SLOT_LEN];
    slot[HEADER_LEN] = EMPTY_MARK;
    slot
};

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Macro {
        let a = MacroTarget::Key(Usage(0x04));
        Macro {
            repeat: RepeatMode::Count,
            count: 3,
            events: vec![
                MacroEvent {
                    action: MacroAction::Press(a),
                    delay: 20,
                },
                MacroEvent {
                    action: MacroAction::Release(a),
                    delay: 4,
                },
                MacroEvent {
                    action: MacroAction::Press(MacroTarget::Mouse(1)),
                    delay: 300,
                },
                MacroEvent {
                    action: MacroAction::Release(MacroTarget::Mouse(1)),
                    delay: 8,
                },
            ],
        }
    }

    #[test]
    fn encode_layout() {
        let slot = sample().encode().unwrap();
        assert_eq!(&slot[..8], &[0, 1, 0, 16, 0, 3, 0, 0]);
        assert_eq!(&slot[8..12], &[0x04, 0x10, 0, 20]);
        assert_eq!(&slot[12..16], &[0x04, 0x00, 0, 4]);
        assert_eq!(&slot[16..20], &[0x01, 0x11, 0x01, 0x2C]);
        assert_eq!(&slot[20..24], &[0x01, 0x01, 0, 8]);
        assert!(slot[24..].iter().all(|&b| b == 0));
        assert_eq!(Macro::decode(&slot), Some(sample()));
    }

    #[test]
    fn rejects_invalid() {
        let mut m = sample();
        m.events[0].delay = 2;
        assert!(m.encode().is_err());
        let e = sample().events[0];
        assert!(
            Macro {
                events: vec![e; MAX_EVENTS + 1],
                ..Macro::default()
            }
            .encode()
            .is_err()
        );
        assert!(
            Macro {
                events: vec![e; MAX_EVENTS],
                ..Macro::default()
            }
            .encode()
            .is_ok()
        );
    }

    #[test]
    fn rejects_bad_count_and_mark() {
        let mut m = sample();
        m.count = 0;
        assert!(m.encode().is_err());
        m.repeat = RepeatMode::Once;
        assert!(m.encode().is_ok());

        let mut m = sample();
        m.events[0].action = MacroAction::Press(MacroTarget::Key(Usage(0xFF)));
        assert!(m.encode().is_err());
        // From the second event on, it does not go into offset 8.
        let mut m = sample();
        m.events[1].action = MacroAction::Release(MacroTarget::Key(Usage(0xFF)));
        assert!(m.encode().is_ok());
    }

    #[test]
    fn empty_slots() {
        assert_eq!(EMPTY_SLOT[8], 0xFF);
        assert!(
            EMPTY_SLOT
                .iter()
                .enumerate()
                .all(|(i, &b)| i == 8 || b == 0)
        );
        assert_eq!(Macro::decode(&EMPTY_SLOT), None);
        let mut erased = [0xFF; SLOT_LEN];
        erased[2] = 0;
        erased[3] = 4;
        assert_eq!(Macro::decode(&erased), None);

        // A macro with empty events becomes an empty slot (the fallback HID is sent).
        let empty = Macro {
            repeat: RepeatMode::Hold,
            count: 5,
            events: vec![],
        };
        assert_eq!(empty.encode().unwrap(), EMPTY_SLOT);

        // A zero-filled slot is a zero-length macro in the FW, not an empty one.
        let zeroed = Macro::decode(&[0; SLOT_LEN]).unwrap();
        assert!(zeroed.events.is_empty());
    }

    #[test]
    fn event_toml_form() {
        #[derive(Deserialize, Serialize)]
        struct Wrap {
            e: MacroEvent,
        }
        let parse = |s: &str| toml::from_str::<Wrap>(s).map(|w| w.e);
        let e = parse(r#"e = { press = "a", delay = 20 }"#).unwrap();
        assert_eq!(e.action, MacroAction::Press(MacroTarget::Key(Usage(0x04))));
        let e = parse(r#"e = { release = "mouse:left", delay = 8 }"#).unwrap();
        assert_eq!(e.action, MacroAction::Release(MacroTarget::Mouse(1)));
        let text = toml::to_string(&Wrap { e }).unwrap();
        assert_eq!(parse(&text).unwrap(), e, "{text}");
        assert!(!text.contains("press"), "{text}");
        for bad in [
            r#"e = { press = "a", release = "b", delay = 4 }"#,
            r#"e = { delay = 4 }"#,
            r#"e = { press = "a" }"#,
            r#"e = { press = "a", dealy = 4 }"#,
            r#"e = { press = "a", delay = 4, wait = 4 }"#,
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
