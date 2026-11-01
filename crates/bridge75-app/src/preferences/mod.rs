//! Settings of the app itself, stored in `<config dir>/open-bridge75-he/preferences.toml`.

mod page;

use std::path::PathBuf;

use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Global, Window};
use serde::{Deserialize, Serialize};

pub use page::PreferencesPage;

/// Locales that have translations in `locales/app.yml`.
const LOCALES: [&str; 2] = ["en", "ja"];

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Language {
    #[default]
    System,
    English,
    Japanese,
}

impl Language {
    pub const ALL: [Language; 3] = [Language::System, Language::English, Language::Japanese];

    pub fn locale(self) -> &'static str {
        match self {
            Language::System => sys_locale::get_locale().map_or("en", |tag| system_locale(&tag)),
            Language::English => "en",
            Language::Japanese => "ja",
        }
    }
}

/// The translated locale for a BCP 47 tag such as `ja-JP`. Falls back to English.
fn system_locale(tag: &str) -> &'static str {
    let language = tag.split(['-', '_']).next().unwrap_or_default();
    LOCALES
        .into_iter()
        .find(|l| l.eq_ignore_ascii_case(language))
        .unwrap_or("en")
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Appearance {
    #[default]
    System,
    Light,
    Dark,
}

impl Appearance {
    pub const ALL: [Appearance; 3] = [Appearance::System, Appearance::Light, Appearance::Dark];
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub language: Language,
    pub appearance: Appearance,
}

impl Global for Preferences {}

impl Preferences {
    fn path() -> Option<PathBuf> {
        Some(
            dirs::config_dir()?
                .join("open-bridge75-he")
                .join("preferences.toml"),
        )
    }

    /// Missing or unreadable files give the defaults.
    fn load() -> Self {
        Self::path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|text| toml::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn save(&self) -> std::io::Result<()> {
        let path = Self::path().ok_or_else(|| std::io::Error::other("no config directory"))?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let text = toml::to_string(self).map_err(std::io::Error::other)?;
        std::fs::write(path, text)
    }

    pub fn global(cx: &App) -> &Self {
        cx.global::<Self>()
    }

    /// Changes the stored preferences, applies them to every window and saves them.
    pub fn update(
        cx: &mut App,
        window: &mut Window,
        edit: impl FnOnce(&mut Preferences),
    ) -> std::io::Result<()> {
        let mut preferences = Self::global(cx).clone();
        edit(&mut preferences);
        rust_i18n::set_locale(preferences.language.locale());
        apply_appearance(preferences.appearance, Some(window), cx);
        cx.set_global(preferences.clone());
        cx.refresh_windows();
        preferences.save()
    }
}

/// Loads the preferences and applies the language. Call after `gpui_kit::init`.
pub fn init(cx: &mut App) {
    let preferences = Preferences::load();
    rust_i18n::set_locale(preferences.language.locale());
    apply_appearance(preferences.appearance, None, cx);
    cx.set_global(preferences);
}

pub fn apply_appearance(appearance: Appearance, window: Option<&mut Window>, cx: &mut App) {
    match appearance {
        Appearance::System => Theme::sync_system_appearance(window, cx),
        Appearance::Light => Theme::change(ThemeMode::Light, window, cx),
        Appearance::Dark => Theme::change(ThemeMode::Dark, window, cx),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_locale_matches_the_language_subtag() {
        assert_eq!(system_locale("ja-JP"), "ja");
        assert_eq!(system_locale("JA"), "ja");
        assert_eq!(system_locale("en-US"), "en");
        assert_eq!(system_locale("fr-FR"), "en");
        assert_eq!(system_locale(""), "en");
    }

    #[test]
    fn preferences_round_trip_and_ignore_unknown_fields() {
        let preferences = Preferences {
            language: Language::Japanese,
            appearance: Appearance::Dark,
        };
        let text = toml::to_string(&preferences).unwrap();
        assert_eq!(toml::from_str::<Preferences>(&text).unwrap(), preferences);
        let older: Preferences = toml::from_str("language = \"english\"\nfuture = 1\n").unwrap();
        assert_eq!(older.language, Language::English);
        assert_eq!(older.appearance, Appearance::System);
    }
}
