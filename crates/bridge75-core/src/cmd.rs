//! Wire cmd (packet byte `b[1]`).
//!
//! These are the values actually sent, not the official configurator's "internal numbers". Dangerous ones (0x0B / 0x0C / 0x16)
//! are intentionally not defined.

pub const HEARTBEAT: u8 = 0x01;
pub const LIGHTING: u8 = 0x07;
pub const DEVICE_INFO: u8 = 0x0D;
pub const FIXED_FN_MASK: u8 = 0x0E;
pub const DEFAULT_KEYMAP: u8 = 0x10;
pub const SYSTEM: u8 = 0x14;
pub const SERIAL_NUMBER: u8 = 0x15;
pub const KEYMAP: u8 = 0x18;
pub const MACRO: u8 = 0x19;
pub const CUSTOM_EVENT: u8 = 0x20;
pub const HE: u8 = 0x21;
pub const ADVANCED_KEY: u8 = 0x22;
pub const RAW_SENSOR: u8 = 0x23;
pub const SOCD: u8 = 0x24;
