//! The keymap of the connected keyboard as this app knows it.
//!
//! The keyboard can't report its keymap, so when it connects the app shows the keymap it last
//! wrote to that keyboard ([`KeymapStore`]), or the default keymap if it never wrote one. Every
//! change is shown at once and both layers are written in full, as the keyboard only saves
//! them together. Changes made while a write is in flight are written together once it ends,
//! so a quick series of changes does not queue one flash write each. A keymap is stored once
//! the keyboard has taken it.

use bridge75_core::keymap::{KeyAction, Layer};
use bridge75_core::matrix::{KeyBitmap, KeyPos};
use bridge75_core::profile::KeymapProfile;
use gpui_kit::{Context, Entity, SharedString, Subscription, Task};

use super::layers::{Change, KeyLayers, UndoStack};
use super::store::{KeymapStore, keyboard_id};
use crate::document::Loadable;
use crate::session::{ConnectionId, RequestError, Session, SessionEvent, SessionState};

/// The keymap of one connected keyboard.
pub struct LoadedKeymap {
    /// Names the keyboard's file in the store.
    keyboard: String,
    layers: KeyLayers,
    /// Keys whose Fn layer function the keyboard fixes. It ignores what is written for them.
    fixed: KeyBitmap,
    /// Whether the keymap came from the store, rather than being the default.
    stored: bool,
    undo: UndoStack,
}

impl LoadedKeymap {
    fn new(keyboard: String, stored: Option<KeymapProfile>, fixed: KeyBitmap) -> Self {
        LoadedKeymap {
            keyboard,
            stored: stored.is_some(),
            layers: stored.map(KeyLayers::from_profile).unwrap_or_default(),
            fixed,
            undo: UndoStack::default(),
        }
    }

    pub fn get(&self, layer: Layer, pos: KeyPos) -> KeyAction {
        self.layers.get(layer, pos)
    }

    pub fn is_default(&self, layer: Layer, pos: KeyPos) -> bool {
        self.layers.is_default(layer, pos)
    }

    /// Whether the keyboard fixes what `pos` does on `layer`, so it can't be changed.
    pub fn is_fixed(&self, layer: Layer, pos: KeyPos) -> bool {
        layer == Layer::Fn && self.fixed.contains(pos)
    }

    pub fn has_fn_key(&self) -> bool {
        self.layers.has_fn_key()
    }

    /// Whether this app wrote a keymap to the keyboard before, so the keymap shown is what the
    /// keyboard has unless another tool changed it since.
    pub fn is_stored(&self) -> bool {
        self.stored
    }

    pub fn is_changed(&self) -> bool {
        self.layers.is_changed()
    }
}

pub struct KeymapDocument {
    session: Entity<Session>,
    store: KeymapStore,
    /// The connection the keymap belongs to.
    connection: Option<ConnectionId>,
    keymap: Loadable<LoadedKeymap>,
    /// Counts every change, so a finished write can tell whether newer changes are waiting.
    revision: u64,
    /// The newest revision the keyboard has taken.
    written: u64,
    /// The revision being written, while a write is in flight.
    writing: Option<u64>,
    write_error: Option<SharedString>,
    save_error: Option<SharedString>,
    _load: Option<Task<()>>,
    _write: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl KeymapDocument {
    pub fn new(session: Entity<Session>, store: KeymapStore, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::Connected(connection) => this.load(*connection, cx),
            SessionEvent::Disconnected => this.clear(cx),
            SessionEvent::Travel(_) | SessionEvent::Calibration(_) => {}
        });
        let mut document = KeymapDocument {
            session: session.clone(),
            store,
            connection: None,
            keymap: Loadable::Idle,
            revision: 0,
            written: 0,
            writing: None,
            write_error: None,
            save_error: None,
            _load: None,
            _write: None,
            _subscriptions: vec![subscription],
        };
        if let Some(connection) = session.read(cx).connection() {
            document.load(connection, cx);
        }
        document
    }

    pub fn keymap(&self) -> &Loadable<LoadedKeymap> {
        &self.keymap
    }

    /// Why the latest write failed. Cleared once a write succeeds.
    pub fn write_error(&self) -> Option<&SharedString> {
        self.write_error.as_ref()
    }

    /// Why the keymap the keyboard took couldn't be stored on this computer.
    pub fn save_error(&self) -> Option<&SharedString> {
        self.save_error.as_ref()
    }

    pub fn has_undo_history(&self) -> bool {
        self.keymap.loaded().is_some_and(|k| !k.undo.is_empty())
    }

    /// Reads the stored keymap and the fixed keys again for the current connection.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if let Some(connection) = self.session.read(cx).connection() {
            self.load(connection, cx);
        }
    }

    fn load(&mut self, connection: ConnectionId, cx: &mut Context<Self>) {
        self.connection = Some(connection);
        self.writing = None;
        self.write_error = None;
        self.save_error = None;
        self._write = None;
        self.written = self.revision;
        let serial = match self.session.read(cx).state() {
            SessionState::Connected { info, .. } => info.serial.clone(),
            _ => None,
        };
        let keyboard = keyboard_id(serial.as_deref());
        let stored = match self.store.load(&keyboard) {
            Ok(stored) => stored,
            Err(e) => {
                self.keymap = Loadable::Failed(e.to_string().into());
                self._load = None;
                cx.notify();
                return;
            }
        };
        self.keymap = Loadable::Loading;
        cx.notify();
        let read = self.session.update(cx, |session, cx| {
            session.run(|device| device.fixed_fn_mask(), cx)
        });
        self._load = Some(cx.spawn(async move |this, cx| {
            let result = read.await;
            let _ = this.update(cx, |this, cx| {
                this.finish_load(connection, keyboard, stored, result, cx)
            });
        }));
    }

    fn finish_load(
        &mut self,
        connection: ConnectionId,
        keyboard: String,
        stored: Option<KeymapProfile>,
        result: Result<KeyBitmap, RequestError>,
        cx: &mut Context<Self>,
    ) {
        if self.connection != Some(connection) {
            return;
        }
        self.keymap = match result {
            Ok(fixed) => Loadable::Loaded(LoadedKeymap::new(keyboard, stored, fixed)),
            Err(e) => Loadable::Failed(e.to_string().into()),
        };
        cx.notify();
    }

    fn clear(&mut self, cx: &mut Context<Self>) {
        self.connection = None;
        self.keymap = Loadable::Idle;
        self.writing = None;
        self.write_error = None;
        self.save_error = None;
        self._load = None;
        self._write = None;
        cx.notify();
    }

    /// Assigns `action` to `pos` on `layer` and writes the keymap.
    pub fn assign(&mut self, layer: Layer, pos: KeyPos, action: KeyAction, cx: &mut Context<Self>) {
        let Some(keymap) = self.keymap.loaded_mut() else {
            return;
        };
        if keymap.is_fixed(layer, pos) {
            return;
        }
        let Some(previous) = keymap.layers.set(layer, pos, action) else {
            return;
        };
        keymap.undo.push(vec![Change {
            layer,
            pos,
            previous,
        }]);
        self.changed(cx);
    }

    /// Puts back what the newest change replaced and writes the keymap. Returns the key it
    /// changed back.
    pub fn undo(&mut self, cx: &mut Context<Self>) -> Option<(Layer, KeyPos)> {
        let keymap = self.keymap.loaded_mut()?;
        let changes = keymap.undo.pop()?;
        for change in changes.iter().rev() {
            keymap.layers.set(change.layer, change.pos, change.previous);
        }
        self.changed(cx);
        changes.first().map(|change| (change.layer, change.pos))
    }

    /// Writes the keymap again after a failed write.
    pub fn retry_write(&mut self, cx: &mut Context<Self>) {
        self.write_error = None;
        // A failed write leaves `written` behind `revision`, so this writes again.
        self.write(cx);
        cx.notify();
    }

    fn changed(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        self.write(cx);
        cx.notify();
    }

    /// Starts writing the current keymap, unless a write is in flight; that one starts the
    /// next write when it ends.
    fn write(&mut self, cx: &mut Context<Self>) {
        let (Some(connection), Some(keymap)) = (self.connection, self.keymap.loaded()) else {
            return;
        };
        if self.writing.is_some() || self.written == self.revision {
            return;
        }
        let revision = self.revision;
        let profile = keymap.layers.profile().clone();
        let (base, fn_layer) = keymap.layers.build();
        self.writing = Some(revision);
        let write = self.session.update(cx, |session, cx| {
            session.run(move |device| device.write_keymaps(&base, &fn_layer), cx)
        });
        self._write = Some(cx.spawn(async move |this, cx| {
            let result = write.await;
            let _ = this.update(cx, |this, cx| {
                this.finish_write(connection, revision, profile, result, cx)
            });
        }));
    }

    fn finish_write(
        &mut self,
        connection: ConnectionId,
        revision: u64,
        profile: KeymapProfile,
        result: Result<(), RequestError>,
        cx: &mut Context<Self>,
    ) {
        if self.connection != Some(connection) {
            return;
        }
        self.writing = None;
        match result {
            Ok(()) => {
                self.written = revision;
                self.write_error = None;
                if let Some(keymap) = self.keymap.loaded_mut() {
                    keymap.stored = true;
                    self.save_error = self
                        .store
                        .save(&keymap.keyboard, &profile)
                        .err()
                        .map(|e| e.to_string().into());
                }
                // Changes made during the write.
                self.write(cx);
            }
            // Part of the keymap may have reached the keyboard, but it only saves a complete
            // one, so it keeps the previous keymap once it restarts.
            Err(e) => self.write_error = Some(e.to_string().into()),
        }
        cx.notify();
    }
}

#[cfg(test)]
impl KeymapDocument {
    /// A document whose keymap was loaded from connection 1 of a session with no keyboard
    /// behind it, so nothing a test does can reach a real keyboard.
    pub fn for_test(
        stored: Option<KeymapProfile>,
        fixed: KeyBitmap,
        store: KeymapStore,
        cx: &mut Context<Self>,
    ) -> Self {
        use gpui_kit::AppContext as _;

        let session = cx.new(|_| Session::detached());
        let mut document = KeymapDocument::new(session, store, cx);
        let connection = ConnectionId::for_test(1);
        document.connection = Some(connection);
        document.finish_load(connection, "test".into(), stored, Ok(fixed), cx);
        document
    }

    /// Forgets the keymap as an unplugged keyboard does.
    pub fn disconnect_for_test(&mut self, cx: &mut Context<Self>) {
        self.clear(cx);
    }
}

#[cfg(test)]
mod tests {
    use bridge75_core::hid::Usage;
    use gpui_kit::{AppContext as _, TestAppContext};

    use super::*;

    fn pos(id: &str) -> KeyPos {
        id.parse().unwrap()
    }

    fn key(name: &str) -> KeyAction {
        KeyAction::Key(name.parse::<Usage>().unwrap())
    }

    fn connection() -> ConnectionId {
        ConnectionId::for_test(1)
    }

    /// A loaded document whose writes never run: these tests hand in the results themselves.
    fn loaded(cx: &mut TestAppContext) -> Entity<KeymapDocument> {
        let fixed: KeyBitmap = [pos("f1")].into_iter().collect();
        cx.new(|cx| KeymapDocument::for_test(None, fixed, KeymapStore::none(), cx))
    }

    fn get(document: &KeymapDocument, layer: Layer, id: &str) -> KeyAction {
        document.keymap().loaded().unwrap().get(layer, pos(id))
    }

    #[gpui_kit::test]
    fn changes_during_a_write_are_written_together_afterwards(cx: &mut TestAppContext) {
        let document = loaded(cx);
        document.update(cx, |document, cx| {
            document.assign(Layer::Base, pos("caps"), key("lctrl"), cx);
            assert_eq!(document.writing, Some(1));
            document.assign(Layer::Fn, pos("h"), key("left"), cx);
            document.assign(Layer::Fn, pos("j"), key("down"), cx);
            // Still the first write.
            assert_eq!(document.writing, Some(1));

            let first = document.keymap().loaded().unwrap().layers.profile().clone();
            document.finish_write(connection(), 1, first, Ok(()), cx);
            assert_eq!(document.written, 1);
            assert_eq!(document.writing, Some(3));
            assert!(document.keymap().loaded().unwrap().is_stored());
        });
    }

    #[gpui_kit::test]
    fn a_failed_write_keeps_the_keymap_and_can_be_retried(cx: &mut TestAppContext) {
        let document = loaded(cx);
        document.update(cx, |document, cx| {
            document.assign(Layer::Base, pos("caps"), key("esc"), cx);
            let profile = document.keymap().loaded().unwrap().layers.profile().clone();
            let error = RequestError::Device(bridge75_device::Error::Timeout("write".into()));
            document.finish_write(connection(), 1, profile, Err(error), cx);
            assert!(document.write_error().is_some());
            assert_eq!(get(document, Layer::Base, "caps"), key("esc"));
            assert!(!document.keymap().loaded().unwrap().is_stored());

            document.retry_write(cx);
            assert!(document.write_error().is_none());
            assert_eq!(document.writing, Some(1));
        });
    }

    #[gpui_kit::test]
    fn undo_puts_back_the_previous_assignment_and_writes_it(cx: &mut TestAppContext) {
        let document = loaded(cx);
        document.update(cx, |document, cx| {
            document.assign(Layer::Fn, pos("h"), key("left"), cx);
            document.assign(Layer::Fn, pos("h"), key("home"), cx);
            assert_eq!(document.undo(cx), Some((Layer::Fn, pos("h"))));
            assert_eq!(get(document, Layer::Fn, "h"), key("left"));
            assert_eq!(document.revision, 3);
            assert_eq!(document.undo(cx), Some((Layer::Fn, pos("h"))));
            assert!(
                document
                    .keymap()
                    .loaded()
                    .unwrap()
                    .is_default(Layer::Fn, pos("h"))
            );
            assert!(!document.has_undo_history());
            assert_eq!(document.undo(cx), None);
        });
    }

    #[gpui_kit::test]
    fn fixed_keys_and_unchanged_assignments_are_not_written(cx: &mut TestAppContext) {
        let document = loaded(cx);
        document.update(cx, |document, cx| {
            document.assign(Layer::Fn, pos("f1"), key("a"), cx);
            document.assign(Layer::Base, pos("a"), key("a"), cx);
            assert_eq!(document.revision, 0);
            assert!(!document.has_undo_history());
            // The base layer of a fixed key can be changed.
            document.assign(Layer::Base, pos("f1"), key("a"), cx);
            assert_eq!(document.revision, 1);
        });
    }

    #[gpui_kit::test]
    fn results_from_an_earlier_connection_are_ignored(cx: &mut TestAppContext) {
        let document = loaded(cx);
        document.update(cx, |document, cx| {
            document.assign(Layer::Base, pos("caps"), key("esc"), cx);
            document.connection = Some(ConnectionId::for_test(2));
            let profile = KeymapProfile::default();
            document.finish_write(connection(), 1, profile, Ok(()), cx);
            assert_eq!(document.written, 0);
            document.finish_load(connection(), "old".into(), None, Ok(KeyBitmap::EMPTY), cx);
            assert_eq!(document.keymap().loaded().unwrap().keyboard, "test");
        });
    }

    #[gpui_kit::test]
    fn a_stored_keymap_is_shown_when_the_keyboard_connects(cx: &mut TestAppContext) {
        let mut stored = KeymapProfile::default();
        stored.base.insert(pos("caps"), key("lctrl"));
        let document = cx.new(|cx| {
            KeymapDocument::for_test(Some(stored), KeyBitmap::EMPTY, KeymapStore::none(), cx)
        });
        document.read_with(cx, |document, _| {
            let keymap = document.keymap().loaded().unwrap();
            assert!(keymap.is_stored());
            assert_eq!(keymap.get(Layer::Base, pos("caps")), key("lctrl"));
        });
    }
}
