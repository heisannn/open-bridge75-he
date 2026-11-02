use gpui_kit::SharedString;
use gpui_kit::assets::IconName;
use rust_i18n::t;

/// A page in the navigation. The order of [`Page::ALL`] is the order in the sidebar.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Overview,
    Trigger,
    Keymap,
    Socd,
    Monitor,
    Preferences,
}

impl Page {
    pub const ALL: [Page; 6] = [
        Page::Overview,
        Page::Trigger,
        Page::Keymap,
        Page::Socd,
        Page::Monitor,
        Page::Preferences,
    ];

    /// Position in [`Page::ALL`].
    pub fn ix(self) -> usize {
        self as usize
    }

    pub fn label(self) -> SharedString {
        match self {
            Page::Overview => t!("nav.overview"),
            Page::Trigger => t!("nav.trigger"),
            Page::Keymap => t!("nav.keymap"),
            Page::Socd => t!("nav.socd"),
            Page::Monitor => t!("nav.monitor"),
            Page::Preferences => t!("nav.preferences"),
        }
        .into()
    }

    pub fn icon(self) -> IconName {
        match self {
            Page::Overview => IconName::Keyboard,
            Page::Trigger => IconName::Gauge,
            Page::Keymap => IconName::Command,
            Page::Socd => IconName::ArrowLeftRight,
            Page::Monitor => IconName::Activity,
            Page::Preferences => IconName::Settings,
        }
    }

    /// Whether the page lays out its own scrolling instead of scrolling as a whole.
    pub fn scrolls_itself(self) -> bool {
        matches!(self, Page::Trigger | Page::Keymap)
    }
}
