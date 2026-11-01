//! Commands that handle continuous notifications from the keyboard (Ctrl+C cleans up, then exits).

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use bridge75_core::he::{AllCalibration, KeyCalibration, Travel};
use bridge75_core::matrix::{COLS, KeyBitmap, KeyPos, KeySel, LAYOUT, ROWS};
use bridge75_core::request;
use bridge75_core::response::{HeResponse, Response};
use bridge75_device::Device;
use clap::Args;

use crate::interrupt::{self, requested as stopped};
use crate::positions;

const POLL: Duration = Duration::from_millis(20);

fn redraw(line: &str) {
    eprint!("\r{line}\x1b[K");
    let _ = std::io::stderr().flush();
}

// ---- Live travel reports ----

/// Clears the monitored keys on exit (sent even when exiting with an error).
struct MonitorGuard<'a>(&'a Device);

impl Drop for MonitorGuard<'_> {
    fn drop(&mut self) {
        let _ = self.0.send(request::he_clear_monitor());
    }
}

pub fn monitor(dev: &Device, keys: &[KeySel]) -> Result<()> {
    let keys = positions(keys);
    interrupt::watch();
    dev.set_monitor(&keys.iter().copied().collect::<KeyBitmap>())?;
    let _guard = MonitorGuard(dev);
    eprintln!("Showing key travel (Ctrl+C to stop)");
    eprintln!(
        "Note: when several keys move at the same instant, only the report for the last key processed arrives"
    );

    let mut travel: BTreeMap<KeyPos, Travel> =
        keys.iter().map(|p| (*p, Travel::default())).collect();
    let bars = keys.len() <= 6;
    let mut dirty = true;
    let mut last_draw = Instant::now();
    while !stopped() {
        if let Some(Response::He(HeResponse::Travel { pos, travel: t })) =
            dev.next_response(POLL)?
            && let Some(v) = travel.get_mut(&pos)
        {
            *v = t;
            dirty = true;
        }
        if dirty && last_draw.elapsed() >= Duration::from_millis(16) {
            let cells: Vec<String> = travel
                .iter()
                .map(|(pos, t)| {
                    if bars {
                        let filled = t.raw().div_ceil(4) as usize;
                        format!(
                            "{pos} {}{} {t}",
                            "█".repeat(filled),
                            "·".repeat(9 - filled.min(9))
                        )
                    } else {
                        format!("{pos} {t}")
                    }
                })
                .collect();
            redraw(&cells.join("  "));
            dirty = false;
            last_draw = Instant::now();
        }
    }
    eprintln!();
    Ok(())
}

// ---- Raw sensor values ----

#[derive(Args)]
pub struct SensorsArgs {
    /// Keys to show (with --csv, all keys if omitted)
    keys: Vec<KeySel>,
    /// Write every frame to a CSV file (columns: ms since start of receiving, then each key's value)
    #[arg(long)]
    csv: Option<PathBuf>,
    /// Seconds to record (until Ctrl+C if omitted)
    #[arg(long)]
    duration: Option<f64>,
}

/// Stops the stream on exit. Otherwise 6000 packets/s keep arriving.
struct StreamGuard<'a>(&'a Device);

impl Drop for StreamGuard<'_> {
    fn drop(&mut self) {
        let _ = self.0.set_raw_stream(false);
    }
}

pub fn sensors(dev: &Device, args: &SensorsArgs) -> Result<()> {
    if args.keys.is_empty() && args.csv.is_none() {
        bail!("no keys given (they can be omitted only with --csv, which then records all keys)");
    }
    let keys = positions(&args.keys);
    let mut csv = match &args.csv {
        Some(path) => {
            let file =
                File::create(path).with_context(|| format!("cannot create {}", path.display()))?;
            let mut w = BufWriter::new(file);
            let names: Vec<String> = keys.iter().map(|p| p.to_string()).collect();
            writeln!(w, "ms,{}", names.join(","))?;
            Some(w)
        }
        None => None,
    };
    let duration = args.duration.map(Duration::from_secs_f64);
    interrupt::watch();

    dev.drain();
    dev.set_raw_stream(true)?;
    let _guard = StreamGuard(dev);
    eprintln!(
        "Receiving raw sensor values (Ctrl+C to stop). Values decrease as a key is pressed deeper"
    );

    let mut frame = [[0u16; COLS]; ROWS];
    let mut frames = 0u64;
    let start = Instant::now();
    let mut last_draw = start;
    let mut last_packet = start;
    while !stopped() && duration.is_none_or(|d| start.elapsed() < d) {
        match dev.next_response(POLL)? {
            Some(Response::RawSensorRow { row, values }) if (row as usize) < ROWS => {
                last_packet = Instant::now();
                frame[row as usize] = values;
                if row as usize == ROWS - 1 {
                    frames += 1;
                    if let Some(w) = &mut csv {
                        let ms = start.elapsed().as_secs_f64() * 1000.0;
                        let values: Vec<String> = keys
                            .iter()
                            .map(|p| frame[p.row as usize][p.col as usize].to_string())
                            .collect();
                        writeln!(w, "{ms:.3},{}", values.join(","))?;
                    }
                }
            }
            _ if last_packet.elapsed() > Duration::from_secs(2) => {
                bail!("no raw sensor values received");
            }
            _ => {}
        }
        if last_draw.elapsed() >= Duration::from_millis(100) {
            let rate = frames as f64 / start.elapsed().as_secs_f64();
            let line = if csv.is_some() {
                format!("{frames} frames ({rate:.0}/s)")
            } else {
                let cells: Vec<String> = keys
                    .iter()
                    .map(|p| format!("{p} {:4}", frame[p.row as usize][p.col as usize]))
                    .collect();
                format!("{}  [{rate:.0}/s]", cells.join("  "))
            };
            redraw(&line);
            last_draw = Instant::now();
        }
    }
    eprintln!();
    if let Some(mut w) = csv {
        w.flush()?;
        eprintln!("Wrote {frames} frames");
    }
    Ok(())
}

// ---- Calibration ----

#[derive(Args)]
#[group(required = true, multiple = false)]
pub struct CalibrateArgs {
    /// Calibrate a single key
    key: Option<KeyPos>,
    /// Start calibrating all keys (clears the current calibration values and calibrated flags)
    #[arg(long)]
    all: bool,
}

pub fn calibrate(dev: &Device, args: &CalibrateArgs) -> Result<()> {
    interrupt::watch();
    // The rest calibration values are relearned as soon as this is sent, so ask for hands off first.
    interrupt::hands_off()?;
    match args.key {
        Some(pos) => calibrate_key(dev, pos),
        None => calibrate_all(dev),
    }
}

fn calibrate_key(dev: &Device, pos: KeyPos) -> Result<()> {
    dev.drain();
    dev.start_calibrate_key(pos)?;
    eprintln!("{pos}: press it to the bottom, hold for about 1 s, then release");
    let start = Instant::now();
    while !stopped() {
        match dev.next_response(POLL)? {
            Some(Response::He(HeResponse::CalibrateKey(state))) => match state {
                KeyCalibration::Running => {}
                KeyCalibration::Done => {
                    println!("Calibration of {pos} finished");
                    return Ok(());
                }
                KeyCalibration::TimedOut => {
                    bail!("calibration of {pos} timed out")
                }
                KeyCalibration::Other(v) => bail!("unexpected state {v}"),
            },
            _ if start.elapsed() > Duration::from_secs(60) => bail!("not finished after 60 s"),
            _ => {}
        }
    }
    bail!("aborted (single-key calibration continues on the FW side until it times out)")
}

fn calibrate_all(dev: &Device) -> Result<()> {
    dev.drain();
    dev.start_calibrate_all()?;
    eprintln!("Started calibrating all keys (Ctrl+C to abort)");
    eprintln!("Press each key to the bottom one at a time and release it.");
    eprintln!("Calibration finishes about 10 s after the last key is calibrated.");
    let all = KeyBitmap::all();
    let mut calibrated = KeyBitmap::EMPTY;
    let mut aborted_at: Option<Instant> = None;
    loop {
        if stopped() && aborted_at.is_none() {
            dev.abort_calibrate_all()?;
            aborted_at = Some(Instant::now());
        }
        if aborted_at.is_some_and(|t| t.elapsed() > Duration::from_secs(3)) {
            break;
        }
        if let Some(Response::He(HeResponse::CalibrateAll {
            state,
            calibrated: got,
        })) = dev.next_response(POLL)?
        {
            calibrated = got;
            let done = all.iter().filter(|p| calibrated.contains(*p)).count();
            redraw(&format!("Calibrated {done}/{}", LAYOUT.len()));
            match state {
                AllCalibration::Running => {}
                AllCalibration::Finished => break,
                AllCalibration::Other(v) => bail!("unexpected state {v}"),
            }
        }
    }
    eprintln!();
    let missing: Vec<String> = all
        .iter()
        .filter(|p| !calibrated.contains(*p))
        .map(|p| p.to_string())
        .collect();
    if aborted_at.is_some() {
        println!("Aborted");
    } else {
        println!("Calibration finished");
    }
    if !missing.is_empty() {
        println!(
            "Keys not calibrated ({}): {}",
            missing.len(),
            missing.join(" ")
        );
    }
    Ok(())
}
