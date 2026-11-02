//! Debounce, polling rate and sleep time as last read from or written to the keyboard. No GPUI.

use bridge75_core::he::{Debounce, PollingRate};
use bridge75_core::info::SleepTimes;

/// Sleep times offered in the menu, in seconds. The keyboard sleeps after 30 minutes unless
/// told otherwise.
pub const SLEEP_PRESETS: [u16; 6] = [300, 600, 900, 1800, 3600, 7200];

/// One of the settings, which are written one at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Setting {
    Debounce,
    PollingRate,
    Sleep,
}

impl Setting {
    /// How many settings there are.
    pub const COUNT: usize = 3;

    /// Position among the settings, below [`Setting::COUNT`].
    pub fn ix(self) -> usize {
        self as usize
    }
}

/// A new value of one setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Debounce(Debounce),
    PollingRate(PollingRate),
    Sleep(SleepTimes),
}

impl Change {
    pub fn setting(self) -> Setting {
        match self {
            Change::Debounce(_) => Setting::Debounce,
            Change::PollingRate(_) => Setting::PollingRate,
            Change::Sleep(_) => Setting::Sleep,
        }
    }
}

/// The settings of one connected keyboard. Small enough to copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GeneralSettings {
    pub debounce: Debounce,
    pub polling_rate: PollingRate,
    pub sleep: SleepTimes,
}

impl GeneralSettings {
    /// What the firmware starts with.
    pub const DEFAULT: GeneralSettings = GeneralSettings {
        debounce: Debounce::DEFAULT,
        polling_rate: PollingRate::Khz8,
        sleep: SleepTimes::DEFAULT,
    };

    /// Takes `change`. Returns whether anything changed.
    pub fn apply(&mut self, change: Change) -> bool {
        let before = *self;
        match change {
            Change::Debounce(debounce) => self.debounce = debounce,
            Change::PollingRate(rate) => self.polling_rate = rate,
            Change::Sleep(sleep) => self.sleep = sleep,
        }
        *self != before
    }

    /// The debounce `delta` steps away, if it is in range.
    pub fn stepped_debounce(&self, delta: i16) -> Option<Debounce> {
        let value = i16::from(self.debounce.value()) + delta;
        u8::try_from(value).ok().and_then(|v| Debounce::new(v).ok())
    }

    /// The sleep times with the keyboard sleeping after `seconds`.
    ///
    /// The lighting must not sleep later than the keyboard, so it is brought forward when it
    /// would. This keyboard has no lighting, so that changes nothing it does.
    pub fn sleep_after(&self, seconds: u16) -> SleepTimes {
        SleepTimes {
            light: self.sleep.light.min(seconds),
            keyboard: seconds,
        }
    }
}

/// The sleep times the menu offers: the presets, and `current` among them when the keyboard
/// has a time set elsewhere.
pub fn sleep_choices(current: u16) -> Vec<u16> {
    let mut choices = SLEEP_PRESETS.to_vec();
    if !choices.contains(&current) {
        choices.push(current);
        choices.sort_unstable();
    }
    choices
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> GeneralSettings {
        GeneralSettings::DEFAULT
    }

    #[test]
    fn debounce_steps_stay_in_range() {
        let mut settings = settings();
        assert_eq!(settings.stepped_debounce(1).unwrap().value(), 4);
        settings.apply(Change::Debounce(Debounce::new(0).unwrap()));
        assert_eq!(settings.stepped_debounce(-1), None);
        settings.apply(Change::Debounce(Debounce::new(Debounce::MAX).unwrap()));
        assert_eq!(settings.stepped_debounce(1), None);
        assert_eq!(
            settings.stepped_debounce(-1).unwrap().value(),
            Debounce::MAX - 1
        );
    }

    #[test]
    fn the_lighting_never_sleeps_after_the_keyboard() {
        let mut settings = settings();
        assert_eq!(settings.sleep_after(600).light, 60);
        settings.apply(Change::Sleep(SleepTimes {
            light: 1200,
            keyboard: 1800,
        }));
        let sleep = settings.sleep_after(600);
        assert_eq!((sleep.light, sleep.keyboard), (600, 600));
        assert!(sleep.validate().is_ok());
    }

    #[test]
    fn applying_the_same_value_changes_nothing() {
        let mut settings = settings();
        assert!(!settings.apply(Change::PollingRate(PollingRate::Khz8)));
        assert!(settings.apply(Change::PollingRate(PollingRate::Khz1)));
        assert_eq!(settings.polling_rate, PollingRate::Khz1);
    }

    #[test]
    fn a_time_set_elsewhere_joins_the_presets_in_order() {
        assert_eq!(sleep_choices(1800), SLEEP_PRESETS);
        assert_eq!(sleep_choices(1000), [300, 600, 900, 1000, 1800, 3600, 7200]);
        assert!(SLEEP_PRESETS.contains(&SleepTimes::DEFAULT.keyboard));
        assert!(SLEEP_PRESETS.windows(2).all(|w| w[0] < w[1]));
    }
}
