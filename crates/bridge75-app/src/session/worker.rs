//! The thread that owns the keyboard connection. It does not depend on GPUI.
//!
//! [`Device`] requests drain the receive queue before they wait, so only one thread may talk to
//! the keyboard. This thread searches for the keyboard while none is open, runs jobs one at a
//! time while it is open, and forwards live travel reports, calibration reports and heartbeat
//! changes between jobs. Reports that arrive while a job waits for its response are dropped by
//! the device layer.

use std::ffi::CString;
use std::sync::Arc;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bridge75_core::he::{AllCalibration, KeyCalibration, Travel};
use bridge75_core::info::{DeviceInfo, Heartbeat};
use bridge75_core::matrix::{KeyBitmap, KeyPos};
use bridge75_core::request;
use bridge75_core::response::{HeResponse, Response};
use bridge75_device::{Device, DeviceEntry, Error};
use gpui_kit::base::async_util::Sender;

/// How often to look for the keyboard while none is connected.
const SEARCH_INTERVAL: Duration = Duration::from_secs(1);
/// How long to wait for a job before checking the receive queue again.
const PUMP_INTERVAL: Duration = Duration::from_millis(4);
/// How often travel reports are forwarded (about one display frame).
const TRAVEL_INTERVAL: Duration = Duration::from_millis(16);

/// Identifies one open connection. A new one is issued every time the keyboard is opened, so
/// work meant for an earlier connection never reaches a keyboard plugged in later.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ConnectionId(u64);

/// What was read from the keyboard when it was opened.
#[derive(Clone, Debug)]
pub struct KeyboardInfo {
    pub device: DeviceInfo,
    pub serial: Option<String>,
    pub heartbeat: Heartbeat,
}

/// A calibration report the keyboard sends on its own once calibration has started.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CalibrationReport {
    /// Calibration of one key (0x21 sub7).
    Key(KeyCalibration),
    /// Calibration of all keys (0x21 sub8), with the keys calibrated so far in this run.
    All {
        state: AllCalibration,
        calibrated: KeyBitmap,
    },
}

/// Why no keyboard is connected.
#[derive(Clone, PartialEq, Eq, Debug)]
pub enum SearchState {
    NotFound,
    /// Only the bootloader (FW update mode) is visible.
    Bootloader,
    /// A keyboard was found but could not be opened. Opening is tried again.
    Failed(String),
    /// A device did not identify itself as a Bridge75 HE. It is left alone until it is
    /// unplugged.
    Rejected(String),
}

pub enum Event {
    Searching(SearchState),
    Connected {
        connection: ConnectionId,
        info: Arc<KeyboardInfo>,
    },
    Disconnected,
    /// Latest travel per key since the previous report.
    Travel(Vec<(KeyPos, Travel)>),
    Calibration(CalibrationReport),
    /// A heartbeat response that differs from the previous one, such as the calibrated flag
    /// changing after calibration.
    Heartbeat(Heartbeat),
}

/// A unit of work for the open keyboard. It receives `None` when its connection is gone.
pub type Job<D = Device> = Box<dyn FnOnce(Option<&D>) + Send>;

pub struct Command<D = Device> {
    pub connection: ConnectionId,
    pub job: Job<D>,
}

/// Runs `command` on `device` if it is meant for the `open` connection; otherwise the job
/// learns that its connection is gone.
fn dispatch<D>(command: Command<D>, open: Option<(ConnectionId, &D)>) {
    match open {
        Some((connection, device)) if command.connection == connection => {
            (command.job)(Some(device))
        }
        _ => (command.job)(None),
    }
}

/// Handle to the worker thread.
///
/// Dropping it closes the job queue and waits for the thread. Jobs already queued still run
/// first (those whose connection is gone are told so), then the thread closes the keyboard.
pub struct Worker {
    commands: Option<mpsc::Sender<Command>>,
    thread: Option<JoinHandle<()>>,
}

impl Worker {
    pub fn spawn(events: Sender<Event>) -> Self {
        let (commands, rx) = mpsc::channel();
        let thread = std::thread::Builder::new()
            .name("bridge75-session".into())
            .spawn(move || run(&rx, &events))
            .expect("failed to spawn the session thread");
        Worker {
            commands: Some(commands),
            thread: Some(thread),
        }
    }

    /// Queues a job. Returns the command back if the thread has stopped.
    pub fn send(&self, command: Command) -> Result<(), Command> {
        let commands = self.commands.as_ref().expect("worker is running");
        commands.send(command).map_err(|e| e.0)
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        drop(self.commands.take());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The app has quit (the command or event channel is closed).
struct Quit;

fn run(commands: &mpsc::Receiver<Command>, events: &Sender<Event>) {
    let mut next_id = 0;
    let mut rejected = Rejected::default();
    loop {
        let Ok((device, info)) = search(commands, events, &mut rejected) else {
            return;
        };
        next_id += 1;
        let connection = ConnectionId(next_id);
        let heartbeat = info.heartbeat;
        let info = Arc::new(info);
        if emit(events, Event::Connected { connection, info }).is_err() {
            return;
        }
        let mut state = Served::new(heartbeat);
        match serve(&device, connection, commands, events, &mut state) {
            Ok(()) => {
                drop(device);
                if emit(events, Event::Disconnected).is_err() {
                    return;
                }
            }
            Err(Quit) => {
                // Monitored keys are cleared by the FW too once heartbeats stop; this is quicker.
                let _ = device.send(request::he_clear_monitor());
                // A run left going would keep taking key presses as calibration.
                if state.calibrating_all {
                    let _ = device.abort_calibrate_all();
                }
                return;
            }
        }
    }
}

fn emit(events: &Sender<Event>, event: Event) -> Result<(), Quit> {
    events.send_blocking(event).map_err(|_| Quit)
}

/// Waits until a keyboard can be opened. Jobs that arrive meanwhile are rejected.
fn search(
    commands: &mpsc::Receiver<Command>,
    events: &Sender<Event>,
    rejected: &mut Rejected,
) -> Result<(Device, KeyboardInfo), Quit> {
    let mut last: Option<SearchState> = None;
    loop {
        let state = match open(rejected) {
            Ok(found) => return Ok(found),
            Err(state) => state,
        };
        if last.as_ref() != Some(&state) {
            emit(events, Event::Searching(state.clone()))?;
            last = Some(state);
        }
        let deadline = Instant::now() + SEARCH_INTERVAL;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match commands.recv_timeout(left) {
                Ok(command) => dispatch(command, None),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return Err(Quit),
            }
        }
    }
}

/// Devices that failed identification, with the reason. Each is skipped until it disappears
/// from the device list (it was unplugged).
#[derive(Default)]
struct Rejected(Vec<(CString, String)>);

impl Rejected {
    fn contains(&self, path: &CString) -> bool {
        self.0.iter().any(|(p, _)| p == path)
    }
}

/// What to do with the devices that are plugged in.
#[derive(Debug, PartialEq, Eq)]
enum Choice<'a> {
    /// Open the device at this path.
    Open(&'a CString),
    Wait(SearchState),
}

/// Picks the first normal-mode device not rejected before. Forgets rejected devices that are
/// gone, so plugging one in again gives it another try.
fn choose<'a>(entries: &'a [DeviceEntry], rejected: &mut Rejected) -> Choice<'a> {
    rejected
        .0
        .retain(|(path, _)| entries.iter().any(|e| e.path == *path));
    if let Some(entry) = entries
        .iter()
        .find(|e| !e.bootloader && !rejected.contains(&e.path))
    {
        return Choice::Open(&entry.path);
    }
    Choice::Wait(match rejected.0.first() {
        Some((_, reason)) => SearchState::Rejected(reason.clone()),
        None if entries.iter().any(|e| e.bootloader) => SearchState::Bootloader,
        None => SearchState::NotFound,
    })
}

fn open(rejected: &mut Rejected) -> Result<(Device, KeyboardInfo), SearchState> {
    let failed = |e: Error| SearchState::Failed(e.to_string());
    let entries = bridge75_device::list().map_err(failed)?;
    let path = match choose(&entries, rejected) {
        Choice::Open(path) => path,
        Choice::Wait(state) => return Err(state),
    };
    let device = Device::open(path).map_err(|e| match e {
        Error::Bootloader => SearchState::Bootloader,
        e => failed(e),
    })?;
    // Opening has already sent 0x23 (stop the raw stream), and identification sends 0x0D. A
    // device that fails identification gets nothing more, now or on later searches.
    let info = match device.check_product() {
        Ok(info) => info,
        Err(e) => {
            let reason = e.to_string();
            rejected.0.push((path.clone(), reason.clone()));
            return Err(SearchState::Rejected(reason));
        }
    };
    let serial = device.serial_number().ok().filter(|s| !s.is_empty());
    let heartbeat = device.heartbeat().map_err(failed)?;
    Ok((
        device,
        KeyboardInfo {
            device: info,
            serial,
            heartbeat,
        },
    ))
}

/// What [`serve`] keeps track of across jobs.
struct Served {
    /// The latest heartbeat, so only changes are forwarded.
    heartbeat: Heartbeat,
    /// Whether the keyboard last reported a calibration of all keys as running.
    calibrating_all: bool,
}

impl Served {
    fn new(heartbeat: Heartbeat) -> Self {
        Served {
            heartbeat,
            calibrating_all: false,
        }
    }

    /// The event to forward for `response`, if any. Travel reports are batched separately.
    fn event_for(&mut self, response: Response) -> Option<Event> {
        match response {
            Response::He(HeResponse::CalibrateKey(state)) => {
                Some(Event::Calibration(CalibrationReport::Key(state)))
            }
            Response::He(HeResponse::CalibrateAll { state, calibrated }) => {
                self.calibrating_all = state == AllCalibration::Running;
                Some(Event::Calibration(CalibrationReport::All {
                    state,
                    calibrated,
                }))
            }
            Response::Heartbeat(heartbeat) if heartbeat != self.heartbeat => {
                self.heartbeat = heartbeat;
                Some(Event::Heartbeat(heartbeat))
            }
            _ => None,
        }
    }
}

/// Runs jobs and forwards notifications until the keyboard goes away.
fn serve(
    device: &Device,
    connection: ConnectionId,
    commands: &mpsc::Receiver<Command>,
    events: &Sender<Event>,
    state: &mut Served,
) -> Result<(), Quit> {
    let mut travel = TravelBatch::default();
    loop {
        match commands.recv_timeout(PUMP_INTERVAL) {
            Ok(command) => dispatch(command, Some((connection, device))),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Err(Quit),
        }
        loop {
            match device.next_response(Duration::ZERO) {
                Ok(Some(Response::He(HeResponse::Travel { pos, travel: t }))) => {
                    travel.push(pos, t)
                }
                Ok(Some(response)) => {
                    if let Some(event) = state.event_for(response) {
                        emit(events, event)?;
                    }
                }
                Ok(None) => break,
                Err(_) => return Ok(()),
            }
        }
        if let Some(batch) = travel.take_due(Instant::now()) {
            emit(events, Event::Travel(batch))?;
        }
    }
}

/// Collects travel reports and releases them at most once per [`TRAVEL_INTERVAL`].
#[derive(Default)]
struct TravelBatch {
    latest: Vec<(KeyPos, Travel)>,
    last_sent: Option<Instant>,
}

impl TravelBatch {
    fn push(&mut self, pos: KeyPos, travel: Travel) {
        match self.latest.iter_mut().find(|(p, _)| *p == pos) {
            Some(entry) => entry.1 = travel,
            None => self.latest.push((pos, travel)),
        }
    }

    fn take_due(&mut self, now: Instant) -> Option<Vec<(KeyPos, Travel)>> {
        let due = self
            .last_sent
            .is_none_or(|t| now.duration_since(t) >= TRAVEL_INTERVAL);
        if self.latest.is_empty() || !due {
            return None;
        }
        self.last_sent = Some(now);
        Some(std::mem::take(&mut self.latest))
    }
}

#[cfg(test)]
impl ConnectionId {
    pub fn for_test(id: u64) -> Self {
        ConnectionId(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pos(id: &str) -> KeyPos {
        id.parse().unwrap()
    }

    /// A command for `connection` whose job reports the device it was given.
    fn probe(connection: u64, seen: &mpsc::Sender<Option<&'static str>>) -> Command<&'static str> {
        let seen = seen.clone();
        Command {
            connection: ConnectionId(connection),
            job: Box::new(move |device| seen.send(device.copied()).unwrap()),
        }
    }

    #[test]
    fn jobs_for_another_connection_are_rejected() {
        let (tx, seen) = mpsc::channel();
        let keyboard = "keyboard";
        dispatch(probe(2, &tx), Some((ConnectionId(2), &keyboard)));
        dispatch(probe(1, &tx), Some((ConnectionId(2), &keyboard)));
        dispatch(probe(3, &tx), Some((ConnectionId(2), &keyboard)));
        dispatch(probe(2, &tx), None);
        let seen: Vec<_> = seen.try_iter().collect();
        assert_eq!(seen, [Some("keyboard"), None, None, None]);
    }

    fn entry(path: &str, bootloader: bool) -> DeviceEntry {
        DeviceEntry {
            path: CString::new(path).unwrap(),
            serial: None,
            product: None,
            bootloader,
        }
    }

    #[test]
    fn rejected_devices_are_skipped_until_unplugged() {
        let other = entry("other", false);
        let keyboard = entry("keyboard", false);
        let mut rejected = Rejected::default();
        rejected
            .0
            .push((other.path.clone(), "not a Bridge75 HE".into()));

        let both = [other.clone(), keyboard];
        assert_eq!(choose(&both, &mut rejected), Choice::Open(&both[1].path));
        let alone = [other];
        assert_eq!(
            choose(&alone, &mut rejected),
            Choice::Wait(SearchState::Rejected("not a Bridge75 HE".into()))
        );

        // Unplugged: forgotten, so it is tried again when it comes back.
        assert_eq!(
            choose(&[], &mut rejected),
            Choice::Wait(SearchState::NotFound)
        );
        assert_eq!(choose(&alone, &mut rejected), Choice::Open(&alone[0].path));
    }

    #[test]
    fn the_bootloader_is_never_opened() {
        let mut rejected = Rejected::default();
        assert_eq!(
            choose(&[entry("boot", true)], &mut rejected),
            Choice::Wait(SearchState::Bootloader)
        );
    }

    #[test]
    fn only_heartbeat_changes_are_forwarded() {
        let calibrated = Heartbeat {
            win_lock: false,
            calibrated: true,
        };
        let mut state = Served::new(calibrated);
        assert!(state.event_for(Response::Heartbeat(calibrated)).is_none());
        let cleared = Heartbeat {
            calibrated: false,
            ..calibrated
        };
        assert!(matches!(
            state.event_for(Response::Heartbeat(cleared)),
            Some(Event::Heartbeat(h)) if h == cleared
        ));
        assert!(state.event_for(Response::Heartbeat(cleared)).is_none());
    }

    #[test]
    fn a_running_calibration_of_all_keys_is_tracked() {
        let mut state = Served::new(Heartbeat {
            win_lock: false,
            calibrated: false,
        });
        let report = |state| {
            Response::He(HeResponse::CalibrateAll {
                state,
                calibrated: KeyBitmap::EMPTY,
            })
        };
        assert!(state.event_for(report(AllCalibration::Running)).is_some());
        assert!(state.calibrating_all);
        assert!(state.event_for(report(AllCalibration::Finished)).is_some());
        assert!(!state.calibrating_all);
        assert!(matches!(
            state.event_for(Response::He(HeResponse::CalibrateKey(KeyCalibration::Done))),
            Some(Event::Calibration(CalibrationReport::Key(
                KeyCalibration::Done
            )))
        ));
    }

    #[test]
    fn travel_batch_keeps_the_latest_value_per_key() {
        let mut batch = TravelBatch::default();
        let start = Instant::now();
        batch.push(pos("a"), Travel::from_raw(5));
        batch.push(pos("s"), Travel::from_raw(7));
        batch.push(pos("a"), Travel::from_raw(9));
        assert_eq!(
            batch.take_due(start),
            Some(vec![
                (pos("a"), Travel::from_raw(9)),
                (pos("s"), Travel::from_raw(7))
            ])
        );
        assert_eq!(batch.take_due(start), None);
    }

    #[test]
    fn travel_batch_waits_for_the_interval() {
        let mut batch = TravelBatch::default();
        let start = Instant::now();
        batch.push(pos("a"), Travel::from_raw(1));
        assert!(batch.take_due(start).is_some());
        batch.push(pos("a"), Travel::from_raw(2));
        assert_eq!(batch.take_due(start + TRAVEL_INTERVAL / 2), None);
        assert_eq!(
            batch.take_due(start + TRAVEL_INTERVAL),
            Some(vec![(pos("a"), Travel::from_raw(2))])
        );
    }
}
