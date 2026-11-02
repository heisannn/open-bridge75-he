//! The connection to the keyboard.
//!
//! [`Session`] owns the worker thread and mirrors its connection state. Other features talk to
//! the keyboard only through [`Session::run`], which queues a job on that thread.

mod worker;

use std::fmt;
use std::sync::Arc;

use bridge75_core::he::Travel;
use bridge75_core::matrix::KeyPos;
use bridge75_device::Device;
use gpui_kit::base::async_util::unbounded;
use gpui_kit::{App, AppContext as _, Context, Entity, EventEmitter, Task};

pub use worker::{CalibrationReport, ConnectionId, KeyboardInfo, SearchState};
use worker::{Command, Event, Worker};

#[derive(Clone, Debug)]
pub enum SessionState {
    /// No keyboard is connected.
    Searching(SearchState),
    Connected {
        connection: ConnectionId,
        info: Arc<KeyboardInfo>,
    },
    /// The connection thread stopped unexpectedly. Nothing reaches the keyboard until the app
    /// is restarted.
    Stopped,
}

#[derive(Clone, Debug)]
pub enum SessionEvent {
    Connected(ConnectionId),
    Disconnected,
    /// Latest travel of the keys that moved. Only monitored keys report travel.
    Travel(Arc<[(KeyPos, Travel)]>),
    /// Progress or the end of a calibration started on the current connection.
    Calibration(CalibrationReport),
}

#[derive(Debug)]
pub enum RequestError {
    /// The keyboard was not connected, or the connection the job was meant for is gone.
    NotConnected,
    Device(bridge75_device::Error),
}

impl fmt::Display for RequestError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RequestError::NotConnected => f.write_str("the keyboard is not connected"),
            RequestError::Device(e) => e.fmt(f),
        }
    }
}

pub struct Session {
    state: SessionState,
    /// `None` once the app quits or the thread has stopped.
    worker: Option<Worker>,
    _events: Task<()>,
}

impl EventEmitter<SessionEvent> for Session {}

impl Session {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let (tx, rx) = unbounded();
        let worker = Worker::spawn(tx);
        let events = cx.spawn(async move |this, cx| {
            while let Ok(event) = rx.recv().await {
                if this
                    .update(cx, |this, cx| this.handle_event(event, cx))
                    .is_err()
                {
                    return;
                }
            }
            // The thread dropped its end of the channel: it quit or panicked.
            let _ = this.update(cx, |this, cx| this.handle_stop(cx));
        });
        // Stop the thread before the process exits so the keyboard is closed cleanly.
        cx.on_app_quit(|this, _| {
            this.worker.take();
            async {}
        })
        .detach();
        Session {
            state: SessionState::Searching(SearchState::NotFound),
            worker: Some(worker),
            _events: events,
        }
    }

    /// A session without a connection thread, so tests never reach a real keyboard.
    #[cfg(test)]
    pub fn detached() -> Self {
        Session {
            state: SessionState::Searching(SearchState::NotFound),
            worker: None,
            _events: Task::ready(()),
        }
    }

    /// A session that looks connected but has no connection thread: every job fails with
    /// [`RequestError::NotConnected`].
    #[cfg(test)]
    pub fn connected_for_test() -> Self {
        use bridge75_core::info::{BuildTime, DeviceInfo, FirmwareId, Heartbeat};
        let info = KeyboardInfo {
            device: DeviceInfo {
                firmware: FirmwareId::parse("M484,22,KB,OW,75ALHE,V1.13.10").unwrap(),
                build: BuildTime {
                    date: "Nov 19 2024".into(),
                    time: "10:36:50".into(),
                },
            },
            serial: None,
            heartbeat: Heartbeat {
                win_lock: false,
                calibrated: true,
            },
        };
        Session {
            state: SessionState::Connected {
                connection: ConnectionId::for_test(1),
                info: Arc::new(info),
            },
            ..Session::detached()
        }
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    pub fn connection(&self) -> Option<ConnectionId> {
        match &self.state {
            SessionState::Connected { connection, .. } => Some(*connection),
            SessionState::Searching(_) | SessionState::Stopped => None,
        }
    }

    fn handle_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Searching(state) => {
                self.state = SessionState::Searching(state);
                cx.notify();
            }
            Event::Connected { connection, info } => {
                self.state = SessionState::Connected { connection, info };
                cx.emit(SessionEvent::Connected(connection));
                cx.notify();
            }
            Event::Disconnected => {
                self.state = SessionState::Searching(SearchState::NotFound);
                cx.emit(SessionEvent::Disconnected);
                cx.notify();
            }
            Event::Travel(batch) => cx.emit(SessionEvent::Travel(batch.into())),
            Event::Calibration(report) => cx.emit(SessionEvent::Calibration(report)),
            Event::Heartbeat(heartbeat) => {
                if let SessionState::Connected { info, .. } = &mut self.state {
                    Arc::make_mut(info).heartbeat = heartbeat;
                    cx.notify();
                }
            }
        }
    }

    /// The thread is gone. Unless the app is quitting, that was not meant to happen.
    fn handle_stop(&mut self, cx: &mut Context<Self>) {
        if self.worker.take().is_none() {
            return;
        }
        let was_connected = self.connection().is_some();
        self.state = SessionState::Stopped;
        if was_connected {
            cx.emit(SessionEvent::Disconnected);
        }
        cx.notify();
    }

    /// Runs `job` on the keyboard of the current connection.
    ///
    /// The job runs on the session thread after the jobs queued before it. It fails with
    /// [`RequestError::NotConnected`] if that connection is gone by then.
    pub fn run<T: Send + 'static>(
        &self,
        job: impl FnOnce(&Device) -> bridge75_device::Result<T> + Send + 'static,
        cx: &mut Context<Self>,
    ) -> Task<Result<T, RequestError>> {
        let (Some(connection), Some(worker)) = (self.connection(), self.worker.as_ref()) else {
            return Task::ready(Err(RequestError::NotConnected));
        };
        let (tx, rx) = unbounded();
        let command = Command {
            connection,
            job: Box::new(move |device: Option<&Device>| {
                let result = match device {
                    Some(device) => job(device).map_err(RequestError::Device),
                    None => Err(RequestError::NotConnected),
                };
                let _ = tx.send_blocking(result);
            }),
        };
        if worker.send(command).is_err() {
            return Task::ready(Err(RequestError::NotConnected));
        }
        cx.spawn(async move |_, _| rx.recv().await.unwrap_or(Err(RequestError::NotConnected)))
    }
}

/// Creates the application's session.
pub fn new(cx: &mut App) -> Entity<Session> {
    cx.new(Session::new)
}
