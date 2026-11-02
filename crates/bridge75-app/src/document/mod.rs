//! The keyboard's settings as the app knows them.
//!
//! Settings the keyboard can report are read when it connects and dropped when it goes away.
//! Every edit is shown at once and written straight away. If a write fails, the keys it was
//! meant for are read back from the keyboard, because part of it, or even the failed request,
//! may have taken effect; [`DocumentEvent::WriteFailed`] is emitted. Edits can be undone one by
//! one, newest first; undoing writes the earlier settings back like any other edit.

mod triggers;
mod undo;

use bridge75_core::he::TriggerConfig;
use bridge75_core::matrix::{KeyBitmap, KeyPos, LAYOUT};
use bridge75_device::Device;
use gpui_kit::{
    App, AppContext as _, Context, Entity, EventEmitter, SharedString, Subscription, Task,
};

use crate::session::{ConnectionId, RequestError, Session, SessionEvent};
use triggers::PendingWrite;
pub use triggers::{MIN_TRAVEL, TriggerEdit, TriggerField, TriggerTable, stepped};
use undo::UndoStack;

/// A value read from the keyboard.
#[derive(Clone, Debug, Default)]
pub enum Loadable<T> {
    /// No keyboard is connected.
    #[default]
    Idle,
    Loading,
    Loaded(T),
    Failed(SharedString),
}

impl<T> Loadable<T> {
    pub fn loaded(&self) -> Option<&T> {
        match self {
            Loadable::Loaded(value) => Some(value),
            _ => None,
        }
    }

    pub fn loaded_mut(&mut self) -> Option<&mut T> {
        match self {
            Loadable::Loaded(value) => Some(value),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub enum DocumentEvent {
    /// Reading from the keyboard failed. The message is the device error.
    LoadFailed(SharedString),
    /// A write failed. The settings shown were read back from the keyboard (or are being read
    /// again). The message is the device error.
    WriteFailed(SharedString),
}

/// How a write of trigger settings ended, as seen from the session thread.
#[derive(Debug)]
enum WriteOutcome {
    Written,
    Failed {
        error: String,
        /// The settings of the written keys as read back afterwards, if that worked.
        actual: Option<Vec<(KeyPos, TriggerConfig)>>,
    },
}

/// Writes `writes` in order. When one fails, reads back `keys` before anything else reaches
/// the keyboard: earlier writes have taken effect, and the failed one may have too.
fn write_triggers(
    device: &Device,
    writes: &[(KeyBitmap, TriggerConfig)],
    keys: &[KeyPos],
) -> WriteOutcome {
    match writes
        .iter()
        .try_for_each(|(bitmap, config)| device.set_trigger(bitmap, config))
    {
        Ok(()) => WriteOutcome::Written,
        Err(e) => WriteOutcome::Failed {
            error: e.to_string(),
            actual: device.triggers(keys.iter().copied()).ok(),
        },
    }
}

pub struct KeyboardDocument {
    session: Entity<Session>,
    /// The connection the loaded settings came from.
    connection: Option<ConnectionId>,
    /// Counts every change made to the settings, across reloads and connections, so results
    /// can tell which changes came after them.
    revision: u64,
    triggers: Loadable<TriggerTable>,
    undo: UndoStack,
    _load: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<DocumentEvent> for KeyboardDocument {}

impl KeyboardDocument {
    pub fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::Connected(connection) => this.load(*connection, cx),
            SessionEvent::Disconnected => this.clear(cx),
            SessionEvent::Travel(_) => {}
        });
        let mut document = KeyboardDocument {
            session: session.clone(),
            connection: None,
            revision: 0,
            triggers: Loadable::Idle,
            undo: UndoStack::default(),
            _load: None,
            _subscriptions: vec![subscription],
        };
        if let Some(connection) = session.read(cx).connection() {
            document.load(connection, cx);
        }
        document
    }

    pub fn triggers(&self) -> &Loadable<TriggerTable> {
        &self.triggers
    }

    /// Whether there is an edit to undo.
    pub fn has_undo_history(&self) -> bool {
        self.triggers.loaded().is_some() && !self.undo.is_empty()
    }

    /// Reads the settings again from the current connection.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if let Some(connection) = self.session.read(cx).connection() {
            self.load(connection, cx);
        }
    }

    fn load(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        self.connection = Some(connection);
        self.triggers = Loadable::Loading;
        self.undo.clear();
        cx.notify();
        let read_at = self.revision;
        let read = self.session.update(cx, |session, cx| {
            session.run(|device| device.triggers(LAYOUT.iter().map(|k| k.pos)), cx)
        });
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                this.finish_load(connection, read_at, result, cx)
            });
        }));
    }

    fn finish_load(
        &mut self,
        connection: ConnectionId,
        read_at: u64,
        result: Result<Vec<(KeyPos, TriggerConfig)>, RequestError>,
        cx: &mut Context<Self>,
    ) {
        // A reconnect started a newer load.
        if self.connection != Some(connection) {
            return;
        }
        match result {
            Ok(configs) => self.triggers = Loadable::Loaded(TriggerTable::new(configs, read_at)),
            Err(e) => {
                let message = SharedString::from(e.to_string());
                self.triggers = Loadable::Failed(message.clone());
                cx.emit(DocumentEvent::LoadFailed(message));
            }
        }
        cx.notify();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.connection = None;
        self.triggers = Loadable::Idle;
        self.undo.clear();
        self._load = None;
        cx.notify();
    }

    fn next_revision(&mut self) -> u64 {
        self.revision += 1;
        self.revision
    }

    /// Applies `edit` to `keys` and writes the result to the keyboard.
    pub fn edit_triggers(&mut self, keys: &[KeyPos], edit: TriggerEdit, cx: &mut Context<Self>) {
        let revision = self.next_revision();
        let Loadable::Loaded(table) = &mut self.triggers else {
            return;
        };
        let pending = table.apply(keys.iter().copied(), edit, revision);
        if pending.is_empty() {
            return;
        }
        self.undo.push(revision, pending.previous.clone());
        self.write(pending, cx);
    }

    /// Puts back the settings the newest edit replaced and writes them to the keyboard.
    pub fn undo(&mut self, cx: &mut Context<Self>) {
        if !self.has_undo_history() {
            return;
        }
        let revision = self.next_revision();
        let (Loadable::Loaded(table), Some(entry)) = (&mut self.triggers, self.undo.pop()) else {
            return;
        };
        let pending = table.restore(&entry.previous, revision);
        cx.notify();
        if !pending.is_empty() {
            self.write(pending, cx);
        }
    }

    fn write(&mut self, pending: PendingWrite, cx: &mut Context<Self>) {
        cx.notify();
        let Some(connection) = self.connection else {
            return;
        };
        let revision = pending.revision;
        let keys = pending.keys();
        let writes = pending.writes;
        let write = self.session.update(cx, |session, cx| {
            session.run(move |device| Ok(write_triggers(device, &writes, &keys)), cx)
        });
        cx.spawn(async move |this, cx| {
            let result = write.await;
            let _ = this.update(cx, |this, cx| {
                this.finish_write(connection, revision, result, cx)
            });
        })
        .detach();
    }

    fn finish_write(
        &mut self,
        connection: ConnectionId,
        revision: u64,
        result: Result<WriteOutcome, RequestError>,
        cx: &mut Context<Self>,
    ) {
        // The settings of that connection are gone: they were cleared or read again.
        if self.connection != Some(connection) {
            return;
        }
        let (error, actual) = match result {
            Ok(WriteOutcome::Written) => return,
            Ok(WriteOutcome::Failed { error, actual }) => (error, actual),
            // The job never ran; the session reports the disconnect, which clears the settings.
            Err(e) => {
                cx.emit(DocumentEvent::WriteFailed(e.to_string().into()));
                return;
            }
        };
        self.undo.discard(revision);
        match actual {
            Some(actual) => {
                if let Loadable::Loaded(table) = &mut self.triggers {
                    table.recover(revision, actual);
                }
            }
            // Nothing is known about the keys any more, so read everything again.
            None => self.load(connection, cx),
        }
        cx.emit(DocumentEvent::WriteFailed(error.into()));
        cx.notify();
    }
}

/// Creates the document for `session`.
pub fn new(session: Entity<Session>, cx: &mut App) -> Entity<KeyboardDocument> {
    cx.new(|cx| KeyboardDocument::new(session, cx))
}

#[cfg(test)]
impl KeyboardDocument {
    /// A document showing `configs` as read from connection 1 of a session with no keyboard
    /// behind it, so nothing a test does can reach a real keyboard.
    pub fn for_test(configs: Vec<(KeyPos, TriggerConfig)>, cx: &mut Context<Self>) -> Self {
        let session = cx.new(|_| Session::detached());
        let mut document = KeyboardDocument::new(session, cx);
        let connection = crate::session::ConnectionId::for_test(1);
        document.connection = Some(connection);
        document.finish_load(connection, 0, Ok(configs), cx);
        document
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use bridge75_core::he::{Travel, TriggerMode};
    use gpui_kit::TestAppContext;

    use super::*;

    fn pos(id: &str) -> KeyPos {
        id.parse().unwrap()
    }

    fn mm(tenths: u8) -> Travel {
        Travel::new(tenths).unwrap()
    }

    fn actuation(document: &KeyboardDocument, id: &str) -> Travel {
        document
            .triggers()
            .loaded()
            .and_then(|t| t.get(pos(id)))
            .unwrap()
            .actuation
    }

    /// A document whose settings were loaded from connection 1, with no keyboard behind it.
    ///
    /// Writes go to a detached session. Their results stay queued because these tests never
    /// run the executor, so only the results a test hands in itself arrive.
    fn loaded(cx: &mut TestAppContext) -> (Entity<KeyboardDocument>, Rc<RefCell<Vec<String>>>) {
        let configs = [pos("w"), pos("a"), pos("s")].map(|p| (p, TriggerConfig::DEFAULT));
        let document = cx.new(|cx| KeyboardDocument::for_test(configs.to_vec(), cx));
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

    #[gpui_kit::test]
    fn results_from_an_earlier_connection_are_ignored(cx: &mut TestAppContext) {
        let (document, failures) = loaded(cx);
        document.update(cx, |document, cx| {
            let old = ConnectionId::for_test(1);
            document.connection = Some(ConnectionId::for_test(2));
            document.triggers = Loadable::Loading;

            document.finish_load(old, 0, Ok(vec![(pos("w"), TriggerConfig::DEFAULT)]), cx);
            assert!(matches!(document.triggers, Loadable::Loading));

            let failed = WriteOutcome::Failed {
                error: "no response".into(),
                actual: None,
            };
            document.finish_write(old, 1, Ok(failed), cx);
            assert!(matches!(document.triggers, Loadable::Loading));
        });
        assert!(failures.borrow().is_empty());
    }

    #[gpui_kit::test]
    fn a_failed_write_takes_what_the_keyboard_has_and_keeps_newer_edits(cx: &mut TestAppContext) {
        let (document, failures) = loaded(cx);
        document.update(cx, |document, cx| {
            let set = |t| TriggerEdit::Set(TriggerField::Actuation, mm(t));
            document.edit_triggers(&[pos("w"), pos("a")], set(15), cx);
            let first = document.revision;
            document.edit_triggers(&[pos("a")], set(25), cx);

            // The keyboard kept its settings for both keys.
            let actual = vec![
                (pos("w"), TriggerConfig::DEFAULT),
                (pos("a"), TriggerConfig::DEFAULT),
            ];
            let failed = WriteOutcome::Failed {
                error: "set trigger settings: no response".into(),
                actual: Some(actual),
            };
            document.finish_write(ConnectionId::for_test(1), first, Ok(failed), cx);

            assert_eq!(actuation(document, "w"), mm(18));
            assert_eq!(actuation(document, "a"), mm(25));
            // The failed edit cannot be undone, nor the newer edit of the same key.
            assert!(!document.has_undo_history());
        });
        assert_eq!(*failures.borrow(), ["set trigger settings: no response"]);
    }

    #[gpui_kit::test]
    fn a_failed_read_back_reloads_everything(cx: &mut TestAppContext) {
        let (document, failures) = loaded(cx);
        document.update(cx, |document, cx| {
            document.edit_triggers(&[pos("w")], TriggerEdit::Mode(TriggerMode::Rapid), cx);
            let failed = WriteOutcome::Failed {
                error: "lost connection".into(),
                actual: None,
            };
            let revision = document.revision;
            document.finish_write(ConnectionId::for_test(1), revision, Ok(failed), cx);
            assert!(matches!(document.triggers, Loadable::Loading));
        });
        assert_eq!(failures.borrow().len(), 1);
    }

    #[gpui_kit::test]
    fn undo_restores_edits_newest_first(cx: &mut TestAppContext) {
        let (document, _) = loaded(cx);
        document.update(cx, |document, cx| {
            let keys = [pos("w"), pos("s")];
            document.edit_triggers(&keys, TriggerEdit::Set(TriggerField::Actuation, mm(10)), cx);
            document.edit_triggers(&keys, TriggerEdit::Step(TriggerField::Actuation, 2), cx);
            assert_eq!(actuation(document, "s"), mm(12));

            document.undo(cx);
            assert_eq!(actuation(document, "w"), mm(10));
            document.undo(cx);
            assert_eq!(actuation(document, "w"), mm(18));
            assert!(!document.has_undo_history());
            // Nothing left to undo.
            document.undo(cx);
            assert_eq!(actuation(document, "w"), mm(18));
        });
    }

    #[gpui_kit::test]
    fn relative_steps_keep_differences_between_keys(cx: &mut TestAppContext) {
        let (document, _) = loaded(cx);
        document.update(cx, |document, cx| {
            let set = |t| TriggerEdit::Set(TriggerField::Actuation, mm(t));
            document.edit_triggers(&[pos("w")], set(10), cx);
            document.edit_triggers(&[pos("a")], set(36), cx);
            let keys = [pos("w"), pos("a"), pos("s")];
            document.edit_triggers(&keys, TriggerEdit::Step(TriggerField::Actuation, 1), cx);
            assert_eq!(actuation(document, "w"), mm(11));
            assert_eq!(actuation(document, "a"), mm(36));
            assert_eq!(actuation(document, "s"), mm(19));
        });
    }

    #[gpui_kit::test]
    fn reloading_forgets_the_undo_history(cx: &mut TestAppContext) {
        let (document, _) = loaded(cx);
        document.update(cx, |document, cx| {
            document.edit_triggers(&[pos("w")], TriggerEdit::Mode(TriggerMode::Rapid), cx);
            assert!(document.has_undo_history());
            document.load(ConnectionId::for_test(1), cx);
            assert!(!document.has_undo_history());
        });
    }
}
