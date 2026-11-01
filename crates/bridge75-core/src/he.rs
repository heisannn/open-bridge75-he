//! Value types for HE (magnetic switch) settings (wire 0x21).

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Maximum travel (0.1mm units).
pub const MAX_TRAVEL: u8 = 36;

/// Travel (0.1mm units, 0..=36).
///
/// Strings and profiles express it in mm (`1.6`, `1.6mm`).
#[derive(
    Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Default, Serialize, Deserialize,
)]
#[serde(try_from = "f64", into = "f64")]
pub struct Travel(u8);

impl Travel {
    pub const MAX: Travel = Travel(MAX_TRAVEL);

    pub fn new(raw: u8) -> Result<Self> {
        if raw <= MAX_TRAVEL {
            Ok(Travel(raw))
        } else {
            Err(Error::OutOfRange(format!(
                "travel must be 0 to 3.6mm (got {raw} x 0.1mm)"
            )))
        }
    }

    /// Creates without checking the range. Used for values received from the keyboard.
    pub const fn from_raw(raw: u8) -> Self {
        Travel(raw)
    }

    pub const fn raw(self) -> u8 {
        self.0
    }

    pub fn from_mm(mm: f64) -> Result<Self> {
        let tenths = (mm * 10.0).round();
        if !(0.0..=MAX_TRAVEL as f64).contains(&tenths) {
            return Err(Error::OutOfRange(format!(
                "travel must be 0 to 3.6mm (got {mm}mm)"
            )));
        }
        if (mm * 10.0 - tenths).abs() > 1e-6 {
            return Err(Error::Invalid(format!(
                "travel must be in 0.1mm steps (got {mm}mm)"
            )));
        }
        Ok(Travel(tenths as u8))
    }

    pub fn mm(self) -> f64 {
        self.0 as f64 / 10.0
    }
}

impl fmt::Display for Travel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.0 / 10, self.0 % 10)
    }
}

impl FromStr for Travel {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let s = s.trim();
        let number = s.strip_suffix("mm").unwrap_or(s).trim();
        let mm: f64 = number
            .parse()
            .map_err(|_| Error::Invalid(format!("travel must be given in mm: {s}")))?;
        Travel::from_mm(mm)
    }
}

impl TryFrom<f64> for Travel {
    type Error = Error;

    fn try_from(mm: f64) -> Result<Self> {
        Travel::from_mm(mm)
    }
}

impl From<Travel> for f64 {
    fn from(t: Travel) -> f64 {
        t.mm()
    }
}

/// Trigger mode (the sub0 / 1 / 12 / 13 subcommand value itself).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TriggerMode {
    /// Press / release at the actuation point (QuickTrigger OFF in the official configurator).
    Normal = 0,
    /// Press at the actuation point, release at the release point.
    Split = 1,
    /// Rapid Trigger. RT ends when the key returns to the actuation point (QuickTrigger ON).
    Rapid = 12,
    /// Rapid Trigger. The end of RT is judged at the release point (QuickTrigger + SecondarySensitivity).
    RapidSplit = 13,
}

impl TriggerMode {
    pub const ALL: [TriggerMode; 4] = [
        TriggerMode::Normal,
        TriggerMode::Split,
        TriggerMode::Rapid,
        TriggerMode::RapidSplit,
    ];

    pub fn from_raw(raw: u8) -> Option<Self> {
        Self::ALL.into_iter().find(|m| *m as u8 == raw)
    }

    pub fn name(self) -> &'static str {
        match self {
            TriggerMode::Normal => "normal",
            TriggerMode::Split => "split",
            TriggerMode::Rapid => "rapid",
            TriggerMode::RapidSplit => "rapid-split",
        }
    }

    pub fn is_rapid(self) -> bool {
        matches!(self, TriggerMode::Rapid | TriggerMode::RapidSplit)
    }

    /// Whether this mode uses the release point (`b[30]`) separately from the actuation point.
    pub fn uses_release_point(self) -> bool {
        matches!(self, TriggerMode::Split | TriggerMode::RapidSplit)
    }
}

impl fmt::Display for TriggerMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for TriggerMode {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let lower = s.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|m| m.name() == lower)
            .ok_or_else(|| {
                Error::Invalid(format!(
                    "unknown mode: {s} (normal / split / rapid / rapid-split)"
                ))
            })
    }
}

/// Trigger settings for one key.
///
/// - RT sensitivity is an "exceeds" check, so it actually reacts once the key has moved 0.1mm more than the setting.
/// - Changes of 0.2mm or less in the press direction are ignored, so an actuation point of 0.1 / 0.2mm acts as 0.3mm.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct TriggerConfig {
    pub mode: TriggerMode,
    /// Actuation point (`b[29]`).
    pub actuation: Travel,
    /// Release point / RT exit point (`b[30]`). For `normal` / `rapid`, the same value as the actuation point is sent.
    pub release: Travel,
    /// RT press sensitivity (`b[31]`).
    pub rt_press: Travel,
    /// RT release sensitivity (`b[32]`).
    pub rt_release: Travel,
}

impl TriggerConfig {
    /// The official configurator's initial RT sensitivity (0.5mm).
    pub const DEFAULT_RT: Travel = Travel(5);

    /// FW default (mode 0, 1.8mm). The FW does not initialize the RT sensitivity, so the official configurator's initial value is used.
    pub const DEFAULT: TriggerConfig = TriggerConfig {
        mode: TriggerMode::Normal,
        actuation: Travel(18),
        release: Travel(18),
        rt_press: Self::DEFAULT_RT,
        rt_release: Self::DEFAULT_RT,
    };

    /// Builds from `b[7..=11]` of the sub5 response (mode, actuation point, release point, RT press, RT release).
    pub fn from_wire(bytes: [u8; 5]) -> Result<Self> {
        let [mode, actuation, release, rt_press, rt_release] = bytes;
        let mode = TriggerMode::from_raw(mode).ok_or_else(|| {
            Error::Invalid(format!(
                "the keyboard returned an unknown mode value {mode}"
            ))
        })?;
        Ok(TriggerConfig {
            mode,
            actuation: Travel::from_raw(actuation),
            release: Travel::from_raw(release),
            rt_press: Travel::from_raw(rt_press),
            rt_release: Travel::from_raw(rt_release),
        })
    }

    pub fn validate(&self) -> Result<()> {
        for t in [self.actuation, self.release, self.rt_press, self.rt_release] {
            Travel::new(t.raw())?;
        }
        Ok(())
    }
}

impl Default for TriggerConfig {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// Polling rate (sub9). The value only takes effect in the hidden polling rate test mode (Fn+O+K).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub enum PollingRate {
    Khz1 = 1,
    Khz2 = 2,
    Khz4 = 4,
    Khz8 = 8,
}

impl PollingRate {
    pub fn khz(self) -> u8 {
        self as u8
    }
}

impl TryFrom<u8> for PollingRate {
    type Error = Error;

    fn try_from(khz: u8) -> Result<Self> {
        match khz {
            1 => Ok(PollingRate::Khz1),
            2 => Ok(PollingRate::Khz2),
            4 => Ok(PollingRate::Khz4),
            8 => Ok(PollingRate::Khz8),
            _ => Err(Error::OutOfRange(format!(
                "polling rate must be 1 / 2 / 4 / 8 kHz (got {khz})"
            ))),
        }
    }
}

impl From<PollingRate> for u8 {
    fn from(rate: PollingRate) -> u8 {
        rate.khz()
    }
}

impl FromStr for PollingRate {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        let s = s.trim().to_ascii_lowercase();
        let number = s.strip_suffix("khz").unwrap_or(&s).trim();
        let khz: u8 = number
            .parse()
            .map_err(|_| Error::Invalid(format!("polling rate: {s}")))?;
        khz.try_into()
    }
}

impl fmt::Display for PollingRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}kHz", self.khz())
    }
}

/// Debounce (sub10, AntiShake in the official configurator).
///
/// A change in the same direction is accepted once it lasts more than (value + 3) frames. Default 3.
/// The FW does not check the range, but at 252 or more the travel is almost never updated, so such values are rejected.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(try_from = "u8", into = "u8")]
pub struct Debounce(u8);

impl Debounce {
    pub const MAX: u8 = 251;
    pub const DEFAULT: Debounce = Debounce(3);

    pub fn new(value: u8) -> Result<Self> {
        if value <= Self::MAX {
            Ok(Debounce(value))
        } else {
            Err(Error::OutOfRange(format!(
                "debounce must be 0 to {} (got {value})",
                Self::MAX
            )))
        }
    }

    pub fn value(self) -> u8 {
        self.0
    }
}

impl TryFrom<u8> for Debounce {
    type Error = Error;

    fn try_from(value: u8) -> Result<Self> {
        Debounce::new(value)
    }
}

impl From<Debounce> for u8 {
    fn from(d: Debounce) -> u8 {
        d.0
    }
}

/// Status of single-key calibration (sub7).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum KeyCalibration {
    Done,
    Running,
    TimedOut,
    Other(u8),
}

impl From<u8> for KeyCalibration {
    fn from(v: u8) -> Self {
        match v {
            0 => KeyCalibration::Done,
            1 => KeyCalibration::Running,
            2 => KeyCalibration::TimedOut,
            v => KeyCalibration::Other(v),
        }
    }
}

/// Status of all-key calibration (sub8).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AllCalibration {
    Running,
    /// Aborted, or finished.
    Finished,
    Other(u8),
}

impl From<u8> for AllCalibration {
    fn from(v: u8) -> Self {
        match v {
            1 => AllCalibration::Running,
            4 => AllCalibration::Finished,
            v => AllCalibration::Other(v),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn travel_parse() {
        assert_eq!("1.6".parse::<Travel>().unwrap().raw(), 16);
        assert_eq!("0.3mm".parse::<Travel>().unwrap().raw(), 3);
        assert_eq!("3.6".parse::<Travel>().unwrap(), Travel::MAX);
        assert!("3.7".parse::<Travel>().is_err());
        assert!("1.65".parse::<Travel>().is_err());
        assert!("-0.1".parse::<Travel>().is_err());
        assert_eq!(Travel::from_raw(5).to_string(), "0.5");
        assert_eq!(Travel::from_raw(36).to_string(), "3.6");
    }

    #[test]
    fn trigger_from_wire() {
        let c = TriggerConfig::from_wire([12, 16, 16, 8, 8]).unwrap();
        assert_eq!(c.mode, TriggerMode::Rapid);
        assert_eq!(c.actuation.raw(), 16);
        assert!(TriggerConfig::from_wire([5, 0, 0, 0, 0]).is_err());
    }

    #[test]
    fn debounce_limit() {
        assert!(Debounce::new(251).is_ok());
        assert!(Debounce::new(252).is_err());
    }
}
