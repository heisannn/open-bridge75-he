//! Where a calibration stands, as the page shows it. No GPUI.
//!
//! A calibration counts down first, because the keyboard measures the rest position of the keys
//! as soon as it starts: a key held then is measured pressed. While it runs, the keyboard
//! reports progress on its own; [`Calibration::report`] follows those reports.

use bridge75_core::he::{AllCalibration, KeyCalibration};
use bridge75_core::matrix::{KeyBitmap, KeyPos, LAYOUT};

use crate::session::CalibrationReport;

/// Seconds between asking for hands off the keys and starting.
pub const COUNTDOWN_SECONDS: u8 = 3;

/// What to calibrate.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Target {
    All,
    Key(KeyPos),
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Phase {
    /// Nothing has run on this keyboard yet.
    Ready,
    /// Waiting for hands to leave the keys. Nothing has been sent.
    Countdown {
        target: Target,
        seconds_left: u8,
    },
    /// The keyboard is calibrating. `stopping` once a stop of all keys has been sent.
    Running {
        target: Target,
        stopping: bool,
    },
    Ended(Outcome),
}

/// How the latest calibration ended.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum Outcome {
    /// A calibration of all keys ended; `stopped` if it was stopped before it finished.
    All {
        stopped: bool,
    },
    KeyDone(KeyPos),
    /// The key was not held at the bottom before the keyboard gave up.
    KeyTimedOut(KeyPos),
    /// The keyboard did not report the end in time.
    NoAnswer,
    /// The keyboard reported a state this app does not know.
    Unexpected(u8),
    /// The request to start could not be sent.
    Failed(String),
    Disconnected,
}

#[derive(Debug)]
pub struct Calibration {
    phase: Phase,
    /// Keys calibrated on this connection since the latest calibration of all keys started.
    /// The keyboard can't report calibration values, so keys it calibrated before are not here.
    calibrated: KeyBitmap,
    /// Counts the runs, so a late result of an earlier run is ignored.
    run: u64,
}

impl Default for Calibration {
    fn default() -> Self {
        Calibration {
            phase: Phase::Ready,
            calibrated: KeyBitmap::EMPTY,
            run: 0,
        }
    }
}

impl Calibration {
    pub fn phase(&self) -> &Phase {
        &self.phase
    }

    pub fn calibrated(&self) -> &KeyBitmap {
        &self.calibrated
    }

    /// The number of keys of the board that are in [`Calibration::calibrated`].
    pub fn calibrated_count(&self) -> usize {
        LAYOUT
            .iter()
            .filter(|key| self.calibrated.contains(key.pos))
            .count()
    }

    /// Counting down or running: the keyboard is, or is about to be, taking key presses as
    /// calibration.
    pub fn is_busy(&self) -> bool {
        matches!(self.phase, Phase::Countdown { .. } | Phase::Running { .. })
    }

    pub fn is_running(&self) -> bool {
        matches!(self.phase, Phase::Running { .. })
    }

    /// Identifies the current run for [`Calibration::start_failed`] and
    /// [`Calibration::no_answer`].
    pub fn run(&self) -> u64 {
        self.run
    }

    /// Starts the countdown to `target`. Does nothing while busy.
    pub fn begin(&mut self, target: Target) -> bool {
        if self.is_busy() {
            return false;
        }
        self.phase = Phase::Countdown {
            target,
            seconds_left: COUNTDOWN_SECONDS,
        };
        true
    }

    /// One second of the countdown has passed. Returns the target once it is time to send the
    /// request that starts it; the calibration is then running.
    pub fn tick(&mut self) -> Option<Target> {
        let Phase::Countdown {
            target,
            seconds_left,
        } = &mut self.phase
        else {
            return None;
        };
        *seconds_left = seconds_left.saturating_sub(1);
        if *seconds_left > 0 {
            return None;
        }
        let target = *target;
        self.run += 1;
        match target {
            Target::All => self.calibrated = KeyBitmap::EMPTY,
            // The keyboard forgets the key's rest position, so it is not calibrated until it
            // reports so.
            Target::Key(pos) => self.calibrated.remove(pos),
        }
        self.phase = Phase::Running {
            target,
            stopping: false,
        };
        Some(target)
    }

    /// Cancels the countdown. Nothing was sent, so the earlier result is gone but nothing else
    /// changes.
    pub fn cancel(&mut self) -> bool {
        if !matches!(self.phase, Phase::Countdown { .. }) {
            return false;
        }
        self.phase = Phase::Ready;
        true
    }

    /// Asks to stop a calibration of all keys. Returns whether the stop request should be sent.
    /// A calibration of one key can't be stopped; it ends on its own.
    pub fn stop(&mut self) -> bool {
        match &mut self.phase {
            Phase::Running {
                target: Target::All,
                stopping,
            } if !*stopping => {
                *stopping = true;
                true
            }
            _ => false,
        }
    }

    pub fn report(&mut self, report: CalibrationReport) {
        let Phase::Running { target, stopping } = self.phase else {
            return;
        };
        let outcome = match (target, report) {
            (Target::All, CalibrationReport::All { state, calibrated }) => {
                self.calibrated = calibrated;
                match state {
                    AllCalibration::Running => return,
                    AllCalibration::Finished => Outcome::All { stopped: stopping },
                    AllCalibration::Other(v) => Outcome::Unexpected(v),
                }
            }
            (Target::Key(pos), CalibrationReport::Key(state)) => match state {
                KeyCalibration::Running => return,
                KeyCalibration::Done => {
                    self.calibrated.insert(pos);
                    Outcome::KeyDone(pos)
                }
                KeyCalibration::TimedOut => Outcome::KeyTimedOut(pos),
                KeyCalibration::Other(v) => Outcome::Unexpected(v),
            },
            // A report of the other kind is not about this run.
            _ => return,
        };
        self.phase = Phase::Ended(outcome);
    }

    /// The request that starts run `run` failed.
    pub fn start_failed(&mut self, run: u64, error: String) {
        if run == self.run && self.is_running() {
            self.phase = Phase::Ended(Outcome::Failed(error));
        }
    }

    /// Run `run` has waited too long for the keyboard to report its end. A stopped calibration
    /// of all keys counts as stopped: the stop was sent, only its report is missing.
    pub fn no_answer(&mut self, run: u64) {
        if run != self.run {
            return;
        }
        if let Phase::Running { target, stopping } = self.phase {
            self.phase = Phase::Ended(match target {
                Target::All if stopping => Outcome::All { stopped: true },
                _ => Outcome::NoAnswer,
            });
        }
    }

    /// The keyboard went away. What was calibrated belonged to it.
    pub fn disconnected(&mut self) {
        self.calibrated = KeyBitmap::EMPTY;
        self.phase = if self.is_busy() {
            Phase::Ended(Outcome::Disconnected)
        } else {
            Phase::Ready
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(id: &str) -> KeyPos {
        id.parse().unwrap()
    }

    fn started(target: Target) -> Calibration {
        let mut calibration = Calibration::default();
        assert!(calibration.begin(target));
        for _ in 1..COUNTDOWN_SECONDS {
            assert_eq!(calibration.tick(), None);
        }
        assert_eq!(calibration.tick(), Some(target));
        calibration
    }

    fn all(state: AllCalibration, keys: &[&str]) -> CalibrationReport {
        CalibrationReport::All {
            state,
            calibrated: keys.iter().map(|k| pos(k)).collect(),
        }
    }

    #[test]
    fn the_countdown_runs_before_anything_is_sent() {
        let mut calibration = Calibration::default();
        calibration.begin(Target::All);
        assert_eq!(
            calibration.phase(),
            &Phase::Countdown {
                target: Target::All,
                seconds_left: COUNTDOWN_SECONDS
            }
        );
        assert!(calibration.is_busy() && !calibration.is_running());
        // Nothing else starts meanwhile.
        assert!(!calibration.begin(Target::Key(pos("a"))));
        assert!(calibration.cancel());
        assert_eq!(calibration.phase(), &Phase::Ready);
        assert_eq!(calibration.tick(), None);
    }

    #[test]
    fn all_keys_follow_the_reports_until_finished() {
        let mut calibration = started(Target::All);
        assert_eq!(calibration.run(), 1);
        calibration.report(all(AllCalibration::Running, &["a", "s"]));
        assert_eq!(calibration.calibrated_count(), 2);
        assert!(calibration.is_running());
        // A report about one key is not about this run.
        calibration.report(CalibrationReport::Key(KeyCalibration::Done));
        assert!(calibration.is_running());
        calibration.report(all(AllCalibration::Finished, &["a", "s", "d"]));
        assert_eq!(
            calibration.phase(),
            &Phase::Ended(Outcome::All { stopped: false })
        );
        assert_eq!(calibration.calibrated_count(), 3);
    }

    #[test]
    fn a_new_run_of_all_keys_starts_from_none() {
        let mut calibration = started(Target::Key(pos("a")));
        calibration.report(CalibrationReport::Key(KeyCalibration::Done));
        assert!(calibration.calibrated().contains(pos("a")));
        calibration.begin(Target::All);
        // Cancelling sends nothing, so the key stays calibrated.
        calibration.cancel();
        assert!(calibration.calibrated().contains(pos("a")));
        calibration.begin(Target::All);
        while calibration.tick().is_none() {}
        assert_eq!(calibration.calibrated_count(), 0);
    }

    #[test]
    fn stopping_all_keys_is_sent_once_and_ends_as_stopped() {
        let mut calibration = started(Target::All);
        assert!(calibration.stop());
        assert!(!calibration.stop());
        calibration.report(all(AllCalibration::Finished, &["a"]));
        assert_eq!(
            calibration.phase(),
            &Phase::Ended(Outcome::All { stopped: true })
        );

        // Without the keyboard's report, the stop still ends the run.
        let mut calibration = started(Target::All);
        calibration.stop();
        calibration.no_answer(calibration.run());
        assert_eq!(
            calibration.phase(),
            &Phase::Ended(Outcome::All { stopped: true })
        );
    }

    #[test]
    fn one_key_ends_done_or_timed_out() {
        let mut calibration = started(Target::Key(pos("a")));
        assert!(!calibration.stop());
        calibration.report(CalibrationReport::Key(KeyCalibration::Running));
        assert!(calibration.is_running());
        calibration.report(CalibrationReport::Key(KeyCalibration::TimedOut));
        assert_eq!(
            calibration.phase(),
            &Phase::Ended(Outcome::KeyTimedOut(pos("a")))
        );
        assert!(!calibration.calibrated().contains(pos("a")));
    }

    #[test]
    fn results_of_an_earlier_run_are_ignored() {
        let mut calibration = started(Target::Key(pos("a")));
        let first = calibration.run();
        calibration.report(CalibrationReport::Key(KeyCalibration::Done));
        calibration.begin(Target::Key(pos("s")));
        while calibration.tick().is_none() {}
        calibration.no_answer(first);
        calibration.start_failed(first, "late".into());
        assert!(calibration.is_running());
        calibration.no_answer(calibration.run());
        assert_eq!(calibration.phase(), &Phase::Ended(Outcome::NoAnswer));
    }

    #[test]
    fn unplugging_ends_a_run_and_forgets_the_keys() {
        let mut calibration = started(Target::All);
        calibration.report(all(AllCalibration::Running, &["a"]));
        calibration.disconnected();
        assert_eq!(calibration.phase(), &Phase::Ended(Outcome::Disconnected));
        assert_eq!(calibration.calibrated_count(), 0);
        // Once nothing runs, unplugging only clears what is shown.
        calibration.disconnected();
        assert_eq!(calibration.phase(), &Phase::Ready);
    }
}
