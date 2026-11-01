use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use bridge75_core::profile::Profile;
use bridge75_device::Device;
use clap::Subcommand;

use crate::interrupt::uninterrupted;

#[derive(Subcommand)]
pub enum ProfileCommand {
    /// Write the settings that can be read from the keyboard to TOML
    ///
    /// The keymap, macros and advanced keys cannot be read from the keyboard, so they are not included.
    /// Updating the FW resets all settings, so it is a good idea to back them up before updating.
    Dump {
        /// Output file (stdout if omitted)
        #[arg(short, long)]
        output: Option<PathBuf>,
    },
    /// Check the contents of a profile (no keyboard needed)
    Check { file: PathBuf },
    /// Write the items in a profile to the keyboard
    ///
    /// Pressing Ctrl+C during the write finishes writing everything before exiting.
    Apply { file: PathBuf },
}

pub fn run(dev: &Device, cmd: &ProfileCommand) -> Result<()> {
    match cmd {
        ProfileCommand::Dump { output } => dump(dev, output.as_deref()),
        ProfileCommand::Check { file } => check(file),
        ProfileCommand::Apply { file } => apply(dev, file),
    }
}

fn load(path: &Path) -> Result<Profile> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    Profile::from_toml(&text).with_context(|| path.display().to_string())
}

fn dump(dev: &Device, output: Option<&Path>) -> Result<()> {
    let info = dev.device_info()?;
    let profile = dev.read_profile(&mut |step| eprintln!("Reading {step}"))?;
    let text = format!(
        "# bridge75 profile dump\n\
         # FW {} {} (built {} {})\n\
         # The keymap, macros and advanced keys cannot be read from the keyboard, so they are not included.\n\n{}",
        info.firmware.product,
        info.firmware.version,
        info.build.date,
        info.build.time,
        profile.to_toml()
    );
    match output {
        Some(path) => {
            std::fs::write(path, text)
                .with_context(|| format!("cannot write {}", path.display()))?;
            eprintln!("Wrote {}", path.display());
        }
        None => print!("{text}"),
    }
    Ok(())
}

pub fn check(path: &Path) -> Result<()> {
    let profile = load(path)?;
    println!("{}: OK. Items to write:", path.display());
    for line in summary(&profile) {
        println!("  {line}");
    }
    for warning in profile.warnings() {
        println!("warning: {warning}");
    }
    Ok(())
}

fn apply(dev: &Device, path: &Path) -> Result<()> {
    let profile = load(path)?;
    let mut warnings = profile.warnings();
    // Fixed-function keys differ between models, so read them from the keyboard.
    if profile
        .keymap
        .as_ref()
        .is_some_and(|k| !k.fn_layer.is_empty())
    {
        warnings.extend(profile.fixed_fn_warnings(&dev.fixed_fn_mask()?));
    }
    for warning in warnings {
        eprintln!("warning: {warning}");
    }
    uninterrupted(|| Ok(dev.apply_profile(&profile, &mut |step| eprintln!("Writing {step}"))?))?;
    println!("Wrote {} to the keyboard", path.display());
    Ok(())
}

fn summary(p: &Profile) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(d) = p.debounce {
        lines.push(format!("debounce {}", d.value()));
    }
    if let Some(r) = p.polling_rate {
        lines.push(format!("polling rate {r}"));
    }
    if let Some(s) = p.sleep {
        lines.push(format!(
            "sleep time: keyboard {} s / lighting {} s",
            s.keyboard, s.light
        ));
    }
    if !p.trigger.is_empty() {
        lines.push(format!("trigger settings: {} groups", p.trigger.len()));
    }
    if let Some(k) = &p.keymap {
        lines.push(format!(
            "keymap ({} keys changed on the base layer, {} on the Fn layer; others default)",
            k.base.len(),
            k.fn_layer.len()
        ));
    }
    if let Some(m) = &p.macros {
        lines.push(format!("{} macros (writes all 10 slots)", m.len()));
    }
    if let Some(a) = &p.advanced {
        lines.push(format!("{} advanced keys (writes all 40 slots)", a.len()));
    }
    if let Some(s) = &p.socd {
        lines.push(format!(
            "{} SOCD pairs (writes all 20 pairs)",
            s.pairs.len()
        ));
    }
    if lines.is_empty() {
        lines.push("none".into());
    }
    lines
}
