//! SOCD pairs: two keys of which at most one is sent while both are held.
//!
//! The keyboard matches pairs against the key codes keys send, so a pair follows the keymap.
//! Each pair keeps its slot, which keys that toggle one pair refer to.

mod document;
mod settings;

use bridge75_core::hid::Usage;
use bridge75_core::keymap::{KeyAction, SOCD_PAIRS};
use bridge75_core::socd::{SocdBehavior, SocdMode, SocdPair};
use gpui_kit::assets::IconName as AppIconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, Context, Entity, FocusHandle, InteractiveElement as _, IntoElement,
    KeyBinding, ParentElement as _, Rems, Render, SharedString, Styled as _, Subscription, Window,
    actions, div, rems,
};
use rust_i18n::t;

use crate::document::Loadable;
use crate::keyboard;
use crate::keymap::assignment::{Category, label, short_label};
use crate::page::{self, Field, PageHeader};
pub use document::{SocdDocument, new};
use settings::SocdState;

actions!(
    socd,
    [
        /// Undo the newest change of the SOCD settings.
        Undo,
    ]
);

/// Key context of the whole page.
const CONTEXT: &str = "SocdPage";

/// Widths of the columns of the pair list. A row fits the width every page has, the width of
/// the board; the test below keeps them in step.
const NUMBER_WIDTH: Rems = rems(2.);
const KEY_WIDTH: Rems = rems(7.5);
const BEHAVIOR_WIDTH: Rems = rems(12.);
const MODE_WIDTH: Rems = rems(10.);
const STATE_WIDTH: Rems = rems(4.5);
/// Width of a small icon button, as GPUI Component draws it.
const REMOVE_WIDTH: Rems = rems(1.5);
/// Space between the columns.
const COLUMN_GAP: Rems = rems(0.5);

/// Binds the page's keyboard shortcuts.
pub fn init(cx: &mut App) {
    let mac = cx.platform_info().is_mac();
    cx.bind_keys([KeyBinding::new(
        if mac { "cmd-z" } else { "ctrl-z" },
        Undo,
        Some(CONTEXT),
    )]);
}

/// The modes in the order of their menu.
const MODES: [SocdMode; 3] = [SocdMode::Always, SocdMode::Global, SocdMode::Off];

/// The behaviors in the order of their menu.
const BEHAVIORS: [SocdBehavior; 4] = [
    SocdBehavior::Last,
    SocdBehavior::A,
    SocdBehavior::B,
    SocdBehavior::Neutral,
];

fn mode_label(mode: SocdMode) -> SharedString {
    match mode {
        SocdMode::Always => t!("socd.mode.always"),
        SocdMode::Global => t!("socd.mode.switch"),
        SocdMode::Off => t!("socd.mode.off"),
    }
    .into()
}

/// The name of a key of a pair. A new pair, or a slot read from the keyboard, may lack one.
fn key_label(usage: Usage) -> SharedString {
    if usage == Usage::NONE {
        t!("socd.no_key").into()
    } else {
        label(KeyAction::Key(usage))
    }
}

/// The shorter name of a key, as it fits on a button.
fn short_key_label(usage: Usage) -> SharedString {
    if usage == Usage::NONE {
        t!("socd.no_key").into()
    } else {
        short_label(KeyAction::Key(usage))
    }
}

fn behavior_label(behavior: SocdBehavior, pair: &SocdPair) -> SharedString {
    match behavior {
        SocdBehavior::Last => t!("socd.behavior.last"),
        SocdBehavior::A => t!("socd.behavior.wins", key = short_key_label(pair.a)),
        SocdBehavior::B => t!("socd.behavior.wins", key = short_key_label(pair.b)),
        SocdBehavior::Neutral => t!("socd.behavior.neutral"),
    }
    .into()
}

/// The sections of the key menu: every section of the keymap picker that has plain keys.
fn key_sections() -> Vec<(SharedString, Vec<Usage>)> {
    Category::ALL
        .into_iter()
        .filter_map(|category| {
            let usages: Vec<Usage> = category
                .actions()
                .into_iter()
                .filter_map(|action| match action {
                    KeyAction::Key(usage) => Some(usage),
                    _ => None,
                })
                .collect();
            (!usages.is_empty()).then(|| (category.title(), usages))
        })
        .collect()
}

/// Which key of a pair.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Side {
    A,
    B,
}

impl Side {
    fn get(self, pair: &SocdPair) -> Usage {
        match self {
            Side::A => pair.a,
            Side::B => pair.b,
        }
    }

    fn other(self, pair: &SocdPair) -> Usage {
        match self {
            Side::A => pair.b,
            Side::B => pair.a,
        }
    }

    fn with(self, pair: SocdPair, usage: Usage) -> SocdPair {
        match self {
            Side::A => SocdPair { a: usage, ..pair },
            Side::B => SocdPair { b: usage, ..pair },
        }
    }

    fn id(self) -> &'static str {
        match self {
            Side::A => "socd-key-a",
            Side::B => "socd-key-b",
        }
    }
}

pub struct SocdPage {
    document: Entity<SocdDocument>,
    /// Keeps keyboard focus on the page when its background is clicked.
    focus_handle: FocusHandle,
    /// The add button and the remove button of each slot, so focus can move to the next one
    /// when the focused pair is removed.
    add_focus: FocusHandle,
    remove_focus: Vec<FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

impl SocdPage {
    pub fn new(document: Entity<SocdDocument>, cx: &mut Context<Self>) -> Self {
        let subscriptions = vec![cx.observe(&document, |_, _, cx| cx.notify())];
        SocdPage {
            document,
            focus_handle: cx.focus_handle(),
            add_focus: cx.focus_handle(),
            remove_focus: (0..SOCD_PAIRS).map(|_| cx.focus_handle()).collect(),
            _subscriptions: subscriptions,
        }
    }

    /// Moves keyboard focus into the page and reads again what keys on the keyboard may have
    /// toggled. The shell calls this when the page is opened.
    pub fn open(&self, window: &mut Window, cx: &mut App) {
        self.focus_handle.focus(window, cx);
        self.document
            .update(cx, |document, cx| document.refresh_status(cx));
    }

    fn state(&self, cx: &App) -> Option<SocdState> {
        self.document
            .read(cx)
            .settings()
            .loaded()
            .map(|settings| *settings.state())
    }

    fn edit_pair(
        &mut self,
        index: usize,
        edit: impl FnOnce(SocdPair) -> SocdPair,
        cx: &mut Context<Self>,
    ) {
        let Some(pair) = self.state(cx).map(|s| s.pair(index)) else {
            return;
        };
        let pair = edit(pair);
        self.document
            .update(cx, |document, cx| document.set_pair(index, pair, cx));
    }

    fn set_key(&mut self, index: usize, side: Side, usage: Usage, cx: &mut Context<Self>) {
        self.edit_pair(index, |pair| side.with(pair, usage), cx);
    }

    /// Removes the pair in `index` and moves focus to the remove button of the next pair, or
    /// of the previous one after the last, or to the add button once none is left.
    fn remove_pair(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.document
            .update(cx, |document, cx| document.remove_pair(index, cx));
        let Some(state) = self.state(cx) else {
            return;
        };
        let used: Vec<usize> = state.used().collect();
        let next = used
            .iter()
            .find(|i| **i > index)
            .or_else(|| used.last())
            .map_or(&self.add_focus, |i| &self.remove_focus[*i]);
        next.focus(window, cx);
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        self.document.update(cx, |document, cx| document.undo(cx));
    }

    /// A button that opens a menu of keys for one side of a pair. The key on the other side
    /// can't be picked, as a pair needs two different keys.
    fn render_key_button(
        &self,
        index: usize,
        side: Side,
        pair: &SocdPair,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = cx.entity();
        let (current, other) = (side.get(pair), side.other(pair));
        let name = match side {
            Side::A => t!("socd.key_a_of", pair = index),
            Side::B => t!("socd.key_b_of", pair = index),
        };
        Button::new((side.id(), index))
            .outline()
            .small()
            .w(KEY_WIDTH)
            .dropdown_caret(true)
            .label(short_key_label(current))
            .accessibility_label(SharedString::from(format!(
                "{name}: {}",
                key_label(current)
            )))
            .when(short_key_label(current) != key_label(current), |this| {
                this.tooltip(key_label(current))
            })
            .dropdown_menu(move |menu, window, cx| {
                let rem = window.rem_size();
                key_sections()
                    .into_iter()
                    .fold(menu, |menu, (title, usages)| {
                        let page = page.clone();
                        menu.submenu(title, window, cx, move |submenu, window, _| {
                            // Long sections scroll rather than run off the window.
                            let submenu = submenu.scrollable(true).max_h(rems(24.).to_pixels(rem));
                            usages.iter().fold(submenu, |submenu, usage| {
                                let usage = *usage;
                                submenu.item(
                                    PopupMenuItem::new(label(KeyAction::Key(usage)))
                                        .checked(usage == current)
                                        .disabled(usage == other)
                                        .on_click(window.listener_for(
                                            &page,
                                            move |this: &mut Self, _, _, cx| {
                                                this.set_key(index, side, usage, cx)
                                            },
                                        )),
                                )
                            })
                        })
                    })
            })
            .into_any_element()
    }

    /// A button that opens a menu of `options`, one of which is `current`.
    fn render_choice<T: Copy + PartialEq + 'static>(
        id: (&'static str, usize),
        width: Rems,
        name: SharedString,
        options: Vec<(T, SharedString)>,
        current: T,
        choose: impl Fn(&mut Self, T, &mut Context<Self>) + Copy + 'static,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let page = cx.entity();
        let shown = options
            .iter()
            .find(|(value, _)| *value == current)
            .map(|(_, label)| label.clone())
            .unwrap_or_default();
        Button::new(id)
            .outline()
            .small()
            .w(width)
            .dropdown_caret(true)
            .label(shown.clone())
            .accessibility_label(SharedString::from(format!("{name}: {shown}")))
            .dropdown_menu(move |menu, window, _| {
                options.iter().fold(menu, |menu, (value, label)| {
                    let value = *value;
                    menu.item(
                        PopupMenuItem::new(label.clone())
                            .checked(value == current)
                            .on_click(
                                window.listener_for(&page, move |this: &mut Self, _, _, cx| {
                                    choose(this, value, cx)
                                }),
                            ),
                    )
                })
            })
            .into_any_element()
    }

    fn render_header_row(&self, cx: &App) -> impl IntoElement {
        let cell =
            |width: Rems, text: SharedString| div().w(width).flex_none().truncate().child(text);
        h_flex()
            .gap(COLUMN_GAP)
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(cell(NUMBER_WIDTH, t!("socd.column.pair").into()))
            .child(cell(KEY_WIDTH, t!("socd.column.key_a").into()))
            .child(cell(KEY_WIDTH, t!("socd.column.key_b").into()))
            .child(cell(BEHAVIOR_WIDTH, t!("socd.column.behavior").into()))
            .child(cell(MODE_WIDTH, t!("socd.column.mode").into()))
            .child(cell(STATE_WIDTH, t!("socd.column.state").into()))
    }

    fn render_pair(&self, index: usize, state: &SocdState, cx: &mut Context<Self>) -> AnyElement {
        let pair = state.pair(index);
        let active = state.is_active(index);
        let muted = cx.theme().muted_foreground;
        let behavior = Self::render_choice(
            ("socd-behavior", index),
            BEHAVIOR_WIDTH,
            t!("socd.behavior_of", pair = index).into(),
            BEHAVIORS
                .map(|behavior| (behavior, behavior_label(behavior, &pair)))
                .to_vec(),
            pair.behavior,
            move |this, behavior, cx| {
                this.edit_pair(index, |pair| SocdPair { behavior, ..pair }, cx)
            },
            cx,
        );
        let mode = Self::render_choice(
            ("socd-mode", index),
            MODE_WIDTH,
            t!("socd.mode_of", pair = index).into(),
            MODES.map(|mode| (mode, mode_label(mode))).to_vec(),
            pair.mode,
            move |this, mode, cx| this.edit_pair(index, |pair| SocdPair { mode, ..pair }, cx),
            cx,
        );
        let state = if active {
            Tag::success()
                .outline()
                .small()
                .child(t!("socd.state.active"))
        } else {
            Tag::secondary()
                .outline()
                .small()
                .child(t!("socd.state.inactive"))
        };
        h_flex()
            .id(("socd-pair", index))
            .gap(COLUMN_GAP)
            .child(
                div()
                    .w(NUMBER_WIDTH)
                    .flex_none()
                    .text_sm()
                    .text_color(muted)
                    .child(index.to_string()),
            )
            .child(self.render_key_button(index, Side::A, &pair, cx))
            .child(self.render_key_button(index, Side::B, &pair, cx))
            .child(behavior)
            .child(mode)
            .child(h_flex().w(STATE_WIDTH).flex_none().child(state))
            .child(
                h_flex().w(REMOVE_WIDTH).flex_none().child(
                    Button::new(("socd-remove", index))
                        .track_focus(&self.remove_focus[index])
                        .ghost()
                        .small()
                        .icon(AppIconName::Trash)
                        .accessibility_label(SharedString::from(t!(
                            "socd.remove_pair",
                            pair = index
                        )))
                        .tooltip(t!("socd.remove"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            this.remove_pair(index, window, cx)
                        })),
                ),
            )
            .into_any_element()
    }

    fn render_switch(&self, state: &SocdState, cx: &mut Context<Self>) -> impl IntoElement {
        let on = state.switch();
        let label: SharedString = t!("socd.switch").into();
        Field::new(
            label.clone(),
            Switch::new("socd-switch")
                .checked(on)
                .accessibility_label(label)
                .label(SharedString::from(if on {
                    t!("socd.switch.on")
                } else {
                    t!("socd.switch.off")
                }))
                .on_change(cx.listener(|this, on: &bool, _, cx| {
                    let on = *on;
                    this.document
                        .update(cx, |document, cx| document.set_switch(on, cx))
                })),
        )
    }

    fn render_toolbar(&self, state: &SocdState, cx: &mut Context<Self>) -> impl IntoElement {
        let has_undo = self.document.read(cx).has_undo_history();
        h_flex()
            .gap_2()
            .child(
                Button::new("socd-add")
                    .track_focus(&self.add_focus)
                    .small()
                    .icon(IconName::Plus)
                    .label(t!("socd.add"))
                    .disabled(state.first_free().is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.document.update(cx, |document, cx| {
                            document.add_pair(cx);
                        })
                    })),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!(
                        "socd.count",
                        count = state.used().count(),
                        max = SOCD_PAIRS
                    )),
            )
            .child(div().flex_1())
            .child(
                Button::new("socd-undo")
                    .small()
                    .icon(IconName::Undo2)
                    .label(t!("socd.undo"))
                    .tooltip_with_action(t!("socd.undo"), &Undo, Some(CONTEXT))
                    .disabled(!has_undo)
                    .on_click(cx.listener(|this, _, window, cx| this.undo(&Undo, window, cx))),
            )
    }

    fn render_pairs(&self, state: &SocdState, cx: &mut Context<Self>) -> AnyElement {
        let used: Vec<usize> = state.used().collect();
        if used.is_empty() {
            return div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(t!("socd.no_pairs"))
                .into_any_element();
        }
        v_flex()
            .gap_2()
            .child(self.render_header_row(cx))
            .children(
                used.into_iter()
                    .map(|index| self.render_pair(index, state, cx)),
            )
            .into_any_element()
    }

    fn render_loaded(&self, state: &SocdState, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_6()
            .child(self.render_switch(state, cx))
            .child(
                v_flex()
                    .w(keyboard::WIDTH)
                    .gap_3()
                    .child(self.render_toolbar(state, cx))
                    .child(self.render_pairs(state, cx)),
            )
            .into_any_element()
    }

    fn render_status(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let message = |text: SharedString| div().text_sm().text_color(muted).child(text);
        match self.document.read(cx).settings() {
            Loadable::Loaded(settings) => {
                let state = *settings.state();
                self.render_loaded(&state, cx)
            }
            Loadable::Idle => message(t!("keyboard.not_connected").into()).into_any_element(),
            Loadable::Loading => h_flex()
                .gap_2()
                .child(Spinner::new().small().color(muted))
                .child(message(t!("socd.loading").into()))
                .into_any_element(),
            Loadable::Failed(reason) => v_flex()
                .gap_2()
                .items_start()
                .child(
                    v_flex()
                        .gap_1()
                        .child(div().text_sm().child(t!("socd.load_failed")))
                        .child(div().text_xs().text_color(muted).child(reason.clone())),
                )
                .child(
                    Button::new("socd-reload")
                        .small()
                        .label(t!("socd.retry"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.document.update(cx, |document, cx| document.reload(cx))
                        })),
                )
                .into_any_element(),
        }
    }
}

impl Render for SocdPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("socd-page")
            .w_full()
            .p(page::INSET)
            .gap_6()
            .track_focus(&self.focus_handle)
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::undo))
            .child(PageHeader::new(t!("socd.title")).description(t!("socd.description")))
            .child(self.render_status(cx))
    }
}

#[cfg(test)]
mod tests {
    use bridge75_core::socd::{SocdPairs, SocdStatus};
    use gpui_kit::component::kbd::Kbd;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AnyWindowHandle, AppContext as _, Bounds, TestAppContext, WindowBounds, WindowOptions, px,
        size,
    };

    use super::*;

    fn usage(name: &str) -> Usage {
        name.parse().unwrap()
    }

    /// The page in a headless window, over settings that never reach a real keyboard.
    fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<SocdPage>, Entity<SocdDocument>) {
        let mut document = None;
        let (window, page) = cx.update(|cx| {
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
                let status = SocdStatus {
                    global: false,
                    enabled_mask: 0,
                };
                let socd = cx.new(|cx| SocdDocument::for_test(SocdPairs::default(), status, cx));
                document = Some(socd.clone());
                let page = cx.new(|cx| SocdPage::new(socd, cx));
                page.update(cx, |page, cx| page.open(window, cx));
                page
            })
            .unwrap()
        });
        (window, page, document.unwrap())
    }

    fn pair(document: &Entity<SocdDocument>, index: usize, cx: &App) -> SocdPair {
        document
            .read(cx)
            .settings()
            .loaded()
            .unwrap()
            .state()
            .pair(index)
    }

    #[gpui_kit::test]
    fn pairs_are_added_changed_and_undone(cx: &mut TestAppContext) {
        let (window, page, document) = open(cx);
        let undo = if cx.update(|cx| cx.platform_info().is_mac()) {
            "cmd-z"
        } else {
            "ctrl-z"
        };
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("socd-add", cx);
            let added = pair(&document, 0, cx);
            assert_eq!((added.a, added.b), (usage("a"), usage("d")));
            window.render_frame(cx);

            page.update(cx, |page, cx| {
                page.set_key(0, Side::A, usage("w"), cx);
                page.set_key(0, Side::B, usage("s"), cx);
            });
            let wasd = pair(&document, 0, cx);
            assert_eq!((wasd.a, wasd.b), (usage("w"), usage("s")));

            window.press(undo, cx);
            assert_eq!(pair(&document, 0, cx).b, usage("d"));
            window.render_frame(cx);
            assert!(Kbd::binding_for_action(&Undo, Some(CONTEXT), window).is_some());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn the_switch_turns_on_the_pairs_that_follow_it(cx: &mut TestAppContext) {
        let (window, page, document) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("socd-add", cx);
            page.update(cx, |page, cx| {
                page.edit_pair(
                    0,
                    |pair| SocdPair {
                        mode: SocdMode::Global,
                        ..pair
                    },
                    cx,
                )
            });
            let settings = |cx: &App| *document.read(cx).settings().loaded().unwrap().state();
            assert!(!settings(cx).is_active(0));
            window.render_frame(cx);
            window.click("socd-switch", cx);
            assert!(settings(cx).switch());
            assert!(settings(cx).is_active(0));
        })
        .unwrap();
    }

    #[test]
    fn a_pair_row_fits_the_width_of_every_page() {
        let columns = [
            NUMBER_WIDTH,
            KEY_WIDTH,
            KEY_WIDTH,
            BEHAVIOR_WIDTH,
            MODE_WIDTH,
            STATE_WIDTH,
            REMOVE_WIDTH,
        ];
        let row = columns.iter().map(|w| w.0).sum::<f32>() + COLUMN_GAP.0 * 6.;
        assert!(row <= keyboard::WIDTH.0, "{row} rem");
    }

    #[gpui_kit::test]
    fn removing_a_pair_moves_focus_to_the_next_one(cx: &mut TestAppContext) {
        let (window, page, document) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            for _ in 0..3 {
                window.click("socd-add", cx);
            }
            window.render_frame(cx);
            page.update(cx, |page, cx| page.remove_pair(1, window, cx));
            window.render_frame(cx);
            assert!(page.read(cx).remove_focus[2].is_focused(window));
            // After the last pair, the previous one.
            page.update(cx, |page, cx| page.remove_pair(2, window, cx));
            window.render_frame(cx);
            assert!(page.read(cx).remove_focus[0].is_focused(window));
            page.update(cx, |page, cx| page.remove_pair(0, window, cx));
            window.render_frame(cx);
            assert!(page.read(cx).add_focus.is_focused(window));
            assert_eq!(
                document
                    .read(cx)
                    .settings()
                    .loaded()
                    .unwrap()
                    .state()
                    .used()
                    .count(),
                0
            );
        })
        .unwrap();
    }
}
