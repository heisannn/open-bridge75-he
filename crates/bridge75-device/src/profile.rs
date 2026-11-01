//! Reading and writing profiles.

use bridge75_core::info::SleepTimes;
use bridge75_core::matrix::LAYOUT;
use bridge75_core::profile::{Profile, SocdProfile, TriggerGroup};

use crate::{Device, Result};

impl Device {
    /// Builds a profile from the settings that can be read from the keyboard.
    ///
    /// The keymap, macros and advanced keys cannot be read back, so they are not included.
    /// Sleep time is included only if it is not the default (lighting 60 s / keyboard 1800 s). FW also
    /// reports a stored 0 as the default, so writing out the default would change the stored value when written back.
    /// SOCD is included only when it is configured (some pair is non-empty, or global is on).
    /// The per-pair enabled flags cannot be written, so they are not included (writing back aligns them with global on/off).
    pub fn read_profile(&self, progress: &mut dyn FnMut(&str)) -> Result<Profile> {
        progress("debounce and polling rate");
        let debounce = self.debounce()?;
        let polling_rate = self.polling_rate()?;
        progress("sleep time");
        let sleep = Some(self.sleep_times()?).filter(|s| *s != SleepTimes::DEFAULT);
        progress("trigger settings (81 keys)");
        let triggers = self.triggers(LAYOUT.iter().map(|k| k.pos))?;
        progress("SOCD");
        let pairs = self.socd_pairs()?;
        let status = self.socd_status()?;
        let socd = (status.global || pairs.iter().any(|p| !p.is_empty()))
            .then(|| SocdProfile::from_pairs(&pairs, Some(status.global)));
        Ok(Profile {
            debounce: Some(debounce),
            polling_rate: Some(polling_rate),
            sleep,
            trigger: TriggerGroup::compact(&triggers),
            socd,
            ..Profile::default()
        })
    }

    /// Writes only the items present in the profile. Validates every value before writing anything.
    pub fn apply_profile(&self, profile: &Profile, progress: &mut dyn FnMut(&str)) -> Result<()> {
        profile.validate()?;
        if let Some(debounce) = profile.debounce {
            progress("debounce");
            self.set_debounce(debounce)?;
        }
        if let Some(rate) = profile.polling_rate {
            progress("polling rate");
            self.set_polling_rate(rate)?;
        }
        if let Some(sleep) = profile.sleep {
            progress("sleep time");
            self.set_sleep_times(sleep)?;
        }
        for (i, group) in profile.trigger.iter().enumerate() {
            progress(&format!(
                "trigger settings {}/{}",
                i + 1,
                profile.trigger.len()
            ));
            self.set_trigger(&group.bitmap(), &group.config())?;
        }
        if let Some(keymap) = &profile.keymap {
            progress("keymap (base / Fn)");
            let (base, fn_layer) = keymap.build();
            self.write_keymaps(&base, &fn_layer)?;
        }
        if let Some(slots) = profile.macro_slots() {
            progress("macros (10 slots)");
            self.write_macros(&slots)?;
        }
        if let Some(slots) = profile.advanced_slots() {
            progress("advanced keys (40 slots)");
            self.write_advanced_keys(&slots)?;
        }
        if let Some(socd) = &profile.socd {
            progress("SOCD");
            self.write_socd(&socd.pairs())?;
            if let Some(global) = socd.global {
                self.set_socd_global(global)?;
            }
        }
        Ok(())
    }
}
