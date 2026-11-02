//! Calibration: measuring where each key rests and where it bottoms out.
//!
//! All keys are calibrated together, or one key picked on the board. Either counts down first
//! (see [`run`]). While the keyboard calibrates, every key press is a calibration gesture, so
//! the window ignores key presses and the shell keeps this page open; the run is stopped with
//! the pointer. Every key on the keyboard takes part, so no key could serve as a shortcut.

mod run;

use std::time::Duration;

use bridge75_core::matrix::{Key, KeyPos, LAYOUT};
use gpui_kit::component::button::Button;
use gpui_kit::component::progress::Progress;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    ThemeStyled as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, Entity, FocusHandle, Hsla, InteractiveElement as _, IntoElement,
    KeyBinding, ParentElement as _, Render, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Task, Window, actions, div,
};
use rust_i18n::t;

use crate::keyboard::{self, Direction, KeyboardView};
use crate::page::{self, PageHeader};
use crate::session::{Session, SessionEvent};
use run::{Calibration, Outcome, Phase, Target};

actions!(
    calibration,
    [
        /// Cancel the countdown before a calibration starts.
        CancelCountdown,
        SelectLeft,
        SelectRight,
        SelectUp,
        SelectDown,
    ]
);

/// Key context of the whole page.
const CONTEXT: &str = "CalibrationPage";
/// Key context of the board while it has focus.
const BOARD_CONTEXT: &str = "CalibrationBoard";

/// How long one key may take before the keyboard is taken not to answer. The keyboard itself
/// gives up after about 10 s.
const KEY_ANSWER_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a stop of all keys may take to be reported.
const STOP_ANSWER_TIMEOUT: Duration = Duration::from_secs(3);

/// Binds the page's keyboard shortcuts.
pub fn init(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", CancelCountdown, Some(CONTEXT)),
        KeyBinding::new("left", SelectLeft, Some(BOARD_CONTEXT)),
        KeyBinding::new("right", SelectRight, Some(BOARD_CONTEXT)),
        KeyBinding::new("up", SelectUp, Some(BOARD_CONTEXT)),
        KeyBinding::new("down", SelectDown, Some(BOARD_CONTEXT)),
    ]);
}

fn key_label(pos: KeyPos) -> &'static str {
    pos.key().map_or("", |key| key.label)
}

pub struct CalibrationPage {
    session: Entity<Session>,
    calibration: Calibration,
    /// The key that "Calibrate <key>" calibrates.
    selected: Option<KeyPos>,
    /// Holds focus during a run, where no control would react to a key press anyway.
    focus_handle: FocusHandle,
    board_focus: FocusHandle,
    _countdown: Option<Task<()>>,
    /// Ends the run if the keyboard never reports its end.
    _answer_timeout: Option<Task<()>>,
    /// Swallows key presses in the window while the keyboard calibrates.
    _key_block: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl CalibrationPage {
    pub fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::Calibration(report) => {
                this.calibration.report(*report);
                this.sync(cx);
            }
            SessionEvent::Disconnected => {
                this.calibration.disconnected();
                this.sync(cx);
            }
            SessionEvent::Connected(_) | SessionEvent::Travel(_) => {}
        });
        CalibrationPage {
            session,
            calibration: Calibration::default(),
            selected: None,
            focus_handle: cx.focus_handle(),
            board_focus: cx.focus_handle().tab_stop(true),
            _countdown: None,
            _answer_timeout: None,
            _key_block: None,
            _subscriptions: vec![subscription],
        }
    }

    /// Moves keyboard focus into the page. The shell calls this when the page is opened.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus_handle.focus(window, cx);
    }

    /// Whether a calibration is counting down or running. The shell keeps the page open
    /// meanwhile, so nothing else talks to the keyboard and its reports are not missed.
    pub fn is_busy(&self) -> bool {
        self.calibration.is_busy()
    }

    /// Starts the countdown to calibrating all keys, connected or not.
    #[cfg(test)]
    pub fn begin_for_test(&mut self, cx: &mut Context<Self>) {
        self.calibration.begin(Target::All);
        self.sync(cx);
    }

    fn is_connected(&self, cx: &App) -> bool {
        self.session.read(cx).connection().is_some()
    }

    /// Brings the tasks and the key block in line with the phase, after it changed.
    fn sync(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.calibration.phase(), Phase::Countdown { .. }) {
            self._countdown = None;
        }
        if !self.calibration.is_running() {
            self._answer_timeout = None;
            self._key_block = None;
        } else if self._key_block.is_none() {
            self._key_block = Some(cx.intercept_keystrokes(|_, _, cx| cx.stop_propagation()));
        }
        cx.notify();
    }

    fn begin(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        if !self.is_connected(cx) || !self.calibration.begin(target) {
            return;
        }
        self._countdown = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                let counting = this.update_in(cx, |this, window, cx| this.tick(window, cx));
                if !counting.unwrap_or(false) {
                    return;
                }
            }
        }));
        cx.notify();
    }

    /// One second of the countdown has passed. Returns whether it is still counting.
    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        match self.calibration.tick() {
            Some(target) => {
                self.start(target, window, cx);
                false
            }
            None => {
                cx.notify();
                matches!(self.calibration.phase(), Phase::Countdown { .. })
            }
        }
    }

    fn start(&mut self, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        // The focused control might be a button, which Space or Enter would press.
        self.focus_handle.focus(window, cx);
        let run = self.calibration.run();
        let started = self.session.update(cx, |session, cx| {
            session.run(
                move |device| {
                    // Reports left from earlier are not about this run.
                    device.drain();
                    match target {
                        Target::All => device.start_calibrate_all(),
                        Target::Key(pos) => device.start_calibrate_key(pos),
                    }
                },
                cx,
            )
        });
        cx.spawn(async move |this, cx| {
            if let Err(e) = started.await {
                let _ = this.update(cx, |this, cx| {
                    this.calibration.start_failed(run, e.to_string());
                    this.sync(cx);
                });
            }
        })
        .detach();
        if matches!(target, Target::Key(_)) {
            self.wait_for_answer(KEY_ANSWER_TIMEOUT, cx);
        }
        self.sync(cx);
    }

    fn wait_for_answer(&mut self, timeout: Duration, cx: &mut Context<Self>) {
        let run = self.calibration.run();
        self._answer_timeout = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(timeout).await;
            let _ = this.update(cx, |this, cx| {
                this.calibration.no_answer(run);
                this.sync(cx);
            });
        }));
    }

    fn cancel_countdown(&mut self, _: &CancelCountdown, _: &mut Window, cx: &mut Context<Self>) {
        if self.calibration.cancel() {
            self.sync(cx);
        } else {
            cx.propagate();
        }
    }

    fn stop(&mut self, cx: &mut Context<Self>) {
        if !self.calibration.stop() {
            return;
        }
        let stop = self.session.update(cx, |session, cx| {
            session.run(|device| device.abort_calibrate_all(), cx)
        });
        // A stop that never arrives shows as a missing report, which the timeout handles.
        stop.detach();
        self.wait_for_answer(STOP_ANSWER_TIMEOUT, cx);
        cx.notify();
    }

    fn is_board_enabled(&self, cx: &App) -> bool {
        self.is_connected(cx) && !self.calibration.is_busy()
    }

    fn select(&mut self, pos: KeyPos, window: &mut Window, cx: &mut Context<Self>) {
        self.selected = Some(pos);
        // Keycaps are not focusable, so the arrow keys continue from the clicked key.
        self.board_focus.focus(window, cx);
        cx.notify();
    }

    fn move_selection(&mut self, direction: Direction, cx: &mut Context<Self>) {
        if !self.is_board_enabled(cx) {
            return;
        }
        let from = self.selected.unwrap_or(LAYOUT[0].pos);
        let next = if self.selected.is_none() {
            Some(from)
        } else {
            keyboard::neighbor(from, direction)
        };
        if let Some(next) = next {
            self.selected = Some(next);
            cx.notify();
        }
    }

    /// A short line with an icon that tells what happened.
    fn render_message(
        icon: IconName,
        color: Hsla,
        message: SharedString,
        detail: Option<SharedString>,
        cx: &App,
    ) -> AnyElement {
        v_flex()
            .gap_1()
            .child(
                h_flex()
                    .gap_2()
                    .text_sm()
                    .child(Icon::new(icon).small().text_color(color))
                    .child(message),
            )
            .when_some(detail, |this, detail| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(detail),
                )
            })
            .into_any_element()
    }

    fn render_outcome(&self, outcome: &Outcome, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let done = self.calibration.calibrated_count();
        let total = LAYOUT.len();
        let warning = (IconName::TriangleAlert, theme.warning);
        let failed = (IconName::CircleX, theme.danger);
        let succeeded = (IconName::CircleCheck, theme.success);
        let ((icon, color), message, detail): (_, SharedString, Option<SharedString>) =
            match outcome {
                Outcome::All { stopped: false } if done == total => {
                    (succeeded, t!("calibration.all.done").into(), None)
                }
                Outcome::All { stopped } => (
                    warning,
                    if *stopped {
                        t!("calibration.all.stopped", done = done, total = total)
                    } else {
                        t!("calibration.all.partial", done = done, total = total)
                    }
                    .into(),
                    None,
                ),
                Outcome::KeyDone(pos) => (
                    succeeded,
                    t!("calibration.key.done", key = key_label(*pos)).into(),
                    None,
                ),
                Outcome::KeyTimedOut(pos) => (
                    warning,
                    t!("calibration.key.timed_out", key = key_label(*pos)).into(),
                    Some(t!("calibration.key.timed_out.detail").into()),
                ),
                Outcome::NoAnswer => (failed, t!("calibration.no_answer").into(), None),
                Outcome::Unexpected(state) => (
                    failed,
                    t!("calibration.unexpected").into(),
                    Some(t!("calibration.unexpected.detail", state = state).into()),
                ),
                Outcome::Failed(error) => (
                    failed,
                    t!("calibration.start_failed").into(),
                    Some(error.clone().into()),
                ),
                Outcome::Disconnected => (failed, t!("calibration.disconnected").into(), None),
            };
        Self::render_message(icon, color, message, detail, cx)
    }

    /// The buttons that start a calibration, and what the latest one did.
    fn render_start(&self, cx: &mut Context<Self>) -> AnyElement {
        let outcome = match self.calibration.phase() {
            Phase::Ended(outcome) => Some(self.render_outcome(outcome, cx)),
            _ => None,
        };
        let key_button =
            match self.selected {
                Some(pos) => Button::new("calibrate-key")
                    .label(t!("calibration.start_key", key = key_label(pos)))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.begin(Target::Key(pos), window, cx)
                    })),
                None => Button::new("calibrate-key")
                    .label(t!("calibration.start_selected"))
                    .disabled(true),
            };
        v_flex()
            .gap_4()
            .children(outcome)
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("calibrate-all")
                            .label(t!("calibration.start_all"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.begin(Target::All, window, cx)
                            })),
                    )
                    .child(key_button),
            )
            .into_any_element()
    }

    fn render_countdown(&self, seconds_left: u8, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_4()
            .items_start()
            .child(
                div()
                    .text_sm()
                    .child(t!("calibration.countdown", seconds = seconds_left)),
            )
            .child(
                Button::new("cancel-countdown")
                    .label(t!("calibration.cancel"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.cancel_countdown(&CancelCountdown, window, cx)
                    })),
            )
            .into_any_element()
    }

    fn render_running(&self, target: Target, stopping: bool, cx: &mut Context<Self>) -> AnyElement {
        match target {
            Target::All => {
                let done = self.calibration.calibrated_count();
                let total = LAYOUT.len();
                let progress: SharedString =
                    t!("calibration.progress", done = done, total = total).into();
                v_flex()
                    .gap_4()
                    .items_start()
                    .child(div().text_sm().child(t!("calibration.all.running")))
                    .child(
                        h_flex()
                            .gap_3()
                            .child(
                                Progress::new("calibration-progress")
                                    .w_64()
                                    .value(done as f32 / total as f32 * 100.)
                                    .accessibility_label(progress.clone()),
                            )
                            .child(div().text_sm().child(progress)),
                    )
                    .child(
                        Button::new("stop-calibration")
                            .label(if stopping {
                                t!("calibration.stopping")
                            } else {
                                t!("calibration.stop")
                            })
                            .loading(stopping)
                            .on_click(cx.listener(|this, _, _, cx| this.stop(cx))),
                    )
                    .into_any_element()
            }
            Target::Key(pos) => h_flex()
                .gap_2()
                .text_sm()
                .child(Spinner::new().small().color(cx.theme().muted_foreground))
                .child(t!("calibration.key.running", key = key_label(pos)))
                .into_any_element(),
        }
    }

    fn render_controls(&self, cx: &mut Context<Self>) -> AnyElement {
        let connected = self.is_connected(cx);
        match self.calibration.phase() {
            Phase::Countdown { seconds_left, .. } => self.render_countdown(*seconds_left, cx),
            Phase::Running { target, stopping } => self.render_running(*target, *stopping, cx),
            Phase::Ready | Phase::Ended(_) if connected => self.render_start(cx),
            // Unplugged during a run: the message says to connect it again.
            Phase::Ended(outcome) => self.render_outcome(outcome, cx),
            Phase::Ready => div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(t!("calibration.not_connected"))
                .into_any_element(),
        }
    }

    fn render_key(
        &self,
        key: &'static Key,
        enabled: bool,
        focused: Option<KeyPos>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pos = key.pos;
        let theme = cx.theme();
        let calibrated = self.calibration.calibrated().contains(pos);
        let selected = self.selected == Some(pos);
        let face = v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_0p5()
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(key.label),
            )
            .when(calibrated, |this| {
                this.child(
                    Icon::new(IconName::Check)
                        .xsmall()
                        .text_color(theme.success),
                )
            });
        let description: SharedString = if calibrated {
            t!("calibration.key_calibrated", key = key.label).into()
        } else {
            key.label.into()
        };
        if !enabled {
            // During a run the keys show progress and can't be picked.
            return div()
                .size_full()
                .rounded(theme.radius)
                .border_1()
                .border_color(if selected && self.calibration.is_running() {
                    theme.primary
                } else {
                    theme.border
                })
                .child(face)
                .into_any_element();
        }
        Button::new(key.id)
            .outline()
            .size_full()
            .p_0()
            .tab_stop(false)
            .accessibility_label(description)
            .selected(selected)
            .when(selected, |this| this.border_color(theme.primary))
            .when(focused == Some(pos), |this| {
                this.focus_ring_style(window, cx)
            })
            .on_click(cx.listener(move |this, _, window, cx| this.select(pos, window, cx)))
            .child(face)
            .into_any_element()
    }

    fn render_board(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = self.is_board_enabled(cx);
        let focused =
            (enabled && self.board_focus.is_focused(window) && window.last_input_was_keyboard())
                .then(|| self.selected.unwrap_or(LAYOUT[0].pos));
        let keyboard = KeyboardView::new("calibration-keyboard", |key| {
            self.render_key(key, enabled, focused, window, cx)
        });
        div()
            .id("calibration-board")
            .flex_none()
            .role(Role::Group)
            .aria_label(SharedString::from(t!("calibration.board")))
            .when(enabled, |this| {
                this.track_focus(&self.board_focus)
                    .key_context(BOARD_CONTEXT)
                    .on_action(cx.listener(|this, _: &SelectLeft, _, cx| {
                        this.move_selection(Direction::Left, cx)
                    }))
                    .on_action(cx.listener(|this, _: &SelectRight, _, cx| {
                        this.move_selection(Direction::Right, cx)
                    }))
                    .on_action(cx.listener(|this, _: &SelectUp, _, cx| {
                        this.move_selection(Direction::Up, cx)
                    }))
                    .on_action(cx.listener(|this, _: &SelectDown, _, cx| {
                        this.move_selection(Direction::Down, cx)
                    }))
            })
            .child(keyboard)
    }
}

impl Render for CalibrationPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("calibration-page")
            .w_full()
            .p(page::INSET)
            .gap_6()
            .track_focus(&self.focus_handle)
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::cancel_countdown))
            .child(PageHeader::new(t!("calibration.title")))
            .child(div().w(keyboard::WIDTH).child(self.render_controls(cx)))
            .child(self.render_board(window, cx))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use bridge75_core::he::AllCalibration;
    use bridge75_core::matrix::KeyBitmap;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AnyWindowHandle, AppContext as _, Bounds, TestAppContext, WindowBounds, WindowOptions, px,
        size,
    };

    use super::*;
    use crate::preferences::Preferences;
    use crate::session::CalibrationReport;

    actions!(calibration_test, [Typed]);

    /// The page in a headless window, over a session that never reaches a real keyboard.
    fn open(
        cx: &mut TestAppContext,
        session: Session,
    ) -> (AnyWindowHandle, Entity<CalibrationPage>, Entity<Session>) {
        let session = cx.update(|cx| cx.new(|_| session));
        let (window, page) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            cx.set_global(Preferences::default());
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(1180.), px(800.)),
                })),
                ..Default::default()
            };
            let session = session.clone();
            gpui_kit::open_window(options, cx, |window, cx| {
                let page = cx.new(|cx| CalibrationPage::new(session, cx));
                page.update(cx, |page, cx| page.focus(window, cx));
                page
            })
            .unwrap()
        });
        (window, page, session)
    }

    fn phase(page: &Entity<CalibrationPage>, cx: &App) -> Phase {
        page.read(cx).calibration.phase().clone()
    }

    #[gpui_kit::test]
    fn escape_cancels_the_countdown(cx: &mut TestAppContext) {
        let (window, page, _) = open(cx, Session::connected_for_test());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("calibrate-all", cx);
            assert!(matches!(phase(&page, cx), Phase::Countdown { .. }));
            window.press("escape", cx);
            assert_eq!(phase(&page, cx), Phase::Ready);
        })
        .unwrap();
        // Nothing starts later either.
        cx.executor().advance_clock(Duration::from_secs(5));
        cx.update(|cx| assert_eq!(phase(&page, cx), Phase::Ready));
    }

    #[gpui_kit::test]
    fn the_countdown_ends_by_sending_the_start(cx: &mut TestAppContext) {
        let (window, page, _) = open(cx, Session::connected_for_test());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("calibrate-all", cx);
        })
        .unwrap();
        cx.executor().advance_clock(Duration::from_secs(1));
        cx.update(|cx| {
            assert_eq!(
                phase(&page, cx),
                Phase::Countdown {
                    target: Target::All,
                    seconds_left: 2
                }
            )
        });
        cx.executor().advance_clock(Duration::from_secs(2));
        cx.run_until_parked();
        // The test session has no keyboard to send to.
        cx.update(|cx| assert!(matches!(phase(&page, cx), Phase::Ended(Outcome::Failed(_)))));
    }

    #[gpui_kit::test]
    fn a_key_is_picked_on_the_board(cx: &mut TestAppContext) {
        let (window, page, _) = open(cx, Session::connected_for_test());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("a", cx);
            window.press("right", cx);
            let s = LAYOUT.iter().find(|k| k.id == "s").unwrap().pos;
            assert_eq!(page.read(cx).selected, Some(s));
            window.click("calibrate-key", cx);
            assert_eq!(
                phase(&page, cx),
                Phase::Countdown {
                    target: Target::Key(s),
                    seconds_left: run::COUNTDOWN_SECONDS
                }
            );
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn key_presses_are_ignored_while_the_keyboard_calibrates(cx: &mut TestAppContext) {
        let (window, page, session) = open(cx, Session::connected_for_test());
        let typed = Rc::new(Cell::new(0));
        cx.update(|cx| {
            let typed = typed.clone();
            cx.on_action(move |_: &Typed, _| typed.set(typed.get() + 1));
            cx.bind_keys([KeyBinding::new("x", Typed, None)]);
        });
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.press("x", cx);
            assert_eq!(typed.get(), 1);

            page.update(cx, |page, cx| {
                page.calibration.begin(Target::All);
                while page.calibration.tick().is_none() {}
                page.sync(cx);
            });
            window.press("x", cx);
            assert_eq!(typed.get(), 1);
        })
        .unwrap();
        cx.update(|cx| {
            session.update(cx, |_, cx| {
                cx.emit(SessionEvent::Calibration(CalibrationReport::All {
                    state: AllCalibration::Finished,
                    calibrated: KeyBitmap::all(),
                }))
            })
        });
        cx.update_window(window, |_, window, cx| {
            assert_eq!(
                phase(&page, cx),
                Phase::Ended(Outcome::All { stopped: false })
            );
            window.press("x", cx);
            assert_eq!(typed.get(), 2);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn without_a_keyboard_nothing_starts(cx: &mut TestAppContext) {
        let (window, page, _) = open(cx, Session::detached());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.try_find("calibrate-all").is_none());
            page.update(cx, |page, cx| page.begin(Target::All, window, cx));
            assert_eq!(phase(&page, cx), Phase::Ready);
        })
        .unwrap();
    }
}
