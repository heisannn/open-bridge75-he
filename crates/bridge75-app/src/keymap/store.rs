//! The keymap this app last wrote to each keyboard, kept on this computer because the keyboard
//! can't report it. No GPUI.
//!
//! Each keyboard has one profile file, `<config dir>/open-bridge75-he/keyboards/<serial>.toml`,
//! that `bridge75 profile apply` can also read. Saving replaces only its keymap, so the other
//! sections stay as they are.

use std::io;
use std::path::{Path, PathBuf};

use bridge75_core::profile::{KeymapProfile, Profile};

/// The file of a keyboard without a usable serial number.
const UNKNOWN_KEYBOARD: &str = "unknown";

/// Names the file of the keyboard with `serial`. Only letters, digits, `-` and `_` are kept,
/// so the serial number can't point outside the directory.
pub fn keyboard_id(serial: Option<&str>) -> String {
    let id: String = serial
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
        .collect();
    if id.is_empty() {
        UNKNOWN_KEYBOARD.to_owned()
    } else {
        id
    }
}

#[derive(Clone, Debug)]
pub struct KeymapStore {
    /// `None` if the system has no config directory; nothing is kept then.
    dir: Option<PathBuf>,
}

impl KeymapStore {
    pub fn new() -> Self {
        KeymapStore {
            dir: dirs::config_dir().map(|dir| dir.join("open-bridge75-he").join("keyboards")),
        }
    }

    /// A store in `dir`, for tests.
    #[cfg(test)]
    pub fn at(dir: impl Into<PathBuf>) -> Self {
        KeymapStore {
            dir: Some(dir.into()),
        }
    }

    /// A store that keeps nothing, for tests.
    #[cfg(test)]
    pub fn none() -> Self {
        KeymapStore { dir: None }
    }

    fn path(&self, keyboard: &str) -> Option<PathBuf> {
        Some(self.dir.as_ref()?.join(format!("{keyboard}.toml")))
    }

    fn read(path: &Path) -> io::Result<Option<Profile>> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        Profile::from_toml(&text)
            .map(Some)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
    }

    /// The keymap last written to `keyboard`, or `None` if this app never wrote one.
    pub fn load(&self, keyboard: &str) -> io::Result<Option<KeymapProfile>> {
        let Some(path) = self.path(keyboard) else {
            return Ok(None);
        };
        let profile = Self::read(&path)
            .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
        Ok(profile.and_then(|p| p.keymap))
    }

    /// Records `keymap` as written to `keyboard`.
    pub fn save(&self, keyboard: &str, keymap: &KeymapProfile) -> io::Result<()> {
        let path = self
            .path(keyboard)
            .ok_or_else(|| io::Error::other("no config directory"))?;
        let mut profile = Self::read(&path)?.unwrap_or_default();
        profile.keymap = Some(keymap.clone());
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        // Written beside the file and moved over it, so an interrupted save leaves the
        // previous file intact.
        let partial = path.with_extension("toml.partial");
        std::fs::write(&partial, profile.to_toml())?;
        std::fs::rename(&partial, &path)
    }
}

#[cfg(test)]
mod tests {
    use bridge75_core::keymap::KeyAction;
    use bridge75_core::matrix::KeyPos;

    use super::*;

    /// A fresh directory under the system temp directory, removed when dropped.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("bridge75-app-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            TempDir(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn keymap() -> KeymapProfile {
        let mut keymap = KeymapProfile::default();
        let caps: KeyPos = "caps".parse().unwrap();
        keymap
            .base
            .insert(caps, "lctrl".parse::<KeyAction>().unwrap());
        keymap
    }

    #[test]
    fn serial_numbers_become_safe_file_names() {
        assert_eq!(keyboard_id(Some("75AL-HE_0042")), "75AL-HE_0042");
        assert_eq!(keyboard_id(Some("../../etc")), "etc");
        assert_eq!(keyboard_id(Some(" ")), UNKNOWN_KEYBOARD);
        assert_eq!(keyboard_id(None), UNKNOWN_KEYBOARD);
    }

    #[test]
    fn a_saved_keymap_is_loaded_again() {
        let dir = TempDir::new("store-round-trip");
        let store = KeymapStore::at(&dir.0);
        assert_eq!(store.load("kb").unwrap(), None);
        store.save("kb", &keymap()).unwrap();
        assert_eq!(store.load("kb").unwrap(), Some(keymap()));
        assert_eq!(store.load("other").unwrap(), None);
    }

    #[test]
    fn saving_keeps_the_other_sections_of_the_profile() {
        let dir = TempDir::new("store-other-sections");
        std::fs::create_dir_all(&dir.0).unwrap();
        std::fs::write(dir.0.join("kb.toml"), "debounce = 3\n").unwrap();
        let store = KeymapStore::at(&dir.0);
        store.save("kb", &keymap()).unwrap();
        let text = std::fs::read_to_string(dir.0.join("kb.toml")).unwrap();
        let profile = Profile::from_toml(&text).unwrap();
        assert_eq!(profile.debounce.map(|d| d.value()), Some(3));
        assert_eq!(profile.keymap, Some(keymap()));
        assert!(!dir.0.join("kb.toml.partial").exists());
    }

    #[test]
    fn a_broken_file_is_reported_and_not_overwritten() {
        let dir = TempDir::new("store-broken");
        std::fs::create_dir_all(&dir.0).unwrap();
        std::fs::write(dir.0.join("kb.toml"), "keymap = 1\n").unwrap();
        let store = KeymapStore::at(&dir.0);
        let error = store.load("kb").unwrap_err();
        assert!(error.to_string().contains("kb.toml"), "{error}");
        assert!(store.save("kb", &keymap()).is_err());
        let text = std::fs::read_to_string(dir.0.join("kb.toml")).unwrap();
        assert_eq!(text, "keymap = 1\n");
    }

    #[test]
    fn without_a_config_directory_nothing_is_kept() {
        let store = KeymapStore::none();
        assert_eq!(store.load("kb").unwrap(), None);
        assert!(store.save("kb", &keymap()).is_err());
    }
}
