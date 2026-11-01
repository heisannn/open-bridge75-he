//! The 75ALHE key matrix (6 rows x 22 columns, 81 keys) and the key selection bitmap.
//!
//! Matrix positions are shared by the keymap send order, the HE bitmap, and the 0x23 ordering.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::keymap::KeyEntry;
use crate::{Error, Result};

pub const ROWS: usize = 6;
pub const COLS: usize = 22;
/// Number of keymap cells (row-major, row * 22 + col).
pub const CELLS: usize = ROWS * COLS;

/// Matrix position.
///
/// As a string it is a key ID (`esc`, `a`, `lshift` ...) or `r<row>c<col>`.
/// Either way, parsing fails unless the position holds a real key.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct KeyPos {
    pub row: u8,
    pub col: u8,
}

impl KeyPos {
    pub const fn new(row: u8, col: u8) -> Self {
        assert!((row as usize) < ROWS && (col as usize) < COLS);
        Self { row, col }
    }

    /// Keymap cell number (row * 22 + col).
    pub const fn index(self) -> usize {
        self.row as usize * COLS + self.col as usize
    }

    pub const fn from_index(index: usize) -> Self {
        Self::new((index / COLS) as u8, (index % COLS) as u8)
    }

    /// The key at this position. `None` for an empty cell.
    pub fn key(self) -> Option<&'static Key> {
        LAYOUT.iter().find(|k| k.pos == self)
    }
}

impl fmt::Display for KeyPos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.key() {
            Some(key) => f.write_str(key.id),
            None => write!(f, "r{}c{}", self.row, self.col),
        }
    }
}

impl FromStr for KeyPos {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let lower = s.trim().to_ascii_lowercase();
        if let Some(key) = LAYOUT.iter().find(|k| k.id == lower) {
            return Ok(key.pos);
        }
        parse_row_col(&lower)
            .filter(|pos| pos.key().is_some())
            .ok_or_else(|| Error::UnknownKey(s.to_string()))
    }
}

fn parse_row_col(s: &str) -> Option<KeyPos> {
    let (row, col) = s.strip_prefix('r')?.split_once('c')?;
    let (row, col): (u8, u8) = (row.parse().ok()?, col.parse().ok()?);
    ((row as usize) < ROWS && (col as usize) < COLS).then(|| KeyPos::new(row, col))
}

impl TryFrom<String> for KeyPos {
    type Error = Error;

    fn try_from(s: String) -> Result<Self> {
        s.parse()
    }
}

impl From<KeyPos> for String {
    fn from(pos: KeyPos) -> String {
        pos.to_string()
    }
}

/// Physical key.
#[derive(Debug)]
pub struct Key {
    /// ID used by the CLI and profiles. Same as the default HID code's name ([`crate::hid::Usage`]).
    pub id: &'static str,
    /// Keycap legend.
    pub label: &'static str,
    pub pos: KeyPos,
    /// Entry in the default keymap.
    pub default: KeyEntry,
    /// Where the keycap sits on the board.
    pub rect: KeyRect,
}

/// Width of the board in quarter units (16u).
pub const BOARD_WIDTH: u8 = 64;
/// Height of the board in quarter units (6.25u).
pub const BOARD_HEIGHT: u8 = 25;

/// Keycap rectangle in quarter units (4 = 1u), measured from the top-left corner of the board.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct KeyRect {
    pub x: u8,
    pub y: u8,
    pub w: u8,
    pub h: u8,
}

impl KeyRect {
    /// Top edge of a matrix row. The function row sits 0.25u above the rest.
    const fn row_top(row: u8) -> u8 {
        if row == 0 { 0 } else { row * 4 + 1 }
    }
}

const fn k(
    id: &'static str,
    label: &'static str,
    row: u8,
    col: u8,
    usage: u8,
    x: u8,
    w: u8,
) -> Key {
    Key {
        id,
        label,
        pos: KeyPos::new(row, col),
        default: KeyEntry::key(usage),
        rect: KeyRect {
            x,
            y: KeyRect::row_top(row),
            w,
            h: 4,
        },
    }
}

/// Layout taken from the official configurator's `KeyInfo_75ALHE.config` (docs/protocol.md section 9).
///
/// The geometry converts the config's on-screen `PosX` / `Width` (1u = about 36.75 px) to quarter units.
pub static LAYOUT: [Key; 81] = [
    k("esc", "Esc", 0, 0, 0x29, 0, 4),
    k("f1", "F1", 0, 2, 0x3A, 6, 4),
    k("f2", "F2", 0, 3, 0x3B, 10, 4),
    k("f3", "F3", 0, 4, 0x3C, 14, 4),
    k("f4", "F4", 0, 5, 0x3D, 18, 4),
    k("f5", "F5", 0, 6, 0x3E, 24, 4),
    k("f6", "F6", 0, 7, 0x3F, 28, 4),
    k("f7", "F7", 0, 8, 0x40, 32, 4),
    k("f8", "F8", 0, 9, 0x41, 36, 4),
    k("f9", "F9", 0, 10, 0x42, 42, 4),
    k("f10", "F10", 0, 11, 0x43, 46, 4),
    k("f11", "F11", 0, 12, 0x44, 50, 4),
    k("f12", "F12", 0, 13, 0x45, 54, 4),
    k("del", "Del", 0, 15, 0x4C, 60, 4),
    k("grave", "`~", 1, 0, 0x35, 0, 4),
    k("1", "1!", 1, 1, 0x1E, 4, 4),
    k("2", "2@", 1, 2, 0x1F, 8, 4),
    k("3", "3#", 1, 3, 0x20, 12, 4),
    k("4", "4$", 1, 4, 0x21, 16, 4),
    k("5", "5%", 1, 5, 0x22, 20, 4),
    k("6", "6^", 1, 6, 0x23, 24, 4),
    k("7", "7&", 1, 7, 0x24, 28, 4),
    k("8", "8*", 1, 8, 0x25, 32, 4),
    k("9", "9(", 1, 9, 0x26, 36, 4),
    k("0", "0)", 1, 10, 0x27, 40, 4),
    k("minus", "-_", 1, 11, 0x2D, 44, 4),
    k("equal", "=+", 1, 12, 0x2E, 48, 4),
    k("backspace", "Back", 1, 14, 0x2A, 52, 8),
    k("home", "Home", 1, 15, 0x4A, 60, 4),
    k("tab", "Tab", 2, 0, 0x2B, 0, 6),
    k("q", "Q", 2, 1, 0x14, 6, 4),
    k("w", "W", 2, 2, 0x1A, 10, 4),
    k("e", "E", 2, 3, 0x08, 14, 4),
    k("r", "R", 2, 4, 0x15, 18, 4),
    k("t", "T", 2, 5, 0x17, 22, 4),
    k("y", "Y", 2, 6, 0x1C, 26, 4),
    k("u", "U", 2, 7, 0x18, 30, 4),
    k("i", "I", 2, 8, 0x0C, 34, 4),
    k("o", "O", 2, 9, 0x12, 38, 4),
    k("p", "P", 2, 10, 0x13, 42, 4),
    k("lbracket", "[{", 2, 11, 0x2F, 46, 4),
    k("rbracket", "]}", 2, 12, 0x30, 50, 4),
    k("backslash", "\\|", 2, 14, 0x31, 54, 6),
    k("pgup", "PgUp", 2, 15, 0x4B, 60, 4),
    k("caps", "Caps", 3, 0, 0x39, 0, 7),
    k("a", "A", 3, 2, 0x04, 7, 4),
    k("s", "S", 3, 3, 0x16, 11, 4),
    k("d", "D", 3, 4, 0x07, 15, 4),
    k("f", "F", 3, 5, 0x09, 19, 4),
    k("g", "G", 3, 6, 0x0A, 23, 4),
    k("h", "H", 3, 7, 0x0B, 27, 4),
    k("j", "J", 3, 8, 0x0D, 31, 4),
    k("k", "K", 3, 9, 0x0E, 35, 4),
    k("l", "L", 3, 10, 0x0F, 39, 4),
    k("semicolon", ";:", 3, 11, 0x33, 43, 4),
    k("quote", "'\"", 3, 12, 0x34, 47, 4),
    k("enter", "Enter", 3, 14, 0x28, 51, 9),
    k("pgdn", "PgDn", 3, 15, 0x4E, 60, 4),
    k("lshift", "LShift", 4, 0, 0xE1, 0, 9),
    k("z", "Z", 4, 2, 0x1D, 9, 4),
    k("x", "X", 4, 3, 0x1B, 13, 4),
    k("c", "C", 4, 4, 0x06, 17, 4),
    k("v", "V", 4, 5, 0x19, 21, 4),
    k("b", "B", 4, 6, 0x05, 25, 4),
    k("n", "N", 4, 7, 0x11, 29, 4),
    k("m", "M", 4, 8, 0x10, 33, 4),
    k("comma", ",<", 4, 9, 0x36, 37, 4),
    k("dot", ".>", 4, 10, 0x37, 41, 4),
    k("slash", "/?", 4, 11, 0x38, 45, 4),
    k("rshift", "RShift", 4, 13, 0xE5, 49, 7),
    k("up", "Up", 4, 14, 0x52, 56, 4),
    k("end", "End", 4, 15, 0x4D, 60, 4),
    k("lctrl", "LCtrl", 5, 0, 0xE0, 0, 5),
    k("lwin", "LWin", 5, 1, 0xE3, 5, 5),
    k("lalt", "LAlt", 5, 2, 0xE2, 10, 5),
    k("space", "Space", 5, 6, 0x2C, 15, 25),
    k("ralt", "RAlt", 5, 10, 0xE6, 40, 5),
    Key {
        id: "fn",
        label: "Fn",
        pos: KeyPos::new(5, 11),
        default: KeyEntry::FN,
        rect: KeyRect {
            x: 45,
            y: KeyRect::row_top(5),
            w: 5,
            h: 4,
        },
    },
    k("left", "Left", 5, 13, 0x50, 52, 4),
    k("down", "Down", 5, 14, 0x51, 56, 4),
    k("right", "Right", 5, 15, 0x4F, 60, 4),
];

/// Key selection bitmap (22 bytes). Keys whose `bit[row]` in `byte[col]` is 1 are selected.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct KeyBitmap(pub [u8; COLS]);

impl KeyBitmap {
    pub const EMPTY: KeyBitmap = KeyBitmap([0; COLS]);

    /// All 81 real keys.
    pub fn all() -> Self {
        LAYOUT.iter().map(|k| k.pos).collect()
    }

    pub fn insert(&mut self, pos: KeyPos) {
        self.0[pos.col as usize] |= 1 << pos.row;
    }

    pub fn remove(&mut self, pos: KeyPos) {
        self.0[pos.col as usize] &= !(1 << pos.row);
    }

    pub fn contains(&self, pos: KeyPos) -> bool {
        self.0[pos.col as usize] & (1 << pos.row) != 0
    }

    /// Iterates in row-major order. Includes the bits of empty cells.
    pub fn iter(&self) -> impl Iterator<Item = KeyPos> + '_ {
        (0..CELLS)
            .map(KeyPos::from_index)
            .filter(|&pos| self.contains(pos))
    }

    pub fn len(&self) -> usize {
        self.0
            .iter()
            .map(|b| (b & 0x3F).count_ones() as usize)
            .sum()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl FromIterator<KeyPos> for KeyBitmap {
    fn from_iter<I: IntoIterator<Item = KeyPos>>(iter: I) -> Self {
        let mut bitmap = KeyBitmap::EMPTY;
        for pos in iter {
            bitmap.insert(pos);
        }
        bitmap
    }
}

impl fmt::Debug for KeyBitmap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set()
            .entries(self.iter().map(|p| p.to_string()))
            .finish()
    }
}

/// Key selection. `all` means all 81 keys.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub enum KeySel {
    All,
    Key(KeyPos),
}

impl KeySel {
    pub fn to_bitmap(sels: &[KeySel]) -> KeyBitmap {
        let mut bitmap = KeyBitmap::EMPTY;
        for sel in sels {
            match sel {
                KeySel::All => return KeyBitmap::all(),
                KeySel::Key(pos) => bitmap.insert(*pos),
            }
        }
        bitmap
    }
}

impl FromStr for KeySel {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        if s.trim().eq_ignore_ascii_case("all") {
            Ok(KeySel::All)
        } else {
            s.parse().map(KeySel::Key)
        }
    }
}

impl fmt::Display for KeySel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KeySel::All => f.write_str("all"),
            KeySel::Key(pos) => pos.fmt(f),
        }
    }
}

impl TryFrom<String> for KeySel {
    type Error = Error;

    fn try_from(s: String) -> Result<Self> {
        s.parse()
    }
}

impl From<KeySel> for String {
    fn from(sel: KeySel) -> String {
        sel.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hid::Usage;
    use crate::keymap::KeyAction;

    #[test]
    fn layout_is_consistent() {
        assert_eq!(KeyBitmap::all().len(), 81);
        for (i, key) in LAYOUT.iter().enumerate() {
            assert!(
                LAYOUT[i + 1..]
                    .iter()
                    .all(|k| k.id != key.id && k.pos != key.pos),
                "{}",
                key.id
            );
            // Key IDs match the default HID code names (except Fn).
            if let KeyAction::Key(usage) = KeyAction::from(key.default) {
                assert_eq!(usage.name(), Some(key.id));
                assert_eq!(key.id.parse::<Usage>().unwrap(), usage);
            } else {
                assert_eq!(key.id, "fn");
            }
        }
    }

    #[test]
    fn geometry_fits_the_board_without_overlaps() {
        for (i, key) in LAYOUT.iter().enumerate() {
            let r = key.rect;
            assert!(r.w >= 4 && r.h == 4, "{}", key.id);
            assert!(
                r.x + r.w <= BOARD_WIDTH && r.y + r.h <= BOARD_HEIGHT,
                "{}",
                key.id
            );
            for other in &LAYOUT[i + 1..] {
                let o = other.rect;
                let apart =
                    r.x + r.w <= o.x || o.x + o.w <= r.x || r.y + r.h <= o.y || o.y + o.h <= r.y;
                assert!(apart, "{} overlaps {}", key.id, other.id);
            }
        }
    }

    #[test]
    fn geometry_follows_matrix_order() {
        // Within a row, columns run left to right.
        for row in 0..ROWS as u8 {
            let xs: Vec<u8> = LAYOUT
                .iter()
                .filter(|k| k.pos.row == row)
                .map(|k| k.rect.x)
                .collect();
            assert!(xs.windows(2).all(|w| w[0] < w[1]), "row {row}");
        }
        // Rows 1-5 fill 16u, ignoring the gaps in front of the arrow keys.
        for id in ["home", "pgup", "pgdn", "end", "right"] {
            let rect = id.parse::<KeyPos>().unwrap().key().unwrap().rect;
            assert_eq!(rect.x + rect.w, BOARD_WIDTH, "{id}");
        }
    }

    #[test]
    fn boot_keys_match_firmware() {
        // firmware.md section 8: boot keys S / X / F are at (3,3) (4,3) (3,5).
        assert_eq!("s".parse::<KeyPos>().unwrap(), KeyPos::new(3, 3));
        assert_eq!("x".parse::<KeyPos>().unwrap(), KeyPos::new(4, 3));
        assert_eq!("f".parse::<KeyPos>().unwrap(), KeyPos::new(3, 5));
    }

    #[test]
    fn parse_positions() {
        assert_eq!("R5C11".parse::<KeyPos>().unwrap().to_string(), "fn");
        assert!("r0c1".parse::<KeyPos>().is_err()); // empty cell
        assert!("r6c0".parse::<KeyPos>().is_err());
    }

    #[test]
    fn bitmap_layout() {
        let mut bitmap = KeyBitmap::EMPTY;
        bitmap.insert(KeyPos::new(3, 2)); // A
        bitmap.insert(KeyPos::new(5, 2)); // LAlt
        assert_eq!(bitmap.0[2], 0b0010_1000);
        assert_eq!(
            bitmap.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
            ["a", "lalt"]
        );
        assert_eq!(bitmap.len(), 2);
    }
}
