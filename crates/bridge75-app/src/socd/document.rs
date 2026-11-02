//! The SOCD settings of the connected keyboard.
//!
//! The pairs and the switch are read when the keyboard connects and dropped when it goes away.
//! Every change is shown at once and written straight away. The keyboard takes all 20 pairs in
//! one write, so changes made while a pair write is in flight are written together once it
//! ends. If a write fails, what it was meant to change is read back from the keyboard before
//! anything else reaches it, the page shows that, and [`DocumentEvent::WriteFailed`] is
//! emitted. Only when that read fails too is everything read again.
//!
//! Keys assigned to toggle SOCD flip the enabled flags on the keyboard without telling the
//! app, so [`SocdDocument::refresh_status`] reads them again.

use bridge75_core::socd::{SocdPair, SocdPairs, SocdStatus};
use bridge75_device::Device;
use gpui_kit::{Context, Entity, EventEmitter, Subscription, Task};

use super::settings::{Change, SocdSettings};
use crate::document::{DocumentEvent, Loadable};
use crate::session::{ConnectionId, RequestError, Session, SessionEvent};

/// How a write ended, as seen from the session thread.
#[derive(Debug)]
enum WriteOutcome<T, A> {
    Written(T),
    Failed {
        error: String,
        /// What the write was meant to change, as read back afterwards, if that worked.
        actual: Option<A>,
    },
}

type PairsOutcome = WriteOutcome<(), SocdPairs>;
type SwitchOutcome = WriteOutcome<SocdStatus, SocdStatus>;

/// Writes `pairs`. When that fails, reads them back before anything else reaches the keyboard:
/// part of the write, or even the failed request, may have taken effect.
fn write_pairs(device: &Device, pairs: &SocdPairs) -> PairsOutcome {
    match device.write_socd(pairs) {
        Ok(()) => WriteOutcome::Written(()),
        Err(e) => WriteOutcome::Failed {
            error: e.to_string(),
            actual: device.socd_pairs().ok(),
        },
    }
}

/// Sets the switch. The keyboard does not answer it, so its status is read to confirm, and read
/// again after a failure.
fn write_switch(device: &Device, on: bool) -> SwitchOutcome {
    match device
        .set_socd_global(on)
        .and_then(|()| device.socd_status())
    {
        Ok(status) => WriteOutcome::Written(status),
        Err(e) => WriteOutcome::Failed {
            error: e.to_string(),
            actual: device.socd_status().ok(),
        },
    }
}

pub struct SocdDocument {
    session: Entity<Session>,
    /// The connection the settings came from.
    connection: Option<ConnectionId>,
    /// Counts every load and disconnect, so results meant for settings that were read again
    /// are ignored.
    epoch: u64,
    settings: Loadable<SocdSettings>,
    /// Counts every change of the pairs, so a finished write can tell whether newer changes
    /// are waiting.
    revision: u64,
    /// The newest revision of the pairs the keyboard has taken.
    written: u64,
    /// The revision of the pairs being written, while a write is in flight.
    writing: Option<u64>,
    /// Counts switch writes, so a failed one can be told apart from the others.
    switch_write: u64,
    /// Switch writes still in flight. The status they read back is shown only once the last
    /// one ends, so an older one does not undo a newer change on the screen.
    switch_writes: usize,
    _load: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DocumentEvent> for SocdDocument {}

impl SocdDocument {
    pub fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::Connected(connection) => this.load(*connection, cx),
            SessionEvent::Disconnected => this.clear(cx),
            SessionEvent::Travel(_) => {}
        });
        let mut document = SocdDocument {
            session: session.clone(),
            connection: None,
            epoch: 0,
            settings: Loadable::Idle,
            revision: 0,
            written: 0,
            writing: None,
            switch_write: 0,
            switch_writes: 0,
            _load: None,
            _subscriptions: vec![subscription],
        };
        if let Some(connection) = session.read(cx).connection() {
            document.load(connection, cx);
        }
        document
    }

    pub fn settings(&self) -> &Loadable<SocdSettings> {
        &self.settings
    }

    pub fn has_undo_history(&self) -> bool {
        self.settings
            .loaded()
            .is_some_and(SocdSettings::has_undo_history)
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
        self.writing = None;
        self.written = self.revision;
        self.switch_writes = 0;
        cx.notify();
        let epoch = self.epoch;
        let read = self.session.update(cx, |session, cx| {
            session.run(
                |device| Ok((device.socd_pairs()?, device.socd_status()?)),
                cx,
            )
        });
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| this.finish_load(epoch, result, cx));
        }));
    }

    fn finish_load(
        &mut self,
        epoch: u64,
        result: Result<(SocdPairs, SocdStatus), RequestError>,
        cx: &mut Context<Self>,
    ) {
        if self.epoch != epoch {
            return;
        }
        self.settings = match result {
            Ok((pairs, status)) => Loadable::Loaded(SocdSettings::new(pairs, status)),
            Err(e) => Loadable::Failed(e.to_string().into()),
        };
        cx.notify();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.epoch += 1;
        self.connection = None;
        self.settings = Loadable::Idle;
        self.writing = None;
        self.switch_writes = 0;
        self._load = None;
        cx.notify();
    }

    /// Reads the switch and the enabled flags again, which keys on the keyboard can change.
    pub fn refresh_status(&mut self, cx: &mut Context<Self>) {
        if self.settings.loaded().is_none() || self.switch_writes > 0 {
            return;
        }
        let epoch = self.epoch;
        let read = self.session.update(cx, |session, cx| {
            session.run(|device| device.socd_status(), cx)
        });
        cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                // A failed read changes nothing; a lost connection is reported by the session.
                if let Ok(status) = result {
                    this.take_status(epoch, status, cx);
                }
            });
        })
        .detach();
    }

    fn take_status(&mut self, epoch: u64, status: SocdStatus, cx: &mut Context<Self>) {
        if self.epoch != epoch || self.switch_writes > 0 {
            return;
        }
        if let Some(settings) = self.settings.loaded_mut() {
            settings.set_status(status);
            cx.notify();
        }
    }

    /// Puts `pair` in slot `index` and writes the pairs.
    pub fn set_pair(&mut self, index: usize, pair: SocdPair, cx: &mut Context<Self>) {
        let revision = self.revision + 1;
        let Some(settings) = self.settings.loaded_mut() else {
            return;
        };
        if settings.set_pair(index, pair, revision) {
            self.pairs_changed(cx);
        }
    }

    /// Adds a pair in the first free slot. Returns the slot, or `None` if every slot is used.
    pub fn add_pair(&mut self, cx: &mut Context<Self>) -> Option<usize> {
        let state = self.settings.loaded()?.state();
        let index = state.first_free()?;
        let pair = state.next_new_pair();
        self.set_pair(index, pair, cx);
        Some(index)
    }

    /// Empties slot `index`. The other pairs keep their slots, which keys that toggle a pair
    /// refer to.
    pub fn remove_pair(&mut self, index: usize, cx: &mut Context<Self>) {
        self.set_pair(index, SocdPair::default(), cx);
    }

    /// Turns the switch on or off.
    pub fn set_switch(&mut self, on: bool, cx: &mut Context<Self>) {
        let write = self.switch_write + 1;
        let Some(settings) = self.settings.loaded_mut() else {
            return;
        };
        if settings.set_switch(on, write) {
            self.write_switch(on, cx);
        }
    }

    /// Puts back what the newest change replaced and writes it.
    pub fn undo(&mut self, cx: &mut Context<Self>) {
        let Some(settings) = self.settings.loaded_mut() else {
            return;
        };
        match settings.undo() {
            Some(Change::Pairs { .. }) => self.pairs_changed(cx),
            Some(Change::Switch { previous, .. }) => self.write_switch(previous, cx),
            None => {}
        }
    }

    fn pairs_changed(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        self.write_pairs(cx);
        cx.notify();
    }

    /// Starts writing the current pairs, unless a write is in flight; that one starts the
    /// next write when it ends.
    fn write_pairs(&mut self, cx: &mut Context<Self>) {
        let Some(settings) = self.settings.loaded() else {
            return;
        };
        if self.writing.is_some() || self.written == self.revision {
            return;
        }
        let (epoch, revision, pairs) = (self.epoch, self.revision, *settings.state().pairs());
        self.writing = Some(revision);
        let write = self.session.update(cx, |session, cx| {
            session.run(move |device| Ok(write_pairs(device, &pairs)), cx)
        });
        cx.spawn(async move |this, cx| {
            let result = write.await;
            let _ = this.update(cx, |this, cx| {
                this.finish_pairs_write(epoch, revision, result, cx)
            });
        })
        .detach();
    }

    fn finish_pairs_write(
        &mut self,
        epoch: u64,
        revision: u64,
        result: Result<PairsOutcome, RequestError>,
        cx: &mut Context<Self>,
    ) {
        if self.epoch != epoch {
            return;
        }
        self.writing = None;
        let (error, actual) = match result {
            Ok(WriteOutcome::Written(())) => {
                self.written = revision;
                // Changes made during the write.
                self.write_pairs(cx);
                return;
            }
            Ok(WriteOutcome::Failed { error, actual }) => (error, actual),
            Err(e) => return self.not_written(e, cx),
        };
        match actual {
            // The keyboard has these pairs. Changes waiting to be written are dropped with
            // the failed ones, as the page shows what the keyboard has.
            Some(pairs) => {
                if let Some(settings) = self.settings.loaded_mut() {
                    settings.recover_pairs(pairs, self.written);
                }
                self.written = self.revision;
            }
            None => self.reload_after_failure(cx),
        }
        cx.emit(DocumentEvent::WriteFailed(error.into()));
        cx.notify();
    }

    fn write_switch(&mut self, on: bool, cx: &mut Context<Self>) {
        cx.notify();
        if self.connection.is_none() {
            return;
        }
        let epoch = self.epoch;
        self.switch_write += 1;
        let id = self.switch_write;
        self.switch_writes += 1;
        let write = self.session.update(cx, |session, cx| {
            session.run(move |device| Ok(write_switch(device, on)), cx)
        });
        cx.spawn(async move |this, cx| {
            let result = write.await;
            let _ = this.update(cx, |this, cx| {
                this.finish_switch_write(epoch, id, result, cx)
            });
        })
        .detach();
    }

    fn finish_switch_write(
        &mut self,
        epoch: u64,
        id: u64,
        result: Result<SwitchOutcome, RequestError>,
        cx: &mut Context<Self>,
    ) {
        if self.epoch != epoch {
            return;
        }
        self.switch_writes = self.switch_writes.saturating_sub(1);
        let (error, actual) = match result {
            Ok(WriteOutcome::Written(status)) => return self.take_status(epoch, status, cx),
            Ok(WriteOutcome::Failed { error, actual }) => (error, actual),
            Err(e) => return self.not_written(e, cx),
        };
        if let Some(settings) = self.settings.loaded_mut() {
            settings.forget_switch_write(id);
        }
        match actual {
            Some(status) => self.take_status(epoch, status, cx),
            None => self.reload_after_failure(cx),
        }
        cx.emit(DocumentEvent::WriteFailed(error.into()));
        cx.notify();
    }

    /// Nothing is known about the keyboard's settings any more, so they are read again.
    fn reload_after_failure(&mut self, cx: &mut Context<Self>) {
        if let Some(connection) = self.connection {
            self.load(connection, cx);
        }
    }

    /// The job never ran, so it changed nothing; the session reports the disconnect, which
    /// clears the settings.
    fn not_written(&mut self, error: RequestError, cx: &mut Context<Self>) {
        cx.emit(DocumentEvent::WriteFailed(error.to_string().into()));
    }
}

/// Creates the SOCD document for `session`.
pub fn new(session: Entity<Session>, cx: &mut gpui_kit::App) -> Entity<SocdDocument> {
    use gpui_kit::AppContext as _;

    cx.new(|cx| SocdDocument::new(session, cx))
}

#[cfg(test)]
impl SocdDocument {
    /// A document showing `pairs` and `status` as read from connection 1 of a session with no
    /// keyboard behind it, so nothing a test does can reach a real keyboard.
    pub fn for_test(pairs: SocdPairs, status: SocdStatus, cx: &mut Context<Self>) -> Self {
        use gpui_kit::AppContext as _;

        let session = cx.new(|_| Session::detached());
        let mut document = SocdDocument::new(session, cx);
        document.connection = Some(ConnectionId::for_test(1));
        document.finish_load(document.epoch, Ok((pairs, status)), cx);
        document
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use bridge75_core::hid::Usage;
    use bridge75_core::socd::SocdMode;
    use gpui_kit::{AppContext as _, TestAppContext};

    use super::*;

    fn off() -> SocdStatus {
        SocdStatus {
            global: false,
            enabled_mask: 0,
        }
    }

    fn on() -> SocdStatus {
        SocdStatus {
            global: true,
            enabled_mask: 0xFFFFF,
        }
    }

    fn failed<T, A>(actual: Option<A>) -> Result<WriteOutcome<T, A>, RequestError> {
        Ok(WriteOutcome::Failed {
            error: "no response".into(),
            actual,
        })
    }

    /// A loaded document whose writes never run: these tests hand in the results themselves.
    fn loaded(cx: &mut TestAppContext) -> (Entity<SocdDocument>, Rc<RefCell<Vec<String>>>) {
        let document = cx.new(|cx| SocdDocument::for_test(SocdPairs::default(), off(), cx));
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

    fn pair(document: &SocdDocument, index: usize) -> SocdPair {
        document.settings().loaded().unwrap().state().pair(index)
    }

    fn switch(document: &SocdDocument) -> bool {
        document.settings().loaded().unwrap().state().switch()
    }

    #[gpui_kit::test]
    fn changes_during_a_write_are_written_together_afterwards(cx: &mut TestAppContext) {
        let (document, _) = loaded(cx);
        document.update(cx, |document, cx| {
            assert_eq!(document.add_pair(cx), Some(0));
            assert_eq!(document.writing, Some(1));
            assert_eq!(document.add_pair(cx), Some(1));
            document.set_pair(
                1,
                SocdPair {
                    a: Usage(0x52),
                    b: Usage(0x51),
                    ..pair(document, 1)
                },
                cx,
            );
            assert_eq!(document.writing, Some(1));

            let epoch = document.epoch;
            document.finish_pairs_write(epoch, 1, Ok(WriteOutcome::Written(())), cx);
            assert_eq!(document.written, 1);
            assert_eq!(document.writing, Some(3));
            document.finish_pairs_write(epoch, 3, Ok(WriteOutcome::Written(())), cx);
            assert_eq!(document.writing, None);
        });
    }

    #[gpui_kit::test]
    fn a_failed_write_shows_what_the_keyboard_has_and_keeps_older_changes(cx: &mut TestAppContext) {
        let (document, failures) = loaded(cx);
        document.update(cx, |document, cx| {
            let epoch = document.epoch;
            document.add_pair(cx);
            document.finish_pairs_write(epoch, 1, Ok(WriteOutcome::Written(())), cx);
            let kept = pair(document, 0);
            document.add_pair(cx);
            document.add_pair(cx);

            // The second write failed; the keyboard has only the first pair.
            let mut actual = SocdPairs::default();
            actual[0] = kept;
            document.finish_pairs_write(epoch, 2, failed(Some(actual)), cx);
            assert!(document.settings().loaded().is_some());
            assert!(pair(document, 1).is_empty() && pair(document, 2).is_empty());
            assert_eq!(document.writing, None);
            assert_eq!(document.written, document.revision);

            // Adding the first pair can still be undone.
            document.undo(cx);
            assert!(pair(document, 0).is_empty());
            assert!(!document.has_undo_history());
        });
        assert_eq!(*failures.borrow(), ["no response"]);
    }

    #[gpui_kit::test]
    fn a_failed_read_back_reads_everything_again(cx: &mut TestAppContext) {
        let (document, failures) = loaded(cx);
        document.update(cx, |document, cx| {
            document.add_pair(cx);
            let epoch = document.epoch;
            document.finish_pairs_write(epoch, 1, failed(None), cx);
            assert!(matches!(document.settings, Loadable::Loading));
            // A result for the settings shown before is ignored.
            document.finish_pairs_write(epoch, 1, Ok(WriteOutcome::Written(())), cx);
            document.finish_load(epoch, Ok((SocdPairs::default(), off())), cx);
            assert!(matches!(document.settings, Loadable::Loading));

            document.load(ConnectionId::for_test(1), cx);
            let epoch = document.epoch;
            document.finish_load(epoch, Ok((SocdPairs::default(), off())), cx);
            document.set_switch(true, cx);
            document.finish_switch_write(epoch, 1, failed(None), cx);
            assert!(matches!(document.settings, Loadable::Loading));
        });
        assert_eq!(failures.borrow().len(), 2);
    }

    #[gpui_kit::test]
    fn a_failed_switch_write_shows_the_switch_the_keyboard_has(cx: &mut TestAppContext) {
        let (document, failures) = loaded(cx);
        document.update(cx, |document, cx| {
            document.set_switch(true, cx);
            let epoch = document.epoch;
            document.finish_switch_write(epoch, 1, failed(Some(off())), cx);
            assert!(!switch(document));
            assert!(!document.has_undo_history());
        });
        assert_eq!(failures.borrow().len(), 1);
    }

    #[gpui_kit::test]
    fn a_job_that_never_ran_changes_nothing(cx: &mut TestAppContext) {
        let (document, failures) = loaded(cx);
        document.update(cx, |document, cx| {
            document.add_pair(cx);
            let epoch = document.epoch;
            document.finish_pairs_write(epoch, 1, Err(RequestError::NotConnected), cx);
            assert!(!pair(document, 0).is_empty());
            assert!(document.has_undo_history());
        });
        assert_eq!(failures.borrow().len(), 1);
    }

    #[gpui_kit::test]
    fn removing_a_pair_keeps_the_others_in_their_slots(cx: &mut TestAppContext) {
        let (document, _) = loaded(cx);
        document.update(cx, |document, cx| {
            document.add_pair(cx);
            document.add_pair(cx);
            let second = pair(document, 1);
            document.remove_pair(0, cx);
            assert!(pair(document, 0).is_empty());
            assert_eq!(pair(document, 1), second);
            assert_eq!(document.add_pair(cx), Some(0));
        });
    }

    #[gpui_kit::test]
    fn the_status_read_back_waits_for_the_last_switch_write(cx: &mut TestAppContext) {
        let (document, _) = loaded(cx);
        document.update(cx, |document, cx| {
            document.set_switch(true, cx);
            document.set_switch(false, cx);
            assert_eq!(document.switch_writes, 2);
            let epoch = document.epoch;
            // The first write read back "on"; the second one is still on its way.
            document.finish_switch_write(epoch, 1, Ok(WriteOutcome::Written(on())), cx);
            assert!(!switch(document));
            // A refreshed status waits too.
            document.take_status(epoch, on(), cx);
            assert!(!switch(document));
            document.finish_switch_write(epoch, 2, Ok(WriteOutcome::Written(off())), cx);
            assert!(!switch(document));
            assert_eq!(document.switch_writes, 0);
            // A status read for other settings (before a reload) is ignored.
            document.take_status(epoch + 1, on(), cx);
            assert!(!switch(document));
        });
    }

    #[gpui_kit::test]
    fn undo_writes_what_it_puts_back(cx: &mut TestAppContext) {
        let (document, _) = loaded(cx);
        document.update(cx, |document, cx| {
            document.add_pair(cx);
            let added = pair(document, 0);
            let follows = SocdPair {
                mode: SocdMode::Global,
                ..added
            };
            document.set_pair(0, follows, cx);
            document.set_switch(true, cx);

            document.undo(cx);
            assert!(!switch(document));
            assert_eq!(document.switch_writes, 2);
            document.undo(cx);
            assert_eq!(pair(document, 0), added);
            assert_eq!(document.revision, 3);
            document.undo(cx);
            assert!(!document.has_undo_history());
        });
    }

    #[gpui_kit::test]
    fn results_after_the_keyboard_goes_away_are_ignored(cx: &mut TestAppContext) {
        let (document, failures) = loaded(cx);
        document.update(cx, |document, cx| {
            document.add_pair(cx);
            document.set_switch(true, cx);
            let epoch = document.epoch;
            document.clear(cx);
            document.finish_pairs_write(epoch, 1, failed(None), cx);
            document.finish_switch_write(epoch, 1, failed(None), cx);
            assert!(matches!(document.settings, Loadable::Idle));

            assert_eq!(document.add_pair(cx), None);
            document.set_switch(true, cx);
            assert_eq!(document.revision, 1);
            assert_eq!(document.switch_writes, 0);
        });
        assert!(failures.borrow().is_empty());
    }
}
