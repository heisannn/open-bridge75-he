//! Status and device info (wire 0x01 / 0x0D / 0x14).

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Response to the 0x01 heartbeat. The keyboard is wired, so the battery and connection values are fixed.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Heartbeat {
    /// Whether Win lock is enabled (`b[6]`).
    pub win_lock: bool,
    /// Whether 40 or more keys have calibration values (`b[20]`).
    pub calibrated: bool,
}

/// First 0x0D packet: `"M484,22,KB,OW,75ALHE,V1.13.10"`.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FirmwareId {
    pub mcu: String,
    /// Bootloader version (on the real keyboard the FW overwrites it with the value from the bootloader area, giving `22`).
    pub bootloader: String,
    pub kind: String,
    /// Customer code.
    pub customer: String,
    /// Product name (hidName). `75ALHE` for this keyboard.
    pub product: String,
    /// FW version. Older FW uses underscores, as in `V1_12_08`.
    pub version: String,
}

impl FirmwareId {
    pub fn parse(s: &str) -> Result<Self> {
        let fields: Vec<&str> = s.split(',').map(str::trim).collect();
        let [mcu, bootloader, kind, customer, product, version] = fields[..] else {
            return Err(Error::Invalid(format!("cannot parse device info: {s:?}")));
        };
        Ok(FirmwareId {
            mcu: mcu.into(),
            bootloader: bootloader.into(),
            kind: kind.into(),
            customer: customer.into(),
            product: product.into(),
            version: version.into(),
        })
    }
}

/// Second 0x0D packet: build date and time (`"Nov 19 2024"`, `"10:36:50"`).
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BuildTime {
    pub date: String,
    pub time: String,
}

/// Both 0x0D packets combined.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DeviceInfo {
    pub firmware: FirmwareId,
    pub build: BuildTime,
}

/// Time until sleep (seconds). `b[9..=12]` of 0x14. Defaults are 60 s for lighting and 1800 s for the keyboard.
///
/// When the saved value is 0, the FW returns the default ([`SleepTimes::DEFAULT`]) to a query.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SleepTimes {
    /// Until the lighting turns off. This keyboard has no lighting, so it has no effect.
    pub light: u16,
    /// Until the keyboard sleeps.
    pub keyboard: u16,
}

impl SleepTimes {
    /// FW default. A query also returns this when the saved value is 0.
    pub const DEFAULT: SleepTimes = SleepTimes {
        light: 60,
        keyboard: 1800,
    };

    /// Official configurator constraints: lighting <= keyboard, keyboard >= 300 s.
    /// 0 is not sent (the FW replaces a saved 0 with the default in queries, so the written value cannot be verified).
    pub fn validate(&self) -> Result<()> {
        if self.keyboard < 300 {
            return Err(Error::OutOfRange(format!(
                "keyboard sleep time must be at least 300 s (got {} s)",
                self.keyboard
            )));
        }
        if self.light == 0 {
            return Err(Error::OutOfRange(
                "lighting sleep time must be at least 1 s (got 0 s)".into(),
            ));
        }
        if self.light > self.keyboard {
            return Err(Error::OutOfRange(format!(
                "lighting sleep time ({} s) must not exceed the keyboard sleep time ({} s)",
                self.light, self.keyboard
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_firmware_id() {
        let id = FirmwareId::parse("M484,22,KB,OW,75ALHE,V1.13.10").unwrap();
        assert_eq!(id.product, "75ALHE");
        assert_eq!(id.version, "V1.13.10");
        assert!(FirmwareId::parse("M484,22").is_err());
    }

    #[test]
    fn sleep_limits() {
        let ok = |light, keyboard| SleepTimes { light, keyboard }.validate().is_ok();
        assert!(SleepTimes::DEFAULT.validate().is_ok());
        assert!(ok(1, 300));
        assert!(ok(300, 300));
        assert!(!ok(0, 1800));
        assert!(!ok(0, 0));
        assert!(!ok(60, 0));
        assert!(!ok(60, 299));
        assert!(!ok(301, 300));
    }
}
