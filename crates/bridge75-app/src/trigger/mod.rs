//! Actuation point and Rapid Trigger settings of the selected keys.
//!
//! The board is one Tab stop. While it has focus, the arrow keys move the focused key, Space
//! adds it to or removes it from the selection and Enter selects only it.

mod summary;

use bridge75_core::he::{Travel, TriggerConfig, TriggerMode};
use bridge75_core::matrix::{Key, KeyPos, LAYOUT};
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Selectable as _, Sizable as _, ThemeStyled as _,
    h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, FocusHandle, FontWeight,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Render, Role,
    SharedString, StatefulInteractiveElement as _, Styled as _, Subscription, Window, actions, div,
};
use rust_i18n::t;

use crate::document::{KeyboardDocument, Loadable, MIN_TRAVEL, TriggerEdit, TriggerField};
use crate::keyboard::{self, Direction, KeySelection, KeyboardView};
use crate::page::{self, Field, PageHeader};
use summary::{FieldSummary, SelectionSummary};

actions!(
    trigger,
    [
        /// Undo the newest change of the trigger settings.
        Undo,
        SelectAll,
        ClearSelection,
        FocusLeft,
        FocusRight,
        FocusUp,
        FocusDown,
        /// Add the focused key to the selection, or remove it.
        ToggleFocusedKey,
        /// Select only the focused key.
        SelectFocusedKey,
    ]
);

/// Key context of the whole page.
const CONTEXT: &str = "TriggerPage";
/// Key context of the board while it has focus.
const BOARD_CONTEXT: &str = "TriggerBoard";

/// What a field shows when the selected keys differ.
const MIXED_VALUE: &str = "—";

/// Binds the page's keyboard shortcuts.
pub fn init(cx: &mut App) {
    let mac = cx.platform_info().is_mac();
    cx.bind_keys([
        KeyBinding::new(if mac { "cmd-z" } else { "ctrl-z" }, Undo, Some(CONTEXT)),
        KeyBinding::new(
            if mac { "cmd-a" } else { "ctrl-a" },
            SelectAll,
            Some(CONTEXT),
        ),
        KeyBinding::new("escape", ClearSelection, Some(CONTEXT)),
        KeyBinding::new("left", FocusLeft, Some(BOARD_CONTEXT)),
        KeyBinding::new("right", FocusRight, Some(BOARD_CONTEXT)),
        KeyBinding::new("up", FocusUp, Some(BOARD_CONTEXT)),
        KeyBinding::new("down", FocusDown, Some(BOARD_CONTEXT)),
        KeyBinding::new("space", ToggleFocusedKey, Some(BOARD_CONTEXT)),
        KeyBinding::new("enter", SelectFocusedKey, Some(BOARD_CONTEXT)),
    ]);
}

fn field_id(field: TriggerField) -> &'static str {
    match field {
        TriggerField::Actuation => "actuation",
        TriggerField::Release => "release",
        TriggerField::RtPress => "rt-press",
        TriggerField::RtRelease => "rt-release",
    }
}

fn field_label(field: TriggerField, modes: &[TriggerMode]) -> SharedString {
    match field {
        TriggerField::Actuation => t!("trigger.actuation"),
        // In Rapid Trigger with exit point, the release point ends Rapid Trigger.
        TriggerField::Release
            if modes
                .iter()
                .filter(|m| m.uses_release_point())
                .all(|m| *m == TriggerMode::RapidSplit) =>
        {
            t!("trigger.rt_exit")
        }
        TriggerField::Release => t!("trigger.release"),
        TriggerField::RtPress => t!("trigger.rt_press"),
        TriggerField::RtRelease => t!("trigger.rt_release"),
    }
    .into()
}

fn mode_label(mode: TriggerMode) -> SharedString {
    match mode {
        TriggerMode::Normal => t!("trigger.mode.normal"),
        TriggerMode::Split => t!("trigger.mode.split"),
        TriggerMode::Rapid => t!("trigger.mode.rapid"),
        TriggerMode::RapidSplit => t!("trigger.mode.rapid_split"),
    }
    .into()
}

/// The short mark of a mode on the keycaps and in the legend.
fn mode_mark(mode: TriggerMode) -> SharedString {
    match mode {
        TriggerMode::Normal => t!("trigger.mark.normal"),
        TriggerMode::Split => t!("trigger.mark.split"),
        TriggerMode::Rapid => t!("trigger.mark.rapid"),
        TriggerMode::RapidSplit => t!("trigger.mark.rapid_split"),
    }
    .into()
}

fn mm(travel: Travel) -> SharedString {
    t!("unit.mm", value = travel.to_string()).into()
}

/// Where the slider shows `travel`. The slider cannot show values outside the range it sets.
fn slider_value(travel: Travel) -> f32 {
    travel.clamp(MIN_TRAVEL, Travel::MAX).mm() as f32
}

fn travel_from_slider(value: f32) -> Travel {
    let raw = (value * 10.0)
        .round()
        .clamp(MIN_TRAVEL.raw() as f32, Travel::MAX.raw() as f32);
    Travel::from_raw(raw as u8)
}

pub struct TriggerPage {
    document: Entity<KeyboardDocument>,
    selection: KeySelection,
    /// Indexed by [`TriggerField::ix`].
    sliders: [Entity<SliderState>; 4],
    /// The value under a slider thumb while it is dragged, not yet written.
    dragging: Option<(TriggerField, Travel)>,
    /// The key the arrow keys move from. `None` until a key is clicked or moved to.
    focused_key: Option<KeyPos>,
    /// Keeps keyboard focus on the page when its background is clicked.
    focus_handle: FocusHandle,
    board_focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl TriggerPage {
    pub fn new(
        document: Entity<KeyboardDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let sliders = TriggerField::ALL.map(|_| {
            cx.new(|_| {
                SliderState::new()
                    .min(slider_value(MIN_TRAVEL))
                    .max(slider_value(Travel::MAX))
                    .step(0.1)
                    .default_value(slider_value(TriggerConfig::DEFAULT.actuation))
            })
        });
        let mut subscriptions = vec![cx.observe_in(&document, window, |this, _, window, cx| {
            this.sync_sliders(window, cx)
        })];
        for (field, slider) in TriggerField::ALL.into_iter().zip(&sliders) {
            subscriptions.push(
                cx.subscribe_in(slider, window, move |this, _, event, _, cx| {
                    match event {
                        // The value label follows the thumb while dragging.
                        SliderEvent::Change(value) => {
                            this.dragging = Some((field, travel_from_slider(value.start())));
                            cx.notify();
                        }
                        SliderEvent::Release(value) => {
                            this.dragging = None;
                            let travel = travel_from_slider(value.start());
                            this.commit(TriggerEdit::Set(field, travel), cx);
                        }
                    }
                }),
            );
        }
        TriggerPage {
            document,
            selection: KeySelection::default(),
            sliders,
            dragging: None,
            focused_key: None,
            focus_handle: cx.focus_handle(),
            board_focus: cx.focus_handle().tab_stop(true),
            _subscriptions: subscriptions,
        }
    }

    /// Moves keyboard focus into the page. The shell calls this when the page is opened.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus_handle.focus(window, cx);
    }

    fn slider(&self, field: TriggerField) -> &Entity<SliderState> {
        &self.sliders[field.ix()]
    }

    fn summary(&self, cx: &App) -> Option<SelectionSummary> {
        let table = self.document.read(cx).triggers().loaded()?;
        SelectionSummary::new(table, &self.selection.keys())
    }

    fn is_loaded(&self, cx: &App) -> bool {
        self.document.read(cx).triggers().loaded().is_some()
    }

    /// Moves each slider to the value of the first selected key that uses it.
    fn sync_sliders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(summary) = self.summary(cx) else {
            return;
        };
        for field in TriggerField::ALL {
            let dragged = self.dragging.is_some_and(|(f, _)| f == field);
            if let Some(value) = summary.field(field).filter(|_| !dragged) {
                self.slider(field).update(cx, |slider, cx| {
                    slider.set_value(slider_value(value.first()), window, cx)
                });
            }
        }
    }

    fn commit(&mut self, edit: TriggerEdit, cx: &mut Context<Self>) {
        let keys = self.selection.keys();
        self.document
            .update(cx, |document, cx| document.edit_triggers(&keys, edit, cx));
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.document.update(cx, |document, cx| document.undo(cx));
    }

    fn change_selection(
        &mut self,
        change: impl FnOnce(&mut KeySelection),
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        change(&mut self.selection);
        self.sync_sliders(window, cx);
        cx.notify();
    }

    fn select_all(&mut self, _: &SelectAll, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_loaded(cx) {
            self.change_selection(KeySelection::select_all, window, cx);
        }
    }

    fn clear_selection(&mut self, _: &ClearSelection, window: &mut Window, cx: &mut Context<Self>) {
        if self.selection.is_empty() {
            cx.propagate();
            return;
        }
        self.change_selection(KeySelection::clear, window, cx);
    }

    fn click_key(
        &mut self,
        pos: KeyPos,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let modifiers = event.modifiers();
        let extend = modifiers.control || modifiers.platform || modifiers.shift;
        self.focused_key = Some(pos);
        // Keycaps are not focusable, so the arrow keys continue from the clicked key.
        self.board_focus.focus(window, cx);
        self.change_selection(|selection| selection.click(pos, extend), window, cx);
    }

    /// The key the arrow keys, Space and Enter act on.
    fn current_key(&self) -> KeyPos {
        self.focused_key
            .or_else(|| self.selection.first())
            .unwrap_or(LAYOUT[0].pos)
    }

    fn move_focus(&mut self, direction: Direction, cx: &mut Context<Self>) {
        if let Some(next) = keyboard::neighbor(self.current_key(), direction) {
            self.focused_key = Some(next);
            cx.notify();
        }
    }

    fn render_key(
        &self,
        key: &'static Key,
        focused: Option<KeyPos>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pos = key.pos;
        let config = self
            .document
            .read(cx)
            .triggers()
            .loaded()
            .and_then(|t| t.get(pos))
            .copied();
        let selected = self.selection.contains(pos);
        let theme = cx.theme();
        let (muted, primary) = (theme.muted_foreground, theme.primary);
        let description: SharedString = match config {
            Some(config) => format!(
                "{} {} {}",
                key.label,
                mm(config.actuation),
                mode_label(config.mode)
            )
            .into(),
            None => key.label.into(),
        };
        Button::new(key.id)
            .outline()
            .size_full()
            .p_0()
            .tab_stop(false)
            .accessibility_label(description)
            .selected(selected)
            .disabled(config.is_none())
            .when(selected, |this| this.border_color(primary))
            .when(focused == Some(pos), |this| {
                this.focus_ring_style(window, cx)
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.click_key(pos, event, window, cx)
            }))
            .child(
                v_flex()
                    .size_full()
                    .items_center()
                    .justify_center()
                    .gap_0p5()
                    .child(div().text_xs().text_color(muted).child(key.label))
                    .when_some(config, |this, config| {
                        this.child(
                            h_flex()
                                .gap_0p5()
                                .child(div().text_sm().child(config.actuation.to_string()))
                                // A fixed slot, so the number does not move with the mode.
                                .child(
                                    div()
                                        .w_4()
                                        .flex_none()
                                        .text_xs()
                                        .text_color(muted)
                                        .child(mode_mark(config.mode)),
                                ),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_board(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let loaded = self.is_loaded(cx);
        let focused = (self.board_focus.is_focused(window) && window.last_input_was_keyboard())
            .then(|| self.current_key());
        let keyboard = KeyboardView::new("trigger-keyboard", |key| {
            self.render_key(key, focused, window, cx)
        });
        div()
            .id("trigger-board")
            .flex_none()
            .role(Role::Group)
            .aria_label(SharedString::from(t!("trigger.board")))
            .when(loaded, |this| {
                this.track_focus(&self.board_focus)
                    .key_context(BOARD_CONTEXT)
                    .on_action(cx.listener(|this, _: &FocusLeft, _, cx| {
                        this.move_focus(Direction::Left, cx)
                    }))
                    .on_action(cx.listener(|this, _: &FocusRight, _, cx| {
                        this.move_focus(Direction::Right, cx)
                    }))
                    .on_action(
                        cx.listener(|this, _: &FocusUp, _, cx| this.move_focus(Direction::Up, cx)),
                    )
                    .on_action(cx.listener(|this, _: &FocusDown, _, cx| {
                        this.move_focus(Direction::Down, cx)
                    }))
                    .on_action(cx.listener(|this, _: &ToggleFocusedKey, window, cx| {
                        let pos = this.current_key();
                        this.focused_key = Some(pos);
                        this.change_selection(|s| s.toggle(pos), window, cx)
                    }))
                    .on_action(cx.listener(|this, _: &SelectFocusedKey, window, cx| {
                        let pos = this.current_key();
                        this.focused_key = Some(pos);
                        this.change_selection(|s| s.select_only(pos), window, cx)
                    }))
            })
            .child(keyboard)
    }

    fn render_legend(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .w(keyboard::WIDTH)
            .flex_wrap()
            .gap_x_4()
            .gap_y_1()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(t!("trigger.legend.actuation"))
            .children(TriggerMode::ALL.map(|mode| {
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(mode_mark(mode)),
                    )
                    .child(mode_label(mode))
            }))
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let count: SharedString = match self.selection.len() {
            1 => t!("trigger.selected_one").into(),
            n => t!("trigger.selected_many", count = n).into(),
        };
        let has_undo = self.document.read(cx).has_undo_history();
        h_flex()
            .w(keyboard::WIDTH)
            .gap_2()
            .child(
                Button::new("select-all")
                    .small()
                    .label(t!("trigger.select_all"))
                    .tooltip_with_action(t!("trigger.select_all"), &SelectAll, Some(CONTEXT))
                    .on_click(
                        cx.listener(|this, _, window, cx| this.select_all(&SelectAll, window, cx)),
                    ),
            )
            .child(
                Button::new("clear-selection")
                    .small()
                    .label(t!("trigger.clear_selection"))
                    .tooltip_with_action(
                        t!("trigger.clear_selection"),
                        &ClearSelection,
                        Some(CONTEXT),
                    )
                    .disabled(self.selection.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.change_selection(KeySelection::clear, window, cx)
                    })),
            )
            .when(!self.selection.is_empty(), |this| {
                this.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(count),
                )
            })
            .child(div().flex_1())
            .child(
                Button::new("undo")
                    .small()
                    .icon(IconName::Undo2)
                    .label(t!("trigger.undo"))
                    .tooltip_with_action(t!("trigger.undo_tooltip"), &Undo, Some(CONTEXT))
                    .disabled(!has_undo)
                    .on_click(cx.listener(|this, _, window, cx| this.undo(&Undo, window, cx))),
            )
    }

    fn render_step(
        &self,
        field: TriggerField,
        delta: i8,
        value: &FieldSummary,
        label: &SharedString,
        cx: &mut Context<Self>,
    ) -> Button {
        let (ix, icon, name, tooltip) = if delta < 0 {
            let name = t!("trigger.decrease_field", field = label);
            (0usize, IconName::Minus, name, t!("trigger.decrease"))
        } else {
            let name = t!("trigger.increase_field", field = label);
            (1usize, IconName::Plus, name, t!("trigger.increase"))
        };
        Button::new((field_id(field), ix))
            .ghost()
            .small()
            .icon(icon)
            .accessibility_label(name)
            .tooltip(tooltip)
            .disabled(!value.is_steppable(delta))
            .on_click(
                cx.listener(move |this, _, _, cx| this.commit(TriggerEdit::Step(field, delta), cx)),
            )
    }

    fn render_field(
        &self,
        field: TriggerField,
        summary: &SelectionSummary,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let value = summary.field(field)?;
        let label = field_label(field, summary.modes());
        let shown: SharedString = match self.dragging {
            Some((f, travel)) if f == field => mm(travel),
            _ => value.same().map_or(MIXED_VALUE.into(), mm),
        };
        let control = h_flex()
            .gap_3()
            .child(
                Slider::new(self.slider(field))
                    .flex_1()
                    .accessibility_label(label.clone()),
            )
            .child(self.render_step(field, -1, value, &label, cx))
            .child(div().w_16().text_center().text_sm().child(shown))
            .child(self.render_step(field, 1, value, &label, cx));
        let note = value.is_mixed().then(|| t!("trigger.mixed").into());
        Some(Field::new(label, control).note(note).into_any_element())
    }

    /// Two fields side by side, so related settings share a row.
    fn render_row(&self, fields: [Option<AnyElement>; 2]) -> Option<AnyElement> {
        if fields.iter().all(Option::is_none) {
            return None;
        }
        Some(
            h_flex()
                .items_start()
                .gap_6()
                .children(fields.map(|field| div().flex_1().min_w_0().children(field)))
                .into_any_element(),
        )
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(summary) = self.summary(cx) else {
            return div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(t!("trigger.no_selection"))
                .into_any_element();
        };
        let mode_ix = summary
            .mode()
            .and_then(|mode| TriggerMode::ALL.iter().position(|m| *m == mode));
        let mode = RadioGroup::horizontal("trigger-mode")
            .children(TriggerMode::ALL.map(mode_label))
            .selected_index(mode_ix)
            .on_change(cx.listener(|this, ix: &usize, _, cx| {
                this.commit(TriggerEdit::Mode(TriggerMode::ALL[*ix]), cx)
            }));
        let note = mode_ix.is_none().then(|| t!("trigger.mixed").into());
        let mut field = |field| self.render_field(field, &summary, cx);
        let travel =
            self.render_row([field(TriggerField::Actuation), field(TriggerField::Release)]);
        let rapid = self.render_row([field(TriggerField::RtPress), field(TriggerField::RtRelease)]);
        v_flex()
            .w(keyboard::WIDTH)
            .gap_4()
            .child(Field::new(SharedString::from(t!("trigger.mode")), mode).note(note))
            .children(travel)
            .children(rapid)
            .into_any_element()
    }

    fn render_status(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let muted = cx.theme().muted_foreground;
        let message = |text: SharedString| div().text_sm().text_color(muted).child(text);
        match self.document.read(cx).triggers() {
            Loadable::Loaded(_) => None,
            Loadable::Idle => Some(message(t!("keyboard.not_connected").into()).into_any_element()),
            Loadable::Loading => Some(
                h_flex()
                    .gap_2()
                    .child(Spinner::new().small().color(muted))
                    .child(message(t!("trigger.loading").into()))
                    .into_any_element(),
            ),
            Loadable::Failed(reason) => Some(
                v_flex()
                    .gap_2()
                    .items_start()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child(t!("trigger.load_failed")))
                            .child(div().text_xs().text_color(muted).child(reason.clone())),
                    )
                    .child(
                        Button::new("reload-triggers")
                            .small()
                            .label(t!("trigger.retry"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.document.update(cx, |document, cx| document.reload(cx))
                            })),
                    )
                    .into_any_element(),
            ),
        }
    }
}

impl Render for TriggerPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.render_status(cx);
        let loaded = status.is_none();

        // The header and board stay in place; only the editor scrolls when the window is short.
        v_flex()
            .id("trigger-page")
            .flex_1()
            .min_h_0()
            .w_full()
            .track_focus(&self.focus_handle)
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::clear_selection))
            .child(
                v_flex()
                    .flex_none()
                    .p(page::INSET)
                    .gap_6()
                    .child(PageHeader::new(
                        t!("trigger.title"),
                        t!("trigger.description"),
                    ))
                    .children(status)
                    .child(
                        v_flex()
                            .gap_2()
                            .child(self.render_board(window, cx))
                            .when(loaded, |this| this.child(self.render_legend(cx))),
                    )
                    .when(loaded, |this| this.child(self.render_toolbar(cx))),
            )
            .when(loaded, |this| {
                this.child(
                    div()
                        .id("trigger-editor")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .child(
                            div()
                                .px(page::INSET)
                                .pb(page::INSET)
                                .child(self.render_editor(cx)),
                        ),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use gpui_kit::component::kbd::Kbd;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AnyWindowHandle, Bounds, Modifiers, TestAppContext, WindowBounds, WindowOptions, px, size,
    };

    use super::*;

    fn pos(id: &str) -> KeyPos {
        LAYOUT.iter().find(|k| k.id == id).unwrap().pos
    }

    /// The page in a headless window, over settings that never reach a real keyboard.
    fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<TriggerPage>) {
        cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(1180.), px(800.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                let configs = LAYOUT
                    .iter()
                    .map(|k| (k.pos, TriggerConfig::DEFAULT))
                    .collect();
                let document = cx.new(|cx| KeyboardDocument::for_test(configs, cx));
                cx.new(|cx| TriggerPage::new(document, window, cx))
            })
            .unwrap()
        })
    }

    fn selected(page: &Entity<TriggerPage>, cx: &App) -> Vec<KeyPos> {
        page.read(cx).selection.keys()
    }

    #[gpui_kit::test]
    fn the_board_is_operated_from_the_keyboard(cx: &mut TestAppContext) {
        let (window, page) = open(cx);
        let select_all = if cx.update(|cx| cx.platform_info().is_mac()) {
            "cmd-a"
        } else {
            "ctrl-a"
        };
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            page.update(cx, |page, cx| page.board_focus.focus(window, cx));
            window.render_frame(cx);

            // From Esc, one key to the right, then down into the number row.
            window.press("right", cx);
            window.press("space", cx);
            assert_eq!(selected(&page, cx), [pos("f1")]);
            window.press("right", cx);
            window.press("space", cx);
            assert_eq!(selected(&page, cx), [pos("f1"), pos("f2")]);
            window.press("space", cx);
            assert_eq!(selected(&page, cx), [pos("f1")]);
            window.press("down", cx);
            window.press("enter", cx);
            assert_eq!(selected(&page, cx), [pos("2")]);

            window.press(select_all, cx);
            assert_eq!(selected(&page, cx).len(), LAYOUT.len());
            window.press("escape", cx);
            assert!(selected(&page, cx).is_empty());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn clicks_select_and_modified_clicks_extend(cx: &mut TestAppContext) {
        let (window, page) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("w", cx);
            assert_eq!(selected(&page, cx), [pos("w")]);
            window.click_with_modifiers("a", Modifiers::control(), cx);
            assert_eq!(selected(&page, cx), [pos("w"), pos("a")]);
            // The arrow keys continue from the clicked key.
            assert!(page.read(cx).board_focus.is_focused(window));
            window.press("right", cx);
            window.press("enter", cx);
            assert_eq!(selected(&page, cx), [pos("s")]);
            window.render_frame(cx);
            assert_eq!(window.find("undo").disabled(), Some(true));
            // The tooltips can show the shortcuts.
            for action in [&Undo as &dyn gpui_kit::Action, &SelectAll, &ClearSelection] {
                assert!(Kbd::binding_for_action(action, Some(CONTEXT), window).is_some());
            }
        })
        .unwrap();
    }
}
