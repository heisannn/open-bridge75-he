mod interrupt;
mod live;
mod profile;
mod trigger;

use std::ffi::CString;
use std::fmt::Display;
use std::process::ExitCode;
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use bridge75_core::he::{Debounce, PollingRate};
use bridge75_core::hid::Usage;
use bridge75_core::info::SleepTimes;
use bridge75_core::keymap::{KeyAction, SystemFn};
use bridge75_core::matrix::{KeyPos, KeySel, LAYOUT, ROWS};
use bridge75_core::socd::{SocdBehavior, SocdMode};
use bridge75_device::Device;
use clap::{Args, Parser, Subcommand};

use crate::interrupt::{Interrupted, uninterrupted};

#[derive(Parser)]
#[command(
    name = "bridge75",
    version,
    about = "Configurator for the Bridge75 HE (75ALHE)"
)]
struct Cli {
    /// Path of the keyboard to use (shown by `bridge75 list`). Defaults to the first one found
    #[arg(long, global = true, help_heading = "Global Options")]
    device: Option<String>,
    /// How long to wait for each response (milliseconds)
    #[arg(
        long,
        global = true,
        default_value_t = 1000,
        help_heading = "Global Options"
    )]
    timeout: u64,
    /// Continue even if the product name (0x0D) is not 75ALHE
    #[arg(long, global = true, help_heading = "Global Options")]
    force: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// List connected keyboards
    List,
    /// List key names and matrix positions (no keyboard needed)
    Keys {
        /// Also show labels and default assignments
        #[arg(short, long)]
        long: bool,
    },
    /// List the syntax for key assignments (no keyboard needed)
    Actions,
    /// Show FW version, build date, SN and calibration status
    Info,
    /// Trigger settings (actuation point, release point, Rapid Trigger)
    #[command(subcommand)]
    Trigger(trigger::TriggerCommand),
    /// Show or set debounce (default 3; lower reacts faster)
    Debounce {
        /// Value to set (0 to 251). Omit to show the current value
        value: Option<u8>,
    },
    /// Show or set the polling rate (only affects the hidden measurement mode Fn+O+K)
    Polling {
        /// 1 / 2 / 4 / 8 (kHz). Omit to show the current value
        rate: Option<PollingRate>,
    },
    /// Show or set sleep time
    Sleep(SleepArgs),
    /// Calibrate keys (also relearns the rest calibration values)
    ///
    /// Counts down 3 s before sending; keep your hands off the keys during the countdown.
    Calibrate(live::CalibrateArgs),
    /// Show live travel reports (Ctrl+C to stop)
    Monitor {
        /// Keys to show
        #[arg(required = true)]
        keys: Vec<KeySel>,
    },
    /// Show or record raw sensor values (12-bit ADC, 1000 frames/s)
    Sensors(live::SensorsArgs),
    /// Show SOCD pairs (configure them with a profile)
    Socd,
    /// Dump, check or write a profile (TOML)
    #[command(subcommand)]
    Profile(profile::ProfileCommand),
}

#[derive(Args)]
struct SleepArgs {
    /// Seconds until the keyboard sleeps (300 or more)
    #[arg(long)]
    keyboard: Option<u16>,
    /// Seconds until the lighting turns off (no effect: this model has no lighting)
    #[arg(long)]
    light: Option<u16>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match interrupt::install().and_then(|()| run(&cli)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => match e.downcast_ref::<Interrupted>() {
            Some(interrupted) => {
                eprintln!("{interrupted}");
                ExitCode::from(interrupt::EXIT_CODE)
            }
            None => {
                eprintln!("error: {e:#}");
                ExitCode::FAILURE
            }
        },
    }
}

fn run(cli: &Cli) -> Result<()> {
    match &cli.command {
        Command::List => return list(),
        Command::Keys { long } => return keys(*long),
        Command::Actions => return actions(),
        Command::Profile(profile::ProfileCommand::Check { file }) => return profile::check(file),
        _ => {}
    }
    // info only shows the product name, so a mismatch is just a warning.
    let dev = open(cli, !matches!(cli.command, Command::Info))?;
    match &cli.command {
        Command::Info => info(&dev),
        Command::Trigger(cmd) => trigger::run(&dev, cmd),
        Command::Debounce { value } => debounce(&dev, *value),
        Command::Polling { rate } => polling(&dev, *rate),
        Command::Sleep(args) => sleep(&dev, args),
        Command::Calibrate(args) => live::calibrate(&dev, args),
        Command::Monitor { keys } => live::monitor(&dev, keys),
        Command::Sensors(args) => live::sensors(&dev, args),
        Command::Socd => socd(&dev),
        Command::Profile(cmd) => profile::run(&dev, cmd),
        Command::List | Command::Keys { .. } | Command::Actions => unreachable!(),
    }
}

/// Opens the keyboard. With `check`, verifies the product name (0x0D) and refuses another model unless `--force` is given.
fn open(cli: &Cli, check: bool) -> Result<Device> {
    let mut dev = match &cli.device {
        Some(path) => {
            let path = CString::new(path.as_str()).context("path contains a NUL byte")?;
            Device::open(&path).map_err(|e| match e {
                bridge75_device::Error::NotConfigInterface(_) => {
                    anyhow!("{e} (use a path shown by `bridge75 list`)")
                }
                e => e.into(),
            })
        }
        None => Device::open_first().map_err(Into::into),
    }
    .context("cannot open the keyboard")?;
    dev.set_timeout(Duration::from_millis(cli.timeout));
    if check && let Err(e) = dev.check_product() {
        if !cli.force {
            return Err(e).context(
                "aborted: could not confirm this is a Bridge75 HE (add --force if you are sure it is)",
            );
        }
        eprintln!("warning: {e} (continuing because of --force)");
    }
    Ok(dev)
}

/// Pads with spaces on the right, counting full-width characters as 2 columns.
pub fn pad(s: &str, width: usize) -> String {
    let w: usize = s.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum();
    format!("{s}{}", " ".repeat(width.saturating_sub(w)))
}

fn field(label: &str, value: impl Display) {
    println!("{}{value}", pad(label, 20));
}

fn list() -> Result<()> {
    let entries = bridge75_device::list()?;
    if entries.is_empty() {
        println!("No Bridge75 HE found");
    }
    for e in entries {
        let mode = if e.bootloader {
            "bootloader (FW update) mode"
        } else {
            "normal"
        };
        let sn = e.serial.as_deref().unwrap_or("-");
        println!(
            "{}  SN {sn}  {mode}\n  {}",
            e.product.as_deref().unwrap_or("?"),
            e.path.to_string_lossy()
        );
    }
    Ok(())
}

fn keys(long: bool) -> Result<()> {
    if long {
        println!(
            "{}{}{}Default",
            pad("Key", 12),
            pad("Label", 10),
            pad("Position", 10)
        );
        for key in &LAYOUT {
            let pos = format!("r{}c{}", key.pos.row, key.pos.col);
            println!(
                "{}{}{}{}",
                pad(key.id, 12),
                pad(key.label, 10),
                pad(&pos, 10),
                KeyAction::from(key.default)
            );
        }
    } else {
        for row in 0..ROWS as u8 {
            let ids: Vec<_> = LAYOUT
                .iter()
                .filter(|k| k.pos.row == row)
                .map(|k| k.id)
                .collect();
            println!("Row {row}: {}", ids.join(" "));
        }
    }
    println!(
        "\nKeys can also be given as r<row>c<col> (e.g. r3c2 = a). `all` selects all 81 keys."
    );
    Ok(())
}

fn actions() -> Result<()> {
    println!(
        "Key assignments (syntax used in [keymap.base] / [keymap.fn] of a profile and in advanced keys):\n"
    );
    for (syntax, meaning) in [
        ("a, lctrl, 0x2c", "regular key (HID usage)"),
        ("none", "no input"),
        ("fn", "Fn"),
        (
            "media:<0..63>",
            "media key (FW bit number; volup / voldown / mute also accepted by name)",
        ),
        (
            "macro:<0..9>[:<key>]",
            "macro (the optional key is the fallback sent when the slot is empty)",
        ),
        (
            "mouse:left+right",
            "mouse buttons (left / right / middle / back / forward)",
        ),
        ("wheel:up, wheel:down", "mouse wheel"),
        ("combo:lctrl+c", "combo (up to 3 keys)"),
        (
            "event:<key>",
            "custom event key (acts as that key when no driver is connected)",
        ),
        ("adv:<0..39>", "advanced key (DKS / MT / TGL)"),
        (
            "socd:<0..19>, socd:all",
            "toggle an SOCD pair / toggle SOCD globally",
        ),
        (
            "raw:<8 hex digits>",
            "raw 4 bytes (macro / advanced key / SOCD indices are range-checked)",
        ),
    ] {
        println!("  {}{meaning}", pad(syntax, 28));
    }
    let systems: Vec<_> = SystemFn::all()
        .map(|f| format!("sys:{}", f.name()))
        .collect();
    println!("  System functions: {}\n", systems.join(", "));
    println!("Key names:");
    let names: Vec<_> = Usage::all().map(|(_, name)| name).collect();
    for chunk in names.chunks(12) {
        println!("  {}", chunk.join(" "));
    }
    println!(
        "\nAliases: escape return bksp period capslock prtsc ins delete pageup pagedown app ctrl shift alt win lgui rgui"
    );
    Ok(())
}

fn info(dev: &Device) -> Result<()> {
    let info = dev.device_info()?;
    let fw = &info.firmware;
    if fw.product != bridge75_core::PRODUCT_NAME {
        eprintln!(
            "warning: product name is not {} ({})",
            bridge75_core::PRODUCT_NAME,
            fw.product
        );
    }
    field(
        "Product",
        format!("{} ({}, customer code {})", fw.product, fw.mcu, fw.customer),
    );
    field(
        "FW",
        format!(
            "{} (built {} {})",
            fw.version, info.build.date, info.build.time
        ),
    );
    field("Bootloader", &fw.bootloader);
    field("SN", dev.serial_number()?);
    let heartbeat = dev.heartbeat()?;
    field(
        "Calibration",
        if heartbeat.calibrated {
            "done"
        } else {
            "not done (fewer than 40 keys)"
        },
    );
    field("Win lock", if heartbeat.win_lock { "ON" } else { "OFF" });
    field(
        "Max travel",
        format!("{:.1}mm", dev.max_travel()? as f64 / 10.0),
    );
    field("Debounce", dev.debounce()?.value());
    field("Polling rate", dev.polling_rate()?);
    Ok(())
}

fn debounce(dev: &Device, value: Option<u8>) -> Result<()> {
    if let Some(value) = value {
        let value = Debounce::new(value)?;
        uninterrupted(|| Ok(dev.set_debounce(value)?))?;
        println!("Debounce set to {}", value.value());
    } else {
        let value = dev.debounce()?.value();
        let note = if value == Debounce::DEFAULT.value() {
            " (default)"
        } else {
            ""
        };
        println!("Debounce: {value}{note}");
    }
    Ok(())
}

fn polling(dev: &Device, rate: Option<PollingRate>) -> Result<()> {
    if let Some(rate) = rate {
        uninterrupted(|| Ok(dev.set_polling_rate(rate)?))?;
        println!("Polling rate set to {rate}");
    } else {
        println!("Polling rate: {}", dev.polling_rate()?);
    }
    println!(
        "(This value is used only by the hidden polling rate measurement mode Fn+O+K. Normal input runs at 8kHz over USB High-Speed.)"
    );
    Ok(())
}

fn sleep(dev: &Device, args: &SleepArgs) -> Result<()> {
    let current = dev.sleep_times()?;
    if args.keyboard.is_none() && args.light.is_none() {
        let minutes = |s: u16| format!("{s} s ({:.1} min)", s as f64 / 60.0);
        field("Keyboard", minutes(current.keyboard));
        field("Lighting", minutes(current.light));
        return Ok(());
    }
    let keyboard = args.keyboard.unwrap_or(current.keyboard);
    let mut light = args.light.unwrap_or(current.light);
    if args.light.is_none() && light > keyboard {
        light = keyboard;
    }
    let times = SleepTimes { light, keyboard };
    times.validate()?;
    uninterrupted(|| Ok(dev.set_sleep_times(times)?))?;
    let after = dev.sleep_times()?;
    if after != times {
        bail!("value read back ({after:?}) differs from the value written ({times:?})");
    }
    println!("Sleep time set to {keyboard} s for the keyboard and {light} s for the lighting");
    Ok(())
}

fn socd(dev: &Device) -> Result<()> {
    let status = dev.socd_status()?;
    let pairs = dev.socd_pairs()?;
    field("Global", if status.global { "ON" } else { "OFF" });
    let mut any = false;
    for (i, pair) in pairs.iter().enumerate().filter(|(_, p)| !p.is_empty()) {
        any = true;
        let behavior = match pair.behavior {
            SocdBehavior::Last => "last input wins",
            SocdBehavior::A => "A wins",
            SocdBehavior::B => "B wins",
            SocdBehavior::Neutral => "release both",
        };
        let effect = match pair.mode {
            SocdMode::Always => "always".to_string(),
            SocdMode::Global => format!(
                "when global is on (currently {})",
                if status.is_active(i as u8, pair.mode) {
                    "active"
                } else {
                    "inactive"
                }
            ),
            SocdMode::Off => "disabled".to_string(),
        };
        println!(
            "  {i:>2}: {} {}{}{effect}",
            pad(&format!("{} / {}", pair.a, pair.b), 20),
            pad(&format!("{behavior} ({})", pair.behavior.name()), 24),
            pad(&format!("mode {}", pair.mode.name()), 14),
        );
    }
    if !any {
        println!("  No pairs");
    }
    Ok(())
}

/// Resolves key selections to the positions of keys that exist. Empty means all keys.
pub fn positions(keys: &[KeySel]) -> Vec<KeyPos> {
    if keys.is_empty() {
        return LAYOUT.iter().map(|k| k.pos).collect();
    }
    KeySel::to_bitmap(keys)
        .iter()
        .filter(|p| p.key().is_some())
        .collect()
}
