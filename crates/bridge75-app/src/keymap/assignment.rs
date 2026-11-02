//! What a key can be assigned, with the names the app shows for it.
//!
//! [`label`] names an assignment in full, as the picker and the editor show it. [`short_label`]
//! is the shorter form that fits on a keycap.

use std::borrow::Cow;

use bridge75_core::hid::{Usage, is_modifier};
use bridge75_core::keymap::{KeyAction, MouseButtons, SystemFn, Wheel};
use gpui_kit::SharedString;
use rust_i18n::t;

/// Modifiers a key can be combined with, as the picker offers them.
pub const COMBO_MODIFIERS: [Usage; 4] = [Usage(0xE0), Usage(0xE1), Usage(0xE2), Usage(0xE3)];

/// A combination holds at most three usages: two modifiers and the key.
pub const MAX_COMBO_MODIFIERS: usize = 2;

/// Media keys whose function is known (FW bit numbers).
const VOLUME_UP: u8 = 16;
const VOLUME_DOWN: u8 = 17;
const MUTE: u8 = 18;

/// A section of the picker.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    General,
    Letters,
    Numbers,
    Symbols,
    Editing,
    Modifiers,
    Function,
    Numpad,
    International,
    Media,
    Mouse,
    Keyboard,
}

impl Category {
    pub const ALL: [Category; 12] = [
        Category::General,
        Category::Letters,
        Category::Numbers,
        Category::Symbols,
        Category::Editing,
        Category::Modifiers,
        Category::Function,
        Category::Numpad,
        Category::International,
        Category::Media,
        Category::Mouse,
        Category::Keyboard,
    ];

    pub fn title(self) -> SharedString {
        match self {
            Category::General => t!("keymap.group.general"),
            Category::Letters => t!("keymap.group.letters"),
            Category::Numbers => t!("keymap.group.numbers"),
            Category::Symbols => t!("keymap.group.symbols"),
            Category::Editing => t!("keymap.group.editing"),
            Category::Modifiers => t!("keymap.group.modifiers"),
            Category::Function => t!("keymap.group.function"),
            Category::Numpad => t!("keymap.group.numpad"),
            Category::International => t!("keymap.group.international"),
            Category::Media => t!("keymap.group.media"),
            Category::Mouse => t!("keymap.group.mouse"),
            Category::Keyboard => t!("keymap.group.keyboard"),
        }
        .into()
    }

    /// The assignments the picker offers in this section.
    pub fn actions(self) -> Vec<KeyAction> {
        let keys = |usages: &mut dyn Iterator<Item = u8>| -> Vec<KeyAction> {
            usages.map(|u| KeyAction::Key(Usage(u))).collect()
        };
        match self {
            Category::General => vec![KeyAction::Fn, KeyAction::None],
            Category::Letters => keys(&mut (0x04..=0x1D)),
            Category::Numbers => keys(&mut (0x1E..=0x27)),
            Category::Symbols => keys(&mut (0x2D..=0x38).filter(|u| *u != 0x32)),
            Category::Editing => keys(
                &mut [
                    0x28, 0x29, 0x2A, 0x2B, 0x2C, 0x39, 0x49, 0x4C, 0x4A, 0x4D, 0x4B, 0x4E, 0x50,
                    0x4F, 0x52, 0x51, 0x46, 0x47, 0x48, 0x65, 0x66,
                ]
                .into_iter(),
            ),
            Category::Modifiers => keys(&mut (0xE0..=0xE7)),
            Category::Function => keys(&mut (0x3A..=0x45).chain(0x68..=0x73)),
            Category::Numpad => keys(&mut (0x53..=0x63).chain([0x67])),
            Category::International => {
                keys(&mut [0x87, 0x88, 0x89, 0x8A, 0x8B, 0x90, 0x91, 0x32, 0x64].into_iter())
            }
            Category::Media => [VOLUME_UP, VOLUME_DOWN, MUTE]
                .map(KeyAction::Media)
                .to_vec(),
            Category::Mouse => [1, 2, 4, 8, 16]
                .map(|bit| KeyAction::Mouse(MouseButtons(bit)))
                .into_iter()
                .chain([Wheel::Up, Wheel::Down].map(KeyAction::Wheel))
                .collect(),
            Category::Keyboard => SystemFn::all().map(KeyAction::System).collect(),
        }
    }
}

/// The names of a usage: in full, and as it fits on a keycap.
fn usage_names(usage: Usage) -> (SharedString, SharedString) {
    let same = |name: &'static str| (SharedString::from(name), SharedString::from(name));
    let pair = |full: &'static str, short: &'static str| (full.into(), short.into());
    let numpad = |key: &'static str| {
        let full: SharedString = t!("keymap.key.numpad", key = key).into();
        (full, SharedString::from(format!("Num{key}")))
    };
    let side = |left: bool, key: &'static str| {
        let full = if left {
            t!("keymap.key.left_modifier", key = key)
        } else {
            t!("keymap.key.right_modifier", key = key)
        };
        let short = format!("{}{key}", if left { "L" } else { "R" });
        (SharedString::from(full), SharedString::from(short))
    };
    let translated = |full: Cow<'static, str>, short: Cow<'static, str>| {
        (SharedString::from(full), SharedString::from(short))
    };
    match usage.0 {
        u @ 0x04..=0x1D => {
            let letter = SharedString::from(((b'A' + u - 0x04) as char).to_string());
            (letter.clone(), letter)
        }
        u @ 0x1E..=0x27 => {
            let digit = SharedString::from(((u - 0x1E + 1) % 10).to_string());
            (digit.clone(), digit)
        }
        0x28 => same("Enter"),
        0x29 => same("Esc"),
        0x2A => pair("Backspace", "Back"),
        0x2B => same("Tab"),
        0x2C => {
            let space: SharedString = t!("keymap.key.space").into();
            (space.clone(), space)
        }
        0x2D => same("-_"),
        0x2E => same("=+"),
        0x2F => same("[{"),
        0x30 => same("]}"),
        0x31 => same("\\|"),
        0x32 => pair("ISO #~", "ISO#"),
        0x33 => same(";:"),
        0x34 => same("'\""),
        0x35 => same("`~"),
        0x36 => same(",<"),
        0x37 => same(".>"),
        0x38 => same("/?"),
        0x39 => pair("Caps Lock", "Caps"),
        u @ 0x3A..=0x45 => {
            let name = SharedString::from(format!("F{}", u - 0x3A + 1));
            (name.clone(), name)
        }
        0x46 => pair("Print Screen", "PrtSc"),
        0x47 => pair("Scroll Lock", "ScrLk"),
        0x48 => same("Pause"),
        0x49 => pair("Insert", "Ins"),
        0x4A => same("Home"),
        0x4B => pair("Page Up", "PgUp"),
        0x4C => pair("Delete", "Del"),
        0x4D => same("End"),
        0x4E => pair("Page Down", "PgDn"),
        0x4F => translated(t!("keymap.key.right"), "→".into()),
        0x50 => translated(t!("keymap.key.left"), "←".into()),
        0x51 => translated(t!("keymap.key.down"), "↓".into()),
        0x52 => translated(t!("keymap.key.up"), "↑".into()),
        0x53 => pair("Num Lock", "NumLk"),
        0x54 => numpad("/"),
        0x55 => numpad("*"),
        0x56 => numpad("-"),
        0x57 => numpad("+"),
        0x58 => numpad("Enter"),
        u @ 0x59..=0x61 => {
            let digit = (u - 0x59 + 1).to_string();
            let full: SharedString = t!("keymap.key.numpad", key = digit).into();
            (full, format!("Num{digit}").into())
        }
        0x62 => numpad("0"),
        0x63 => numpad("."),
        0x64 => pair("ISO \\|", "ISO\\"),
        0x65 => same("Menu"),
        0x66 => {
            let power: SharedString = t!("keymap.key.power").into();
            (power.clone(), power)
        }
        0x67 => numpad("="),
        u @ 0x68..=0x73 => {
            let name = SharedString::from(format!("F{}", u - 0x68 + 13));
            (name.clone(), name)
        }
        0x87 => translated(t!("keymap.key.ro"), t!("keymap.key.ro.short")),
        0x88 => translated(t!("keymap.key.kana"), t!("keymap.key.kana.short")),
        0x89 => same("¥"),
        0x8A => translated(t!("keymap.key.henkan"), t!("keymap.key.henkan.short")),
        0x8B => translated(t!("keymap.key.muhenkan"), t!("keymap.key.muhenkan.short")),
        0x90 => translated(t!("keymap.key.lang1"), t!("keymap.key.lang1.short")),
        0x91 => translated(t!("keymap.key.lang2"), t!("keymap.key.lang2.short")),
        0xE0 => side(true, "Ctrl"),
        0xE1 => side(true, "Shift"),
        0xE2 => side(true, "Alt"),
        0xE3 => side(true, "Win"),
        0xE4 => side(false, "Ctrl"),
        0xE5 => side(false, "Shift"),
        0xE6 => side(false, "Alt"),
        0xE7 => side(false, "Win"),
        _ => {
            let code = SharedString::from(format!("0x{:02X}", usage.0));
            (t!("keymap.key.usage", code = code).into(), code)
        }
    }
}

/// How a usage reads inside a combination: left modifiers by their plain name.
pub fn modifier_name(usage: Usage) -> SharedString {
    match usage.0 {
        0xE0 => "Ctrl".into(),
        0xE1 => "Shift".into(),
        0xE2 => "Alt".into(),
        0xE3 => "Win".into(),
        _ => usage_names(usage).1,
    }
}

fn combo(usages: &[Usage; 3]) -> SharedString {
    let parts: Vec<SharedString> = usages
        .iter()
        .filter(|u| **u != Usage::NONE)
        .map(|u| modifier_name(*u))
        .collect();
    parts.join("+").into()
}

fn system_names(function: SystemFn) -> (SharedString, SharedString) {
    let (full, short) = match function {
        SystemFn::LockAll => (t!("keymap.sys.lock_all"), t!("keymap.sys.lock_all.short")),
        SystemFn::KroToggle => (t!("keymap.sys.kro_toggle"), "6/NKRO".into()),
        SystemFn::Kro6 => (t!("keymap.sys.kro6"), "6KRO".into()),
        SystemFn::Nkro => (t!("keymap.sys.nkro"), "NKRO".into()),
        SystemFn::WinLockToggle => (
            t!("keymap.sys.win_lock_toggle"),
            t!("keymap.sys.win_lock_toggle.short"),
        ),
        SystemFn::WinLockOn => (
            t!("keymap.sys.win_lock_on"),
            t!("keymap.sys.win_lock_on.short"),
        ),
        SystemFn::WinLockOff => (
            t!("keymap.sys.win_lock_off"),
            t!("keymap.sys.win_lock_off.short"),
        ),
        SystemFn::OsWin => (t!("keymap.sys.os_win"), "Win OS".into()),
        SystemFn::OsMac => (t!("keymap.sys.os_mac"), "Mac OS".into()),
    };
    (full.into(), short.into())
}

fn mouse_names(buttons: MouseButtons) -> (SharedString, SharedString) {
    let (full, short) = match buttons.0 {
        1 => (t!("keymap.mouse.left"), t!("keymap.mouse.left.short")),
        2 => (t!("keymap.mouse.right"), t!("keymap.mouse.right.short")),
        4 => (t!("keymap.mouse.middle"), t!("keymap.mouse.middle.short")),
        8 => (t!("keymap.mouse.back"), t!("keymap.mouse.back.short")),
        16 => (t!("keymap.mouse.forward"), t!("keymap.mouse.forward.short")),
        _ => {
            let buttons = buttons.to_string();
            (
                t!("keymap.mouse.buttons", buttons = buttons),
                t!("keymap.mouse.buttons.short"),
            )
        }
    };
    (full.into(), short.into())
}

/// The names of an assignment: in full, and as it fits on a keycap.
fn names(action: KeyAction) -> (SharedString, SharedString) {
    let both = |text: SharedString| (text.clone(), text);
    match action {
        KeyAction::None => (
            t!("keymap.action.none").into(),
            t!("keymap.action.none.short").into(),
        ),
        KeyAction::Key(usage) => usage_names(usage),
        KeyAction::Fn => both("Fn".into()),
        KeyAction::Media(VOLUME_UP) => (
            t!("keymap.media.volume_up").into(),
            t!("keymap.media.volume_up.short").into(),
        ),
        KeyAction::Media(VOLUME_DOWN) => (
            t!("keymap.media.volume_down").into(),
            t!("keymap.media.volume_down.short").into(),
        ),
        KeyAction::Media(MUTE) => both(t!("keymap.media.mute").into()),
        KeyAction::Media(bit) => (
            t!("keymap.media.other", bit = bit).into(),
            format!("Media{bit}").into(),
        ),
        KeyAction::Macro { index, .. } => (
            t!("keymap.action.macro", slot = index).into(),
            format!("M{index}").into(),
        ),
        KeyAction::System(function) => system_names(function),
        KeyAction::Mouse(buttons) => mouse_names(buttons),
        KeyAction::Wheel(Wheel::Up) => (
            t!("keymap.mouse.wheel_up").into(),
            t!("keymap.mouse.wheel_up.short").into(),
        ),
        KeyAction::Wheel(Wheel::Down) => (
            t!("keymap.mouse.wheel_down").into(),
            t!("keymap.mouse.wheel_down.short").into(),
        ),
        KeyAction::Combo(usages) => both(combo(&usages)),
        KeyAction::CustomEvent(usage) => {
            let key = usage_names(usage).1;
            (
                t!("keymap.action.event", key = key).into(),
                format!("Ev {key}").into(),
            )
        }
        KeyAction::Advanced(slot) => (
            t!("keymap.action.advanced", slot = slot).into(),
            format!("Adv{slot}").into(),
        ),
        KeyAction::SocdToggle(pair) => (
            t!("keymap.action.socd", pair = pair).into(),
            format!("SOCD{pair}").into(),
        ),
        KeyAction::SocdToggleAll => (
            t!("keymap.action.socd_all").into(),
            SharedString::from("SOCD"),
        ),
        KeyAction::Raw(_) => both(action.to_string().into()),
    }
}

pub fn label(action: KeyAction) -> SharedString {
    names(action).0
}

pub fn short_label(action: KeyAction) -> SharedString {
    names(action).1
}

/// An assignment split into a key and the modifiers held with it, when the editor can show it
/// that way: a plain key, or a combination of up to two [`COMBO_MODIFIERS`] and one other key.
pub fn split_modifiers(action: KeyAction) -> Option<(Usage, Vec<Usage>)> {
    match action {
        KeyAction::Key(usage) => Some((usage, Vec::new())),
        KeyAction::Combo(usages) => {
            let usages: Vec<Usage> = usages.into_iter().filter(|u| *u != Usage::NONE).collect();
            let (key, modifiers) = usages.split_last()?;
            let editable = !is_modifier(*key)
                && modifiers.iter().all(|m| COMBO_MODIFIERS.contains(m))
                && modifiers.len() <= MAX_COMBO_MODIFIERS;
            editable.then(|| {
                let mut modifiers = modifiers.to_vec();
                modifiers.sort();
                modifiers.dedup();
                (*key, modifiers)
            })
        }
        _ => None,
    }
}

/// `key` held with `modifiers`, as the keyboard stores it. Modifiers beyond what fits are left
/// out.
pub fn with_modifiers(key: Usage, modifiers: &[Usage]) -> KeyAction {
    let mut modifiers: Vec<Usage> = modifiers
        .iter()
        .copied()
        .filter(|m| COMBO_MODIFIERS.contains(m))
        .collect();
    modifiers.sort();
    modifiers.dedup();
    modifiers.truncate(MAX_COMBO_MODIFIERS);
    if modifiers.is_empty() || is_modifier(key) {
        return KeyAction::Key(key);
    }
    let mut usages = [Usage::NONE; 3];
    usages[..modifiers.len()].copy_from_slice(&modifiers);
    usages[modifiers.len()] = key;
    KeyAction::Combo(usages)
}

/// Whether modifiers can be held with the plain key `key`.
pub fn accepts_modifiers(key: Usage) -> bool {
    !is_modifier(key) && key != Usage::NONE
}

#[cfg(test)]
mod tests {
    use bridge75_core::keymap::KeyEntry;

    use super::*;

    fn usage(name: &str) -> Usage {
        name.parse().unwrap()
    }

    #[test]
    fn the_picker_offers_every_named_usage_once() {
        let offered: Vec<KeyAction> = Category::ALL
            .into_iter()
            .flat_map(Category::actions)
            .collect();
        for (usage, name) in Usage::all() {
            let count = offered
                .iter()
                .filter(|a| **a == KeyAction::Key(usage))
                .count();
            assert_eq!(count, 1, "{name}");
        }
        let mut sorted = offered.clone();
        sorted.sort_by_key(|a| KeyEntry::from(*a).0);
        sorted.dedup();
        assert_eq!(sorted.len(), offered.len());
    }

    #[test]
    fn every_offered_assignment_has_a_name() {
        for action in Category::ALL.into_iter().flat_map(Category::actions) {
            let (full, short) = names(action);
            assert!(!full.is_empty() && !short.is_empty(), "{action}");
            assert!(!full.starts_with("keymap."), "{action}: {full}");
            assert!(!short.starts_with("keymap."), "{action}: {short}");
        }
    }

    #[test]
    fn keys_and_combinations_read_like_their_keycaps() {
        assert_eq!(short_label(KeyAction::Key(usage("a"))), "A");
        assert_eq!(short_label(KeyAction::Key(usage("0"))), "0");
        assert_eq!(short_label(KeyAction::Key(usage("f13"))), "F13");
        assert_eq!(short_label(KeyAction::Key(usage("kp-1"))), "Num1");
        assert_eq!(short_label(KeyAction::Key(usage("rshift"))), "RShift");
        let copy = "combo:lctrl+c".parse().unwrap();
        assert_eq!(label(copy), "Ctrl+C");
        let task_manager = "combo:lctrl+lshift+esc".parse().unwrap();
        assert_eq!(label(task_manager), "Ctrl+Shift+Esc");
    }

    #[test]
    fn combinations_split_into_a_key_and_its_modifiers() {
        let split = |text: &str| split_modifiers(text.parse().unwrap());
        assert_eq!(split("a"), Some((usage("a"), vec![])));
        assert_eq!(
            split("combo:lshift+lctrl+c"),
            Some((usage("c"), vec![usage("lctrl"), usage("lshift")]))
        );
        assert_eq!(split("combo:c"), Some((usage("c"), vec![])));
        // Right modifiers, or nothing but modifiers, are not offered by the editor.
        assert_eq!(split("combo:rctrl+c"), None);
        assert_eq!(split("combo:lctrl+lshift"), None);
        assert_eq!(split("fn"), None);
    }

    #[test]
    fn modifiers_make_a_combination_of_at_most_three_usages() {
        let (ctrl, shift, alt) = (usage("lctrl"), usage("lshift"), usage("lalt"));
        assert_eq!(with_modifiers(usage("c"), &[]), KeyAction::Key(usage("c")));
        assert_eq!(
            with_modifiers(usage("c"), &[shift, ctrl]),
            "combo:lctrl+lshift+c".parse().unwrap()
        );
        assert_eq!(
            with_modifiers(usage("c"), &[ctrl, shift, alt]),
            "combo:lctrl+lshift+c".parse().unwrap()
        );
        // A modifier key is not combined.
        assert_eq!(
            with_modifiers(usage("lalt"), &[ctrl]),
            KeyAction::Key(usage("lalt"))
        );
        for action in [
            with_modifiers(usage("c"), &[ctrl]),
            with_modifiers(usage("v"), &[ctrl, alt]),
        ] {
            assert_eq!(KeyAction::from(KeyEntry::from(action)), action);
        }
    }
}
