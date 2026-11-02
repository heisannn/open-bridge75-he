//! Desktop configurator for the Bridge75 HE.
//!
//! Each module is one capability with its state and views. The [`shell`] composes them into the
//! window; [`session`] owns the connection to the keyboard; [`document`] holds the keyboard's
//! settings that the pages edit. The keymap can't be read from the keyboard, so [`keymap`] keeps
//! its own document of what the app wrote, stored on this computer.

rust_i18n::i18n!("locales", fallback = "en");

mod assets;
mod document;
mod keyboard;
mod keymap;
mod monitor;
mod overview;
mod page;
mod preferences;
mod session;
mod shell;
mod socd;
mod trigger;

use gpui_kit::component::TitleBar;
use gpui_kit::{
    App, AppContext as _, KeyBinding, Menu, MenuItem, WindowBounds, WindowOptions, actions, px,
    size,
};

actions!(
    app,
    [
        /// Quit the app.
        Quit,
    ]
);

/// Lets the app quit with cmd-q on macOS, as macOS apps do. Elsewhere, closing the window
/// quits it.
fn init_quit(cx: &mut App) {
    cx.on_action(|_: &Quit, cx| cx.quit());
    if !cx.platform_info().is_mac() {
        return;
    }
    cx.bind_keys([KeyBinding::new("cmd-q", Quit, None)]);
    set_app_menu(cx);
    // The menu follows the language chosen in the preferences.
    cx.observe_global::<preferences::Preferences>(set_app_menu)
        .detach();
}

/// The menu of the app in the macOS menu bar, which shows its shortcut too.
fn set_app_menu(cx: &mut App) {
    cx.set_menus([Menu {
        name: rust_i18n::t!("app.title").into(),
        items: vec![MenuItem::action(rust_i18n::t!("app.quit"), Quit)],
        disabled: false,
    }]);
}

/// Opens the main window and runs until it is closed.
pub fn run() {
    gpui_kit::application()
        .with_assets(assets::AppAssets)
        .run(|cx| {
            gpui_kit::init(cx);
            preferences::init(cx);
            shell::init(cx);
            trigger::init(cx);
            keymap::init(cx);
            socd::init(cx);
            init_quit(cx);

            let session = session::new(cx);
            let document = document::new(session.clone(), cx);
            let keymap = keymap::new(session.clone(), cx);
            let socd = socd::new(session.clone(), cx);
            // Window geometry is a platform boundary, so it is given in pixels.
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::centered(size(px(1180.), px(800.)), cx)),
                window_min_size: Some(size(shell::MIN_WINDOW_WIDTH, px(640.))),
                ..TitleBar::window_options()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                window.set_window_title(&rust_i18n::t!("app.title"));
                cx.new(|cx| shell::Shell::new(session, document, keymap, socd, window, cx))
            })
            .expect("failed to open the main window");

            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            cx.activate(true);
        });
}

#[cfg(test)]
mod tests {
    #[gpui_kit::test]
    fn cmd_q_quits_on_macos(cx: &mut gpui_kit::TestAppContext) {
        cx.update(|cx| {
            super::init_quit(cx);
            let keystroke = gpui_kit::Keystroke::parse("cmd-q").unwrap();
            let bindings = cx.all_bindings_for_input(&[keystroke]);
            if cx.platform_info().is_mac() {
                assert_eq!(bindings.len(), 1);
                assert!(bindings[0].action().partial_eq(&super::Quit));
            } else {
                assert!(bindings.is_empty());
            }
        });
    }

    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;

    /// The texts of one key, by locale.
    type Texts = BTreeMap<String, String>;

    /// Reads `locales/app.yml`, failing on a key or locale that appears twice.
    fn translations() -> BTreeMap<String, Texts> {
        let source = include_str!("../locales/app.yml");
        let mut entries: BTreeMap<String, Texts> = BTreeMap::new();
        let mut key: Option<String> = None;
        for line in source.lines() {
            if line.trim().is_empty()
                || line.trim_start().starts_with('#')
                || line.starts_with("_version")
            {
                continue;
            }
            if let Some(text) = line.strip_prefix("  ") {
                let (locale, value) = text.split_once(':').expect(line);
                let value = value.trim();
                let value = value
                    .strip_prefix('"')
                    .and_then(|v| v.strip_suffix('"'))
                    .unwrap_or(value);
                let key = key.as_ref().expect("a text belongs to a key");
                let texts = entries.get_mut(key).unwrap();
                let previous = texts.insert(locale.to_owned(), value.to_owned());
                assert!(previous.is_none(), "{key} has {locale} twice");
            } else {
                let name = line.strip_suffix(':').expect(line).to_owned();
                assert!(!entries.contains_key(&name), "{name} appears twice");
                entries.insert(name.clone(), Texts::new());
                key = Some(name);
            }
        }
        entries
    }

    /// Adds the keys passed to `t!` in the Rust files under `dir`.
    fn used_keys(dir: &Path, keys: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                used_keys(&path, keys);
                continue;
            }
            if path.extension().is_none_or(|e| e != "rs") {
                continue;
            }
            let source = std::fs::read_to_string(&path).unwrap();
            for (start, _) in source.match_indices("t!(") {
                let rest = source[start + 3..].trim_start();
                let Some(literal) = rest.strip_prefix('"') else {
                    continue;
                };
                let key = &literal[..literal.find('"').unwrap()];
                // Format strings such as "{} {}" are not keys.
                if !key.contains('{') {
                    keys.insert(key.to_owned());
                }
            }
        }
    }

    fn placeholders(text: &str) -> BTreeSet<&str> {
        text.split("%{")
            .skip(1)
            .filter_map(|rest| rest.split_once('}').map(|(name, _)| name))
            .collect()
    }

    fn has_cjk(text: &str) -> bool {
        text.chars().any(|c| {
            matches!(c, '\u{3000}'..='\u{30ff}' | '\u{3400}'..='\u{9fff}' | '\u{ff00}'..='\u{ffef}')
        })
    }

    #[test]
    fn every_text_is_translated() {
        for (key, texts) in translations() {
            let locales: Vec<&str> = texts.keys().map(String::as_str).collect();
            assert_eq!(locales, ["en", "ja"], "{key}");
            assert!(texts.values().all(|t| !t.is_empty()), "{key} is empty");
        }
    }

    #[test]
    fn every_key_in_use_has_texts() {
        let mut keys = BTreeSet::new();
        used_keys(
            &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut keys,
        );
        assert!(keys.contains("app.title"), "the scan found no keys");
        let translations = translations();
        for key in keys {
            assert!(translations.contains_key(&key), "{key} is not in app.yml");
        }
    }

    #[test]
    fn translations_keep_the_placeholders() {
        for (key, texts) in translations() {
            assert_eq!(
                placeholders(&texts["en"]),
                placeholders(&texts["ja"]),
                "{key}"
            );
        }
    }

    #[test]
    fn english_texts_are_english() {
        for (key, texts) in translations() {
            // Language names are written in their own language.
            if key.starts_with("preferences.language.") {
                continue;
            }
            assert!(!has_cjk(&texts["en"]), "{key}");
        }
    }

    #[test]
    fn texts_use_the_ellipsis_character() {
        for (key, texts) in translations() {
            assert!(texts.values().all(|t| !t.contains("...")), "{key}");
        }
    }
}
