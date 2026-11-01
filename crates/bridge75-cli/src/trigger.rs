use anyhow::{Result, bail};
use bridge75_core::he::{Travel, TriggerConfig, TriggerMode};
use bridge75_core::matrix::{KeyPos, KeySel, LAYOUT};
use bridge75_core::profile::TriggerGroup;
use bridge75_device::Device;
use clap::{Args, Subcommand};

use crate::interrupt::{self, uninterrupted};
use crate::{pad, positions};

#[derive(Subcommand)]
pub enum TriggerCommand {
    /// Show trigger settings (all keys if none are given)
    Show {
        keys: Vec<KeySel>,
        /// Show each key separately instead of grouping keys with the same settings
        #[arg(long)]
        per_key: bool,
    },
    /// Change trigger settings. Items not given keep each key's current value
    ///
    /// Example: bridge75 trigger set all --mode rapid -a 1.6 --rt 0.8
    Set(SetArgs),
    /// Reset all keys to the default (normal, 1.8mm). Also relearns the rest calibration values
    ///
    /// The current trigger settings are lost, so back them up with `bridge75 profile dump` first.
    ///
    /// Counts down 3 s before sending; keep your hands off the keys during the countdown.
    Reset {
        /// Required as confirmation
        #[arg(long)]
        yes: bool,
    },
}

#[derive(Args)]
pub struct SetArgs {
    /// Keys to change (`all` for all keys)
    #[arg(required = true)]
    keys: Vec<KeySel>,
    /// normal / split / rapid / rapid-split
    #[arg(short, long)]
    mode: Option<TriggerMode>,
    /// Actuation point (mm, in 0.1 steps, max 3.6)
    #[arg(short, long)]
    actuation: Option<Travel>,
    /// Release point / RT exit point (mm). Used by split / rapid-split
    #[arg(short, long)]
    release: Option<Travel>,
    /// RT sensitivity (both press and release, mm). The effective value is 0.1mm larger
    #[arg(long)]
    rt: Option<Travel>,
    /// RT press sensitivity (mm)
    #[arg(long)]
    rt_press: Option<Travel>,
    /// RT release sensitivity (mm)
    #[arg(long)]
    rt_release: Option<Travel>,
}

pub fn run(dev: &Device, cmd: &TriggerCommand) -> Result<()> {
    match cmd {
        TriggerCommand::Show { keys, per_key } => {
            let configs = dev.triggers(positions(keys))?;
            print(&configs, *per_key);
            Ok(())
        }
        TriggerCommand::Set(args) => set(dev, args),
        TriggerCommand::Reset { yes } => {
            if !yes {
                bail!(
                    "this resets the trigger settings of all keys to the defaults and relearns the rest calibration values.\n\
                     Back up with `bridge75 profile dump` first, then run again with --yes."
                );
            }
            interrupt::hands_off()?;
            uninterrupted(|| Ok(dev.reset_triggers()?))?;
            println!("Reset the trigger settings of all keys to the defaults");
            Ok(())
        }
    }
}

impl SetArgs {
    fn is_empty(&self) -> bool {
        self.mode.is_none()
            && self.actuation.is_none()
            && self.release.is_none()
            && self.rt.is_none()
            && self.rt_press.is_none()
            && self.rt_release.is_none()
    }

    /// Applies the given items on top of the current settings.
    fn apply(&self, current: &TriggerConfig) -> TriggerConfig {
        let mode = self.mode.unwrap_or(current.mode);
        let actuation = self.actuation.unwrap_or(current.actuation);
        // In normal / rapid, send release point = actuation point, as the official app does.
        let release = match self.release {
            Some(r) => r,
            None if mode.uses_release_point() => current.release,
            None => actuation,
        };
        TriggerConfig {
            mode,
            actuation,
            release,
            rt_press: self.rt_press.or(self.rt).unwrap_or(current.rt_press),
            rt_release: self.rt_release.or(self.rt).unwrap_or(current.rt_release),
        }
    }
}

fn set(dev: &Device, args: &SetArgs) -> Result<()> {
    if args.is_empty() {
        bail!(
            "nothing to change: give at least one of --mode / --actuation / --release / --rt and so on"
        );
    }
    let keys = positions(&args.keys);
    let current = dev.triggers(keys.iter().copied())?;
    let updated: Vec<(KeyPos, TriggerConfig)> = current
        .iter()
        .map(|(pos, config)| (*pos, args.apply(config)))
        .collect();

    if let Some((pos, _)) = updated
        .iter()
        .find(|(_, c)| c.mode.is_rapid() && (c.rt_press.raw() == 0 || c.rt_release.raw() == 0))
    {
        bail!("RT sensitivity of {pos} would be 0mm; set it with --rt");
    }
    if let Some((pos, c)) = updated
        .iter()
        .find(|(_, c)| c.mode.uses_release_point() && c.release > c.actuation)
    {
        eprintln!(
            "warning: release point of {pos} ({}mm) is deeper than its actuation point ({}mm)",
            c.release, c.actuation
        );
    }

    uninterrupted(|| {
        for group in TriggerGroup::group(&updated) {
            dev.set_trigger(&group.bitmap(), &group.config())?;
        }
        Ok(())
    })?;
    let after = dev.triggers(keys.iter().copied())?;
    if let Some(((pos, want), (_, got))) = updated.iter().zip(&after).find(|(a, b)| a.1 != b.1) {
        bail!("{pos}: value read back ({got:?}) differs from the value written ({want:?})");
    }
    println!("Set {} keys", keys.len());
    print(&after, false);
    Ok(())
}

fn columns(c: &TriggerConfig) -> [String; 5] {
    let mm = |t: Travel| format!("{t}mm");
    let dash = || "-".to_string();
    [
        c.mode.to_string(),
        mm(c.actuation),
        if c.mode.uses_release_point() {
            mm(c.release)
        } else {
            dash()
        },
        if c.mode.is_rapid() {
            mm(c.rt_press)
        } else {
            dash()
        },
        if c.mode.is_rapid() {
            mm(c.rt_release)
        } else {
            dash()
        },
    ]
}

const WIDTHS: [usize; 5] = [13, 11, 9, 10, 12];

fn row(cols: &[String; 5], keys: &str) {
    let cells: String = cols.iter().zip(WIDTHS).map(|(c, w)| pad(c, w)).collect();
    println!("{cells}{keys}");
}

fn print(configs: &[(KeyPos, TriggerConfig)], per_key: bool) {
    let header = ["Mode", "Actuation", "Release", "RT press", "RT release"].map(String::from);
    row(&header, "Keys");
    if per_key {
        for (pos, config) in configs {
            row(&columns(config), &pos.to_string());
        }
    } else {
        let mut groups = TriggerGroup::group(configs);
        // When showing all keys and most share the same settings, print that group last as "all others".
        let rest = (groups.len() > 1
            && configs.len() == LAYOUT.len()
            && groups[0].keys.len() > LAYOUT.len() / 2)
            .then(|| groups.remove(0));
        for group in &groups {
            let keys = match &group.keys[..] {
                [KeySel::All] => format!("all ({} keys)", LAYOUT.len()),
                keys => keys
                    .iter()
                    .map(|k| k.to_string())
                    .collect::<Vec<_>>()
                    .join(" "),
            };
            row(&columns(&group.config()), &keys);
        }
        if let Some(rest) = rest {
            row(
                &columns(&rest.config()),
                &format!("all others ({} keys)", rest.keys.len()),
            );
        }
    }
    if configs.iter().any(|(_, c)| c.mode.is_rapid()) {
        println!(
            "Note: RT sensitivity triggers only when the movement exceeds it, so keys actually react after moving 0.1mm more than the set value"
        );
    }
}
