//! Raw HID communication with the Bridge75 HE.
//!
//! [`Device`] owns a dedicated I/O thread that handles the send queue, receiving, and the periodic 0x01 heartbeat.
//! Responses are asynchronous and queued, so requests are sent one at a time and each waits for its response ([`Device::request`]).

mod device;
mod profile;

pub use bridge75_core as core;
pub use device::{Device, DeviceEntry, list};

/// A communication failure.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("HID: {0}")]
    Hid(#[from] hidapi::HidError),
    #[error("Bridge75 HE not found")]
    NotFound,
    #[error("Bridge75 HE is in bootloader (FW update) mode; unplug and replug the USB cable")]
    Bootloader,
    #[error("{0} is not the configuration interface of a Bridge75 HE (normal mode)")]
    NotConfigInterface(String),
    #[error(
        "not a Bridge75 HE ({expected}): product name is {found}",
        expected = bridge75_core::PRODUCT_NAME
    )]
    WrongProduct { found: String },
    #[error("{0}: no response")]
    Timeout(String),
    #[error("{what}: unexpected response ({detail})")]
    Unexpected { what: String, detail: String },
    #[error("lost connection to the keyboard: {0}")]
    Disconnected(String),
    #[error(transparent)]
    Protocol(#[from] bridge75_core::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
