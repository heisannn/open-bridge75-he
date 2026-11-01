use std::collections::VecDeque;
use std::ffi::{CStr, CString};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use bridge75_core::he::{Debounce, PollingRate, TriggerConfig};
use bridge75_core::info::{DeviceInfo, FirmwareId, Heartbeat, SleepTimes};
use bridge75_core::keymap::{ADVANCED_SLOTS, Keymap, MACRO_SLOTS};
use bridge75_core::matrix::{COLS, KeyBitmap, KeyPos, ROWS};
use bridge75_core::response::{HeResponse, Response};
use bridge75_core::socd::{SocdPairs, SocdStatus};
use bridge75_core::{Packet, advanced::AdvancedKey, cmd, macros::Macro, request};
use hidapi::{HidApi, HidDevice};

use crate::{Error, Result};

/// Heartbeat interval. If heartbeats stop for 1.5 s, FW treats the driver as disconnected.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(1);
/// Read wait of the I/O thread (ms). Send latency stays below this.
const READ_POLL_MS: i32 = 2;
/// Maximum number of 0x23 rows held in the receive queue (about 5 s worth). The stream delivers 6000 packets/s.
const RAW_ROWS_CAPACITY: usize = 1 << 15;
/// Maximum number of non-0x23 packets held in the receive queue.
const OTHER_CAPACITY: usize = 1 << 14;
/// Number of attempts for a query.
const QUERY_ATTEMPTS: usize = 3;

/// A keyboard that was found.
#[derive(Clone, Debug)]
pub struct DeviceEntry {
    pub path: CString,
    pub serial: Option<String>,
    pub product: Option<String>,
    /// Visible as the bootloader (0416:3F01).
    pub bootloader: bool,
}

/// Lists connected Bridge75 HE keyboards (in normal mode, only the configuration interface).
pub fn list() -> Result<Vec<DeviceEntry>> {
    let api = HidApi::new()?;
    let mut entries: Vec<DeviceEntry> = Vec::new();
    for info in api.device_list() {
        if info.vendor_id() != bridge75_core::VENDOR_ID {
            continue;
        }
        let bootloader = match info.product_id() {
            bridge75_core::PRODUCT_ID => false,
            bridge75_core::BOOTLOADER_PRODUCT_ID => true,
            _ => continue,
        };
        let config_interface = info.usage_page() == bridge75_core::CONFIG_USAGE_PAGE
            || (info.usage_page() == 0
                && info.interface_number() == bridge75_core::CONFIG_INTERFACE);
        if !bootloader && !config_interface {
            continue;
        }
        // The bootloader may show up as several collections, so merge them into one.
        if bootloader && entries.iter().any(|e| e.bootloader) {
            continue;
        }
        entries.push(DeviceEntry {
            path: info.path().to_owned(),
            serial: info
                .serial_number()
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            product: info.product_string().map(str::to_owned),
            bootloader,
        });
    }
    Ok(entries)
}

/// Receive queue. Bounded so it never blocks the I/O thread.
///
/// When full, it drops only new 0x23 rows and old packets that were never read.
/// It is emptied with [`Device::drain`] before each request, so a response being waited for is never dropped.
#[derive(Default)]
struct Inbox {
    state: Mutex<InboxState>,
    ready: Condvar,
}

#[derive(Default)]
struct InboxState {
    /// 0x23 rows (numbered in arrival order).
    raw_rows: VecDeque<(u64, Packet)>,
    /// All other responses and notifications.
    other: VecDeque<(u64, Packet)>,
    next_seq: u64,
    /// The I/O thread has finished.
    closed: bool,
}

impl InboxState {
    /// Takes one packet in arrival order.
    fn pop(&mut self) -> Option<Packet> {
        let raw_first = match (self.raw_rows.front(), self.other.front()) {
            (Some((a, _)), Some((b, _))) => a < b,
            (raw, _) => raw.is_some(),
        };
        let queue = if raw_first {
            &mut self.raw_rows
        } else {
            &mut self.other
        };
        queue.pop_front().map(|(_, p)| p)
    }
}

impl Inbox {
    fn push(&self, packet: Packet) {
        let mut s = self.state.lock().unwrap();
        let seq = s.next_seq;
        s.next_seq += 1;
        if packet.cmd() == cmd::RAW_SENSOR {
            if s.raw_rows.len() >= RAW_ROWS_CAPACITY {
                return;
            }
            s.raw_rows.push_back((seq, packet));
        } else {
            if s.other.len() >= OTHER_CAPACITY {
                s.other.pop_front();
            }
            s.other.push_back((seq, packet));
        }
        drop(s);
        self.ready.notify_one();
    }

    /// `Ok(None)` if nothing arrives within `timeout`. `Err(())` if the I/O thread has finished and the queue is empty.
    fn recv_timeout(&self, timeout: Duration) -> Result<Option<Packet>, ()> {
        let deadline = Instant::now() + timeout;
        let mut s = self.state.lock().unwrap();
        loop {
            if let Some(packet) = s.pop() {
                return Ok(Some(packet));
            }
            if s.closed {
                return Err(());
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(None);
            }
            s = self.ready.wait_timeout(s, left).unwrap().0;
        }
    }

    fn clear(&self) {
        let mut s = self.state.lock().unwrap();
        s.raw_rows.clear();
        s.other.clear();
    }

    fn close(&self) {
        self.state.lock().unwrap().closed = true;
        self.ready.notify_all();
    }
}

/// An open keyboard. Closing it also stops the I/O thread.
pub struct Device {
    /// Dropped first on close. The I/O thread sees the disconnect, sends the remaining packets, then exits.
    outgoing: Option<Sender<Packet>>,
    inbox: Arc<Inbox>,
    failure: Arc<Mutex<Option<String>>>,
    worker: Option<JoinHandle<()>>,
    timeout: Duration,
}

impl Device {
    /// Opens the first keyboard found in normal mode.
    pub fn open_first() -> Result<Self> {
        let entries = list()?;
        match entries.iter().find(|e| !e.bootloader) {
            Some(entry) => Self::open_path(&entry.path),
            None if entries.iter().any(|e| e.bootloader) => Err(Error::Bootloader),
            None => Err(Error::NotFound),
        }
    }

    /// Opens the keyboard at `path`.
    ///
    /// Refuses any path that [`list`] does not report as a normal-mode configuration interface
    /// (so nothing is sent to another model or interface).
    pub fn open(path: &CStr) -> Result<Self> {
        let entries = list()?;
        let entry = entries
            .iter()
            .find(|e| e.path.as_bytes().eq_ignore_ascii_case(path.to_bytes()));
        match entry {
            Some(e) if !e.bootloader => Self::open_path(&e.path),
            Some(_) => Err(Error::Bootloader),
            None => Err(Error::NotConfigInterface(
                path.to_string_lossy().into_owned(),
            )),
        }
    }

    fn open_path(path: &CStr) -> Result<Self> {
        let api = HidApi::new()?;
        let hid = api.open_path(path)?;
        let (outgoing, outgoing_rx) = mpsc::channel();
        let inbox = Arc::new(Inbox::default());
        let failure = Arc::new(Mutex::new(None));
        let worker = {
            let (inbox, failure) = (inbox.clone(), failure.clone());
            std::thread::Builder::new()
                .name("bridge75-io".into())
                .spawn(move || {
                    if let Err(e) = io_loop(&hid, &outgoing_rx, &inbox) {
                        *failure.lock().unwrap() = Some(e.to_string());
                    }
                    inbox.close();
                })
                .expect("failed to spawn the I/O thread")
        };
        let device = Device {
            outgoing: Some(outgoing),
            inbox,
            failure,
            worker: Some(worker),
            timeout: Duration::from_secs(1),
        };
        // The 0x23 stream may still be running after an earlier abnormal exit. It has no response and is not saved, so always stop it.
        device.set_raw_stream(false)?;
        Ok(device)
    }

    /// How long to wait for each response (default 1 s).
    pub fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    fn disconnected(&self) -> Error {
        let reason = self.failure.lock().unwrap().clone();
        Error::Disconnected(reason.unwrap_or_else(|| "the I/O thread stopped".into()))
    }

    /// Puts a packet on the send queue.
    pub fn send(&self, packet: Packet) -> Result<()> {
        let outgoing = self.outgoing.as_ref().expect("cannot send after close");
        outgoing.send(packet).map_err(|_| self.disconnected())
    }

    /// The next received packet. `None` if nothing arrives within `timeout`.
    pub fn recv_timeout(&self, timeout: Duration) -> Result<Option<Packet>> {
        self.inbox
            .recv_timeout(timeout)
            .map_err(|()| self.disconnected())
    }

    /// The next received packet, parsed.
    pub fn next_response(&self, timeout: Duration) -> Result<Option<Response>> {
        Ok(self.recv_timeout(timeout)?.map(|p| Response::parse(&p)))
    }

    /// Discards packets already received.
    pub fn drain(&self) {
        self.inbox.clear();
    }

    /// Waits until a response arrives for which `pick` returns `Some`. Everything else received is discarded.
    pub fn wait<T>(&self, what: &str, mut pick: impl FnMut(&Response) -> Option<T>) -> Result<T> {
        let deadline = Instant::now() + self.timeout;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(Error::Timeout(what.into()));
            }
            if let Some(response) = self.next_response(left)?
                && let Some(value) = pick(&response)
            {
                return Ok(value);
            }
        }
    }

    /// Sends a packet and waits for the matching response.
    pub fn request<T>(
        &self,
        what: &str,
        packet: Packet,
        pick: impl FnMut(&Response) -> Option<T>,
    ) -> Result<T> {
        self.drain();
        self.send(packet)?;
        self.wait(what, pick)
    }

    /// A query. Resends if there is no response (do not use for writes).
    fn query<T>(
        &self,
        what: &str,
        packet: Packet,
        mut pick: impl FnMut(&Response) -> Option<T>,
    ) -> Result<T> {
        let mut last = None;
        for _ in 0..QUERY_ATTEMPTS {
            match self.request(what, packet, &mut pick) {
                Err(e @ Error::Timeout(_)) => last = Some(e),
                other => return other,
            }
        }
        Err(last.unwrap())
    }

    fn he_query<T>(
        &self,
        what: &str,
        packet: Packet,
        mut pick: impl FnMut(&HeResponse) -> Option<T>,
    ) -> Result<T> {
        self.query(what, packet, |r| match r {
            Response::He(he) => pick(he),
            _ => None,
        })
    }

    fn he_ack(&self, what: &str, packet: Packet) -> Result<()> {
        self.request(what, packet, |r| {
            matches!(r, Response::He(HeResponse::Ack)).then_some(())
        })
    }

    /// Sends write packets in order and waits for each one's response (same cmd in `b[1]`).
    fn write_all(&self, what: &str, packets: &[Packet]) -> Result<()> {
        for (i, packet) in packets.iter().enumerate() {
            let label = format!("{what} ({}/{})", i + 1, packets.len());
            let cmd = packet.cmd();
            self.drain();
            self.send(*packet)?;
            self.wait(&label, |r| match r {
                Response::Ack(c) if *c == cmd => Some(()),
                _ => None,
            })?;
        }
        Ok(())
    }

    // ---- Status / info ----

    pub fn heartbeat(&self) -> Result<Heartbeat> {
        self.query("heartbeat", request::heartbeat(), |r| match r {
            Response::Heartbeat(h) => Some(*h),
            _ => None,
        })
    }

    /// 0x0D device info (2 packets).
    pub fn device_info(&self) -> Result<DeviceInfo> {
        let what = "device info";
        let mut id = None;
        let mut build = None;
        self.query(what, request::device_info(), |r| {
            match r {
                Response::DeviceId(s) => id = Some(s.clone()),
                Response::BuildTime(b) => build = Some(b.clone()),
                _ => {}
            }
            (id.is_some() && build.is_some()).then_some(())
        })?;
        Ok(DeviceInfo {
            firmware: FirmwareId::parse(&id.unwrap())?,
            build: build.unwrap(),
        })
    }

    /// Checks that the 0x0D product name is this model ([`bridge75_core::PRODUCT_NAME`]). Call before writing.
    ///
    /// Returns [`Error::WrongProduct`] if it differs. Also fails if the device info cannot be read or parsed.
    pub fn check_product(&self) -> Result<DeviceInfo> {
        let info = self.device_info()?;
        if info.firmware.product != bridge75_core::PRODUCT_NAME {
            return Err(Error::WrongProduct {
                found: info.firmware.product,
            });
        }
        Ok(info)
    }

    pub fn serial_number(&self) -> Result<String> {
        self.query("SN", request::serial_number(), |r| match r {
            Response::SerialNumber(s) => Some(s.clone()),
            _ => None,
        })
    }

    /// 0x10 default keymap (default HID codes per row).
    pub fn default_keymap_usages(&self) -> Result<[[u8; COLS]; ROWS]> {
        let mut rows = [None; ROWS];
        self.query("default keymap", request::default_keymap(), |r| {
            if let Response::DefaultKeymapRow { row, usages } = r {
                rows[*row as usize] = Some(*usages);
            }
            rows.iter().all(Option::is_some).then_some(())
        })?;
        Ok(rows.map(Option::unwrap))
    }

    /// 0x0E fixed-function key mask.
    pub fn fixed_fn_mask(&self) -> Result<KeyBitmap> {
        self.query(
            "fixed-function key mask",
            request::fixed_fn_mask(),
            |r| match r {
                Response::FixedFnMask(m) => Some(*m),
                _ => None,
            },
        )
    }

    /// When the stored value is 0, FW reports the default (lighting 60 s / keyboard 1800 s).
    pub fn sleep_times(&self) -> Result<SleepTimes> {
        self.query("sleep time", request::sleep_query(), |r| match r {
            Response::SleepTimes(t) => Some(*t),
            _ => None,
        })
    }

    pub fn set_sleep_times(&self, times: SleepTimes) -> Result<()> {
        self.request("set sleep time", request::sleep_set(times)?, |r| {
            matches!(r, Response::Ack(cmd::SYSTEM) | Response::SleepTimes(_)).then_some(())
        })
    }

    // ---- HE settings ----

    pub fn max_travel(&self) -> Result<u8> {
        self.he_query("max travel", request::he_max_travel(), |r| match r {
            HeResponse::MaxTravel(v) => Some(*v),
            _ => None,
        })
    }

    /// Trigger settings of one key.
    pub fn trigger(&self, pos: KeyPos) -> Result<TriggerConfig> {
        let bytes = self.he_query(
            &format!("trigger settings of {pos}"),
            request::he_query_key(pos),
            |r| match r {
                HeResponse::KeyTrigger(b) => Some(*b),
                _ => None,
            },
        )?;
        Ok(TriggerConfig::from_wire(bytes)?)
    }

    /// Trigger settings of several keys (queried one key at a time).
    pub fn triggers(
        &self,
        keys: impl IntoIterator<Item = KeyPos>,
    ) -> Result<Vec<(KeyPos, TriggerConfig)>> {
        keys.into_iter()
            .map(|pos| Ok((pos, self.trigger(pos)?)))
            .collect()
    }

    pub fn set_trigger(&self, keys: &KeyBitmap, config: &TriggerConfig) -> Result<()> {
        self.he_ack(
            "set trigger settings",
            request::he_set_trigger(keys, config)?,
        )
    }

    /// Resets the trigger settings of all keys to the defaults.
    ///
    /// FW also discards the rest calibration values and relearns them from about 64 ms of steady readings. Do not send this while holding a key.
    pub fn reset_triggers(&self) -> Result<()> {
        self.he_ack("reset trigger settings", request::he_reset_triggers())
    }

    pub fn polling_rate(&self) -> Result<PollingRate> {
        let khz = self.he_query(
            "polling rate",
            request::he_polling_rate_query(),
            |r| match r {
                HeResponse::PollingRate(v) => Some(*v),
                _ => None,
            },
        )?;
        Ok(PollingRate::try_from(khz)?)
    }

    pub fn set_polling_rate(&self, rate: PollingRate) -> Result<()> {
        let what = "set polling rate";
        let got = self.request(what, request::he_set_polling_rate(rate), |r| match r {
            Response::He(HeResponse::PollingRate(v)) => Some(*v),
            _ => None,
        })?;
        expect_value(what, rate.khz(), got)
    }

    pub fn debounce(&self) -> Result<Debounce> {
        let value = self.he_query("debounce", request::he_debounce_query(), |r| match r {
            HeResponse::Debounce(v) => Some(*v),
            _ => None,
        })?;
        Ok(Debounce::new(value)?)
    }

    pub fn set_debounce(&self, value: Debounce) -> Result<()> {
        let what = "set debounce";
        let got = self.request(what, request::he_set_debounce(value), |r| match r {
            Response::He(HeResponse::Debounce(v)) => Some(*v),
            _ => None,
        })?;
        expect_value(what, value.value(), got)
    }

    /// Sets the keys to monitor for live travel reports. From then on, sub1 notifications ([`HeResponse::Travel`]) arrive.
    ///
    /// If heartbeats stop for 1.5 s, FW clears the monitored keys.
    pub fn set_monitor(&self, keys: &KeyBitmap) -> Result<()> {
        self.he_ack("set monitored keys", request::he_monitor(keys))
    }

    pub fn clear_monitor(&self) -> Result<()> {
        self.he_ack("clear monitored keys", request::he_clear_monitor())
    }

    /// Starts calibration of one key. The result arrives as [`HeResponse::CalibrateKey`].
    ///
    /// FW discards the rest calibration value and relearns it, so send this with your hands off the keys.
    pub fn start_calibrate_key(&self, pos: KeyPos) -> Result<()> {
        self.send(request::he_calibrate_key(pos))
    }

    /// Starts calibration of all keys. Progress arrives as [`HeResponse::CalibrateAll`].
    ///
    /// FW discards the rest calibration values of all keys and relearns them, so send this with your hands off the keys.
    pub fn start_calibrate_all(&self) -> Result<()> {
        self.send(request::he_calibrate_all_start())
    }

    pub fn abort_calibrate_all(&self) -> Result<()> {
        self.send(request::he_calibrate_all_abort())
    }

    /// Starts / stops the 0x23 raw sensor value stream. There is no response.
    ///
    /// It keeps running when heartbeats stop, so always stop it when done.
    pub fn set_raw_stream(&self, enable: bool) -> Result<()> {
        self.send(request::raw_sensor_stream(enable))
    }

    // ---- Writes (settings that cannot be read from the keyboard) ----

    /// Writes both the base layer and the Fn layer.
    pub fn write_keymaps(&self, base: &Keymap, fn_layer: &Keymap) -> Result<()> {
        self.write_all("write keymap", &request::keymaps(base, fn_layer))
    }

    /// Writes macros to all 10 slots.
    pub fn write_macros(&self, slots: &[Option<Macro>; MACRO_SLOTS as usize]) -> Result<()> {
        self.write_all("write macros", &request::macros(slots)?)
    }

    /// Writes advanced keys to all 40 slots.
    pub fn write_advanced_keys(
        &self,
        slots: &[Option<AdvancedKey>; ADVANCED_SLOTS as usize],
    ) -> Result<()> {
        self.write_all("write advanced keys", &request::advanced_keys(slots)?)
    }

    // ---- SOCD ----

    /// Queries the pairs. Bit 7 of byte 0 (enabled flag) cannot be read this way; read it with [`Device::socd_status`].
    pub fn socd_pairs(&self) -> Result<SocdPairs> {
        let mut pairs = SocdPairs::default();
        let mut pages = [false; 2];
        self.query("SOCD pairs", request::socd_query(), |r| {
            if let Response::SocdPairs { page, pairs: got } = r
                && let Some(seen) = pages.get_mut(*page as usize)
            {
                *seen = true;
                pairs[*page as usize * 10..*page as usize * 10 + 10].copy_from_slice(got);
            }
            pages.iter().all(|&p| p).then_some(())
        })?;
        Ok(pairs)
    }

    pub fn socd_status(&self) -> Result<SocdStatus> {
        self.query("SOCD status", request::socd_status(), |r| match r {
            Response::SocdStatus(s) => Some(*s),
            _ => None,
        })
    }

    /// Writes all 20 pairs. FW keeps the existing enabled flag (bit 7) of each pair.
    pub fn write_socd(&self, pairs: &SocdPairs) -> Result<()> {
        for (i, packet) in request::socd_write(pairs).into_iter().enumerate() {
            self.request(&format!("write SOCD pairs ({}/2)", i + 1), packet, is_socd)?;
        }
        Ok(())
    }

    /// Global on/off. The enabled flag (bit 7) of every pair is set to this value too.
    ///
    /// FW does not respond to this request, so the status is queried afterwards to confirm.
    pub fn set_socd_global(&self, on: bool) -> Result<()> {
        self.send(request::socd_set_global(on))?;
        let status = self.socd_status()?;
        if status.global != on {
            return Err(Error::Unexpected {
                what: "set SOCD global on/off".into(),
                detail: format!(
                    "set to {}, but read back {}",
                    on_off(on),
                    on_off(status.global)
                ),
            });
        }
        Ok(())
    }
}

fn on_off(on: bool) -> &'static str {
    if on { "ON" } else { "OFF" }
}

fn is_socd(r: &Response) -> Option<()> {
    matches!(
        r,
        Response::Ack(cmd::SOCD) | Response::SocdPairs { .. } | Response::SocdStatus(_)
    )
    .then_some(())
}

fn expect_value(what: &str, sent: u8, got: u8) -> Result<()> {
    if sent == got {
        Ok(())
    } else {
        Err(Error::Unexpected {
            what: what.into(),
            detail: format!("sent {sent}, got {got}"),
        })
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // Once the sender is dropped, the I/O thread finishes sending the queued packets, then sees the disconnect and exits.
        drop(self.outgoing.take());
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// The thread that owns the HID handle. Loops over the send queue, heartbeat, and receiving.
///
/// When the [`Device`] is closed (the sender is dropped), it sends every packet still in the queue
/// (such as stopping the stream) before exiting.
fn io_loop(hid: &HidDevice, outgoing: &Receiver<Packet>, inbox: &Inbox) -> Result<()> {
    let mut last_heartbeat: Option<Instant> = None;
    let mut buf = [0u8; 65];
    loop {
        loop {
            match outgoing.try_recv() {
                Ok(packet) => {
                    hid.write(&packet.0)?;
                }
                Err(TryRecvError::Empty) => break,
                // The disconnect is seen only after every remaining packet has been received.
                Err(TryRecvError::Disconnected) => return Ok(()),
            }
        }
        if last_heartbeat.is_none_or(|t| t.elapsed() >= HEARTBEAT_INTERVAL) {
            hid.write(&request::heartbeat().0)?;
            last_heartbeat = Some(Instant::now());
        }
        let n = hid.read_timeout(&mut buf, READ_POLL_MS)?;
        if let Some(packet) = Packet::from_report(&buf[..n]) {
            inbox.push(packet);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(cmd: u8, tag: u8) -> Packet {
        Packet::new(cmd).with_data(&[tag])
    }

    #[test]
    fn inbox_keeps_order() {
        let inbox = Inbox::default();
        inbox.push(packet(cmd::RAW_SENSOR, 0));
        inbox.push(packet(cmd::HE, 1));
        inbox.push(packet(cmd::RAW_SENSOR, 2));
        inbox.push(packet(cmd::SOCD, 3));
        let got: Vec<u8> = (0..4)
            .map(|_| inbox.recv_timeout(Duration::ZERO).unwrap().unwrap().data()[0])
            .collect();
        assert_eq!(got, [0, 1, 2, 3]);
        assert_eq!(inbox.recv_timeout(Duration::from_millis(1)), Ok(None));
        inbox.close();
        assert_eq!(inbox.recv_timeout(Duration::ZERO), Err(()));
    }

    #[test]
    fn inbox_drops_only_raw_rows_when_full() {
        let inbox = Inbox::default();
        for _ in 0..RAW_ROWS_CAPACITY + 100 {
            inbox.push(packet(cmd::RAW_SENSOR, 0));
        }
        // Even when flooded with 0x23, responses are not dropped.
        inbox.push(packet(cmd::HE, 7));
        let mut raw = 0;
        let mut other = Vec::new();
        while let Ok(Some(p)) = inbox.recv_timeout(Duration::ZERO) {
            if p.cmd() == cmd::RAW_SENSOR {
                raw += 1;
            } else {
                other.push(p.data()[0]);
            }
        }
        assert_eq!(raw, RAW_ROWS_CAPACITY);
        assert_eq!(other, [7]);
    }

    #[test]
    fn inbox_bounds_unread_packets() {
        let inbox = Inbox::default();
        for i in 0..OTHER_CAPACITY + 3 {
            inbox.push(packet(cmd::HEARTBEAT, i as u8));
        }
        // Drops the oldest unread packets first and keeps the new ones (the response being waited for).
        let first = inbox.recv_timeout(Duration::ZERO).unwrap().unwrap();
        assert_eq!(first.data()[0], 3);
        inbox.clear();
        assert_eq!(inbox.recv_timeout(Duration::ZERO), Ok(None));
    }
}
