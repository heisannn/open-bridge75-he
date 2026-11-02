//! The debounce, polling rate and sleep time of the connected keyboard.
//!
//! They are read when the keyboard connects and dropped when it goes away. Every change is
//! shown at once and written straight away. If a write fails, its setting is read back from
//! the keyboard before anything else reaches it, the page shows that, and
//! [`DocumentEvent::WriteFailed`] is emitted. Only when that read fails too is everything read
//! again.

use bridge75_core::info::SleepTimes;
use bridge75_device::Device;
use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Subscription, Task};

use super::settings::{Change, GeneralSettings, Setting};
use crate::document::{DocumentEvent, Loadable};
use crate::session::{ConnectionId, RequestError, Session, SessionEvent};

/// How a write ended, as seen from the session thread.
#[derive(Debug)]
enum WriteOutcome {
    Written,
    Failed {
        error: String,
        /// The setting as read back afterwards, if that worked.
        actual: Option<Change>,
    },
}

fn read_settings(device: &Device) -> bridge75_device::Result<GeneralSettings> {
    Ok(GeneralSettings {
        debounce: device.debounce()?,
        polling_rate: device.polling_rate()?,
        sleep: device.sleep_times()?,
    })
}

fn read(device: &Device, setting: Setting) -> bridge75_device::Result<Change> {
    Ok(match setting {
        Setting::Debounce => Change::Debounce(device.debounce()?),
        Setting::PollingRate => Change::PollingRate(device.polling_rate()?),
        Setting::Sleep => Change::Sleep(device.sleep_times()?),
    })
}

/// Sets the sleep times. The keyboard only acknowledges them, so they are read back to confirm.
fn write_sleep(device: &Device, sleep: SleepTimes) -> Result<(), String> {
    device
        .set_sleep_times(sleep)
        .and_then(|()| device.sleep_times())
        .map_err(|e| e.to_string())
        .and_then(|actual| {
            (actual == sleep).then_some(()).ok_or_else(|| {
                format!(
                    "set sleep time: the keyboard has {} s instead of {} s",
                    actual.keyboard, sleep.keyboard
                )
            })
        })
}

/// Writes `change`. When that fails, reads its setting back before anything else reaches the
/// keyboard: the failed request may have taken effect.
fn write(device: &Device, change: Change) -> WriteOutcome {
    let written = match change {
        Change::Debounce(debounce) => device.set_debounce(debounce).map_err(|e| e.to_string()),
        Change::PollingRate(rate) => device.set_polling_rate(rate).map_err(|e| e.to_string()),
        Change::Sleep(sleep) => write_sleep(device, sleep),
    };
    match written {
        Ok(()) => WriteOutcome::Written,
        Err(error) => WriteOutcome::Failed {
            error,
            actual: read(device, change.setting()).ok(),
        },
    }
}

pub struct GeneralDocument {
    session: Entity<Session>,
    /// The connection the settings came from.
    connection: Option<ConnectionId>,
    /// Counts every load and disconnect, so results meant for settings that were read again
    /// are ignored.
    epoch: u64,
    settings: Loadable<GeneralSettings>,
    /// Writes still in flight, indexed by [`Setting::ix`]. What a failed write reads back is
    /// shown only once the last write of its setting ends, so it does not undo a newer change
    /// on the screen.
    writes: [usize; Setting::COUNT],
    _load: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DocumentEvent> for GeneralDocument {}

impl GeneralDocument {
    pub fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::Connected(connection) => this.load(*connection, cx),
            SessionEvent::Disconnected => this.clear(cx),
            SessionEvent::Travel(_) | SessionEvent::Calibration(_) => {}
        });
        let mut document = GeneralDocument {
            session: session.clone(),
            connection: None,
            epoch: 0,
            settings: Loadable::Idle,
            writes: [0; Setting::COUNT],
            _load: None,
            _subscriptions: vec![subscription],
        };
        if let Some(connection) = session.read(cx).connection() {
            document.load(connection, cx);
        }
        document
    }

    pub fn settings(&self) -> &Loadable<GeneralSettings> {
        &self.settings
    }

    /// Reads the settings again from the current connection.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if let Some(connection) = self.session.read(cx).connection() {
            self.load(connection, cx);
        }
    }

    fn load(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        self.epoch += 1;
        self.connection = Some(connection);
        self.settings = Loadable::Loading;
        self.writes = [0; Setting::COUNT];
        cx.notify();
        let epoch = self.epoch;
        let read = self
            .session
            .update(cx, |session, cx| session.run(read_settings, cx));
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| this.finish_load(epoch, result, cx));
        }));
    }

    fn finish_load(
        &mut self,
        epoch: u64,
        result: Result<GeneralSettings, RequestError>,
        cx: &mut Context<Self>,
    ) {
        if self.epoch != epoch {
            return;
        }
        self.settings = match result {
            Ok(settings) => Loadable::Loaded(settings),
            Err(e) => Loadable::Failed(e.to_string().into()),
        };
        cx.notify();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.epoch += 1;
        self.connection = None;
        self.settings = Loadable::Idle;
        self.writes = [0; Setting::COUNT];
        self._load = None;
        cx.notify();
    }

    /// Takes `change` and writes it.
    pub fn set(&mut self, change: Change, cx: &mut Context<Self>) {
        let Some(settings) = self.settings.loaded_mut() else {
            return;
        };
        if !settings.apply(change) {
            return;
        }
        cx.notify();
        if self.connection.is_none() {
            return;
        }
        let (epoch, setting) = (self.epoch, change.setting());
        self.writes[setting.ix()] += 1;
        let write = self.session.update(cx, |session, cx| {
            session.run(move |device| Ok(write(device, change)), cx)
        });
        cx.spawn(async move |this, cx| {
            let result = write.await;
            let _ = this.update(cx, |this, cx| this.finish_write(epoch, setting, result, cx));
        })
        .detach();
    }

    fn finish_write(
        &mut self,
        epoch: u64,
        setting: Setting,
        result: Result<WriteOutcome, RequestError>,
        cx: &mut Context<Self>,
    ) {
        if self.epoch != epoch {
            return;
        }
        let writes = &mut self.writes[setting.ix()];
        *writes = writes.saturating_sub(1);
        let last = *writes == 0;
        let (error, actual) = match result {
            Ok(WriteOutcome::Written) => return,
            Ok(WriteOutcome::Failed { error, actual }) => (error, actual),
            // The job never ran, so it changed nothing; the session reports the disconnect,
            // which clears the settings.
            Err(e) => {
                cx.emit(DocumentEvent::WriteFailed(e.to_string().into()));
                return;
            }
        };
        match actual {
            Some(actual) if last => {
                if let Some(settings) = self.settings.loaded_mut() {
                    settings.apply(actual);
                }
            }
            // A newer write of the setting is on its way and settles it.
            Some(_) => {}
            // Nothing is known about the setting any more, so everything is read again.
            None => {
                if let Some(connection) = self.connection {
                    self.load(connection, cx);
                }
            }
        }
        cx.emit(DocumentEvent::WriteFailed(error.into()));
        cx.notify();
    }
}

/// Creates the document for `session`.
pub fn new(session: Entity<Session>, cx: &mut App) -> Entity<GeneralDocument> {
    cx.new(|cx| GeneralDocument::new(session, cx))
}

#[cfg(test)]
impl GeneralDocument {
    /// A document showing `settings` as read from connection 1 of a session with no keyboard
    /// behind it, so nothing a test does can reach a real keyboard.
    pub fn for_test(settings: GeneralSettings, cx: &mut Context<Self>) -> Self {
        let session = cx.new(|_| Session::detached());
        let mut document = GeneralDocument::new(session, cx);
        document.connection = Some(ConnectionId::for_test(1));
        document.finish_load(document.epoch, Ok(settings), cx);
        document
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use bridge75_core::he::{Debounce, PollingRate};
    use gpui_kit::TestAppContext;

    use super::*;

    fn defaults() -> GeneralSettings {
        GeneralSettings::DEFAULT
    }

    fn debounce(value: u8) -> Change {
        Change::Debounce(Debounce::new(value).unwrap())
    }

    fn failed(actual: Option<Change>) -> Result<WriteOutcome, RequestError> {
        Ok(WriteOutcome::Failed {
            error: "no response".into(),
            actual,
        })
    }

    /// A loaded document whose writes never run: these tests hand in the results themselves.
    fn loaded(cx: &mut TestAppContext) -> (Entity<GeneralDocument>, Rc<RefCell<Vec<String>>>) {
        let document = cx.new(|cx| GeneralDocument::for_test(defaults(), cx));
        let failures = Rc::new(RefCell::new(Vec::new()));
        let seen = failures.clone();
        cx.update(|cx| {
            cx.subscribe(&document, move |_, event, _| {
                if let DocumentEvent::WriteFailed(message) = event {
                    seen.borrow_mut().push(message.to_string());
                }
            })
            .detach()
        });
        (document, failures)
    }

    fn shown(document: &GeneralDocument) -> GeneralSettings {
        *document.settings().loaded().unwrap()
    }

    #[gpui_kit::test]
    fn a_failed_write_shows_what_the_keyboard_has(cx: &mut TestAppContext) {
        let (document, failures) = loaded(cx);
        document.update(cx, |document, cx| {
            document.set(Change::PollingRate(PollingRate::Khz1), cx);
            assert_eq!(document.writes, [0, 1, 0]);
            let epoch = document.epoch;
            let actual = Change::PollingRate(PollingRate::Khz8);
            document.finish_write(epoch, Setting::PollingRate, failed(Some(actual)), cx);
            assert_eq!(shown(document).polling_rate, PollingRate::Khz8);
            assert_eq!(document.writes, [0; 3]);
        });
        assert_eq!(*failures.borrow(), ["no response"]);
    }

    #[gpui_kit::test]
    fn a_failed_write_leaves_a_newer_change_on_the_screen(cx: &mut TestAppContext) {
        let (document, failures) = loaded(cx);
        document.update(cx, |document, cx| {
            document.set(debounce(2), cx);
            document.set(debounce(1), cx);
            let epoch = document.epoch;
            // The first write failed, and the keyboard kept 3; the second is on its way.
            document.finish_write(epoch, Setting::Debounce, failed(Some(debounce(3))), cx);
            assert_eq!(shown(document).debounce.value(), 1);
            document.finish_write(epoch, Setting::Debounce, Ok(WriteOutcome::Written), cx);
            assert_eq!(shown(document).debounce.value(), 1);
        });
        assert_eq!(failures.borrow().len(), 1);
    }

    #[gpui_kit::test]
    fn a_failed_read_back_reads_everything_again(cx: &mut TestAppContext) {
        let (document, failures) = loaded(cx);
        document.update(cx, |document, cx| {
            document.set(Change::Sleep(defaults().sleep_after(600)), cx);
            let epoch = document.epoch;
            document.finish_write(epoch, Setting::Sleep, failed(None), cx);
            assert!(matches!(document.settings, Loadable::Loading));
            // Results for the settings shown before are ignored.
            document.finish_load(epoch, Ok(defaults()), cx);
            assert!(matches!(document.settings, Loadable::Loading));
        });
        assert_eq!(failures.borrow().len(), 1);
    }

    #[gpui_kit::test]
    fn a_job_that_never_ran_changes_nothing(cx: &mut TestAppContext) {
        let (document, failures) = loaded(cx);
        document.update(cx, |document, cx| {
            document.set(debounce(0), cx);
            let epoch = document.epoch;
            document.finish_write(
                epoch,
                Setting::Debounce,
                Err(RequestError::NotConnected),
                cx,
            );
            assert_eq!(shown(document).debounce.value(), 0);
        });
        assert_eq!(failures.borrow().len(), 1);
    }

    #[gpui_kit::test]
    fn unchanged_values_are_not_written(cx: &mut TestAppContext) {
        let (document, _) = loaded(cx);
        document.update(cx, |document, cx| {
            document.set(debounce(3), cx);
            document.set(Change::Sleep(SleepTimes::DEFAULT), cx);
            assert_eq!(document.writes, [0; 3]);
        });
    }

    #[gpui_kit::test]
    fn results_after_the_keyboard_goes_away_are_ignored(cx: &mut TestAppContext) {
        let (document, failures) = loaded(cx);
        document.update(cx, |document, cx| {
            document.set(debounce(0), cx);
            let epoch = document.epoch;
            document.clear(cx);
            document.finish_write(epoch, Setting::Debounce, failed(None), cx);
            assert!(matches!(document.settings, Loadable::Idle));
            document.set(debounce(1), cx);
            assert_eq!(document.writes, [0; 3]);
        });
        assert!(failures.borrow().is_empty());
    }
}
