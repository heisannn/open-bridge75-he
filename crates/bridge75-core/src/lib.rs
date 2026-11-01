//! Common types and HID protocol for the Bridge75 HE (FW internal name 75ALHE).
//!
//! This crate does no I/O. It provides packet building ([`request`]) and parsing ([`response`]),
//! the key matrix ([`matrix`]), the types for each setting, and the profile stored on the PC ([`profile`]).
//! The actual sending and receiving is done by `bridge75-device`. The spec is in `docs/protocol.md`.
//!
//! Dangerous commands (0x0B / 0x0C FW update, 0x16 SN rewrite, 0x14 factory reset) have
//! no builder functions.

pub mod advanced;
pub mod cmd;
pub mod error;
pub mod he;
pub mod hid;
pub mod info;
pub mod keymap;
pub mod macros;
pub mod matrix;
pub mod packet;
pub mod profile;
pub mod request;
pub mod response;
pub mod socd;

pub use error::{Error, Result};
pub use packet::Packet;

/// USB Vendor ID (Nuvoton).
pub const VENDOR_ID: u16 = 0x0416;
/// Product ID in normal mode.
pub const PRODUCT_ID: u16 = 0x7372;
/// Product ID of the bootloader (FW update mode).
pub const BOOTLOADER_PRODUCT_ID: u16 = 0x3F01;
/// Interface number used for configuration.
pub const CONFIG_INTERFACE: i32 = 2;
/// Usage Page of the configuration interface.
pub const CONFIG_USAGE_PAGE: u16 = 0xFF1B;
/// Usage of the configuration interface.
pub const CONFIG_USAGE: u16 = 0x91;
/// Product name (hidName) returned by 0x0D.
pub const PRODUCT_NAME: &str = "75ALHE";
