//! Ctrl+C handling.
//!
//! A handler can be registered only once, so a single one is installed with [`install`] at startup
//! and its behavior is switched depending on the situation.
//!
//! - Default: exit immediately (read-only commands).
//! - After [`watch`]: only set a flag. Live commands check [`requested`], clean up, then exit.
//! - Inside [`uninterrupted`]: finish sending the whole write sequence, then exit with code 130.

use std::fmt;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};

/// Exit code when stopped by Ctrl+C.
pub const EXIT_CODE: u8 = 130;

/// Exit immediately.
const EXIT: u8 = 0;
/// Only set the flag.
const WATCH: u8 = 1;
/// Wait until the write finishes.
const DEFER: u8 = 2;

static MODE: AtomicU8 = AtomicU8::new(EXIT);
static REQUESTED: AtomicBool = AtomicBool::new(false);

/// Interrupted by Ctrl+C. main prints the message and exits with [`EXIT_CODE`].
#[derive(Debug)]
pub struct Interrupted(&'static str);

impl fmt::Display for Interrupted {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

impl std::error::Error for Interrupted {}

pub fn install() -> Result<()> {
    ctrlc::set_handler(|| {
        // Set the flag first. uninterrupted() switches MODE before checking the flag, so
        // in either order it either stops before the write starts or sends the whole write.
        REQUESTED.store(true, Ordering::SeqCst);
        match MODE.load(Ordering::SeqCst) {
            DEFER => eprintln!("\nWill exit once the write has finished"),
            WATCH => {}
            _ => std::process::exit(EXIT_CODE.into()),
        }
    })
    .context("cannot install the Ctrl+C handler")
}

/// From now on, Ctrl+C only sets the flag (live commands).
pub fn watch() {
    MODE.store(WATCH, Ordering::SeqCst);
}

/// Whether Ctrl+C was received.
pub fn requested() -> bool {
    REQUESTED.load(Ordering::SeqCst)
}

/// If Ctrl+C arrives during `write`, sends everything first, then ends with [`Interrupted`].
///
/// 0x18 / 0x19 / 0x22 and others are saved only when the last packet arrives, so stopping midway saves nothing.
pub fn uninterrupted<T>(write: impl FnOnce() -> Result<T>) -> Result<T> {
    let previous = MODE.swap(DEFER, Ordering::SeqCst);
    if requested() {
        MODE.store(previous, Ordering::SeqCst);
        return Err(Interrupted("aborted without sending anything").into());
    }
    let result = write();
    MODE.store(previous, Ordering::SeqCst);
    if requested() {
        result?;
        return Err(Interrupted("interrupted after sending the whole write").into());
    }
    result
}

/// Asks the user to take their hands off the keys before a request that relearns rest calibration values (0x21 sub6 / 7 / 8).
///
/// On receipt, FW discards the rest calibration values and relearns them from about 64 ms of steady readings.
/// If sent while a key is held, the pressed position is recorded as the rest position.
/// If Ctrl+C arrives during the 3 s countdown, ends with [`Interrupted`] without sending anything.
pub fn hands_off() -> Result<()> {
    const SECONDS: u64 = 3;
    let previous = MODE.swap(WATCH, Ordering::SeqCst);
    eprintln!(
        "Take your hands off the keys. The rest position will be relearned (Ctrl+C to abort)"
    );
    let start = Instant::now();
    let mut shown = 0;
    while start.elapsed() < Duration::from_secs(SECONDS) {
        if requested() {
            eprintln!();
            MODE.store(previous, Ordering::SeqCst);
            return Err(Interrupted("aborted without sending anything").into());
        }
        let left = SECONDS - start.elapsed().as_secs();
        if left != shown {
            eprint!("\rStarting in {left} s\x1b[K");
            let _ = std::io::stderr().flush();
            shown = left;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    eprint!("\r\x1b[K");
    let _ = std::io::stderr().flush();
    MODE.store(previous, Ordering::SeqCst);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // The state is shared by the whole process, so check everything in order in one test.
    #[test]
    fn deferred_and_cancelled_writes() {
        let interrupted = |r: Result<u8>| r.unwrap_err().downcast::<Interrupted>().unwrap().0;

        // If nothing happens, returns the result as is and restores the mode.
        assert_eq!(uninterrupted(|| Ok(1)).unwrap(), 1);
        assert_eq!(MODE.load(Ordering::SeqCst), EXIT);

        // If received during the write, sends everything and then interrupts.
        let mut sent = 0;
        let result = uninterrupted(|| {
            assert_eq!(MODE.load(Ordering::SeqCst), DEFER);
            REQUESTED.store(true, Ordering::SeqCst);
            sent += 1;
            Ok(2)
        });
        assert_eq!(sent, 1);
        assert_eq!(
            interrupted(result),
            "interrupted after sending the whole write"
        );
        assert_eq!(MODE.load(Ordering::SeqCst), EXIT);

        // If received before the write starts, sends nothing.
        let result = uninterrupted(|| -> Result<u8> { panic!("must not send") });
        assert_eq!(interrupted(result), "aborted without sending anything");

        // If received during the countdown, aborts without waiting.
        watch();
        let start = Instant::now();
        assert!(hands_off().is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
        assert_eq!(MODE.load(Ordering::SeqCst), WATCH);
        REQUESTED.store(false, Ordering::SeqCst);
        MODE.store(EXIT, Ordering::SeqCst);
    }
}
