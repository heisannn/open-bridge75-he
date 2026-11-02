//! What each key sends, on the base layer and on the Fn layer.
//!
//! The board is one Tab stop. While it has focus, the arrow keys move the selected key, Enter
//! opens its assignment menu and Delete restores its default. Clicking a key selects it and
//! opens the menu beside it.

pub(crate) mod assignment;
mod document;
mod layers;
mod store;

use bridge75_core::hid::Usage;
use bridge75_core::keymap::{KeyAction, Layer};
use bridge75_core::matrix::{Key, KeyPos, LAYOUT};
use gpui_kit::assets::IconName as AppIconName;
use gpui_kit::base::ElementExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::tab::{Tab, TabBar};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    ThemeStyled as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Bounds, Context, DismissEvent, Entity, FocusHandle,
    Focusable as _, FontWeight, Hsla, InteractiveElement as _, IntoElement, KeyBinding,
    ParentElement as _, Pixels, Render, Role, SharedString, StatefulInteractiveElement as _,
    Styled as _, Subscription, Window, actions, deferred, div, px, rems,
};
use rust_i18n::t;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::document::Loadable;
use crate::keyboard::{self, Direction, KeyboardView};
use crate::page::{self, Field, PageHeader};
use crate::session::Session;
use assignment::{
    COMBO_MODIFIERS, Category, MAX_COMBO_MODIFIERS, accepts_modifiers, label, modifier_name,
    short_label, split_modifiers, with_modifiers,
};
pub use document::KeymapDocument;
use layers::default_action;
pub use store::KeymapStore;

actions!(
    keymap,
    [
        /// Undo the newest change of the keymap.
        Undo,
        ClearSelection,
        FocusLeft,
        FocusRight,
        FocusUp,
        FocusDown,
        /// Open the assignment menu of the selected key.
        ChooseAssignment,
        /// Give the selected key its default assignment.
        RestoreDefault,
    ]
);

/// Key context of the whole page.
const CONTEXT: &str = "KeymapPage";
/// Key context of the board while it has focus.
const BOARD_CONTEXT: &str = "KeymapBoard";

/// The layers in the order of the layer switch.
const LAYERS: [Layer; 2] = [Layer::Base, Layer::Fn];

/// Binds the page's keyboard shortcuts.
pub fn init(cx: &mut App) {
    let mac = cx.platform_info().is_mac();
    cx.bind_keys([
        KeyBinding::new(if mac { "cmd-z" } else { "ctrl-z" }, Undo, Some(CONTEXT)),
        KeyBinding::new("escape", ClearSelection, Some(CONTEXT)),
        KeyBinding::new("left", FocusLeft, Some(BOARD_CONTEXT)),
        KeyBinding::new("right", FocusRight, Some(BOARD_CONTEXT)),
        KeyBinding::new("up", FocusUp, Some(BOARD_CONTEXT)),
        KeyBinding::new("down", FocusDown, Some(BOARD_CONTEXT)),
        KeyBinding::new("enter", ChooseAssignment, Some(BOARD_CONTEXT)),
        KeyBinding::new("delete", RestoreDefault, Some(BOARD_CONTEXT)),
        KeyBinding::new("backspace", RestoreDefault, Some(BOARD_CONTEXT)),
    ]);
}

/// Creates the keymap document for `session`.
pub fn new(session: Entity<Session>, cx: &mut App) -> Entity<KeymapDocument> {
    cx.new(|cx| KeymapDocument::new(session, KeymapStore::new(), cx))
}

fn layer_label(layer: Layer) -> SharedString {
    match layer {
        Layer::Base => t!("keymap.layer.base"),
        Layer::Fn => t!("keymap.layer.fn"),
    }
    .into()
}

/// The menu entry of `action`: the key, without the modifiers held with it.
fn menu_value(action: KeyAction) -> KeyAction {
    match split_modifiers(action) {
        Some((key, _)) => KeyAction::Key(key),
        None => action,
    }
}

/// The assignment menu, open beside the key it changes.
struct AssignmentMenu {
    layer: Layer,
    pos: KeyPos,
    view: Entity<PopupMenu>,
    _dismiss: Subscription,
}

pub struct KeymapPage {
    document: Entity<KeymapDocument>,
    layer: Layer,
    selected: Option<KeyPos>,
    menu: Option<AssignmentMenu>,
    /// Where each key was drawn in the last frame, so the menu can open beside its key.
    key_bounds: Rc<RefCell<BTreeMap<KeyPos, Bounds<Pixels>>>>,
    /// Keeps keyboard focus on the page when its background is clicked.
    focus_handle: FocusHandle,
    board_focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl KeymapPage {
    pub fn new(
        document: Entity<KeymapDocument>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        // A menu for a key that can't be changed any more (the keyboard was unplugged) closes.
        let subscriptions = vec![cx.observe(&document, |this, _, cx| {
            if this.menu.is_some() && this.editable(cx).is_none() {
                this.close_menu(cx);
            }
        })];
        KeymapPage {
            document,
            layer: Layer::Base,
            selected: None,
            menu: None,
            key_bounds: Rc::default(),
            focus_handle: cx.focus_handle(),
            board_focus: cx.focus_handle().tab_stop(true),
            _subscriptions: subscriptions,
        }
    }

    /// Moves keyboard focus into the page. The shell calls this when the page is opened.
    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        self.focus_handle.focus(window, cx);
    }

    /// The selected key and what it sends on the shown layer, if it can be changed.
    fn editable(&self, cx: &App) -> Option<(KeyPos, KeyAction)> {
        let keymap = self.document.read(cx).keymap().loaded()?;
        let pos = self.selected?;
        (!keymap.is_fixed(self.layer, pos)).then(|| (pos, keymap.get(self.layer, pos)))
    }

    fn select_key(&mut self, pos: KeyPos, cx: &mut Context<Self>) {
        if self.selected != Some(pos) {
            self.close_menu(cx);
        }
        self.selected = Some(pos);
        cx.notify();
    }

    fn set_layer(&mut self, layer: Layer, cx: &mut Context<Self>) {
        self.layer = layer;
        self.close_menu(cx);
        cx.notify();
    }

    fn close_menu(&mut self, cx: &mut Context<Self>) {
        if self.menu.take().is_some() {
            cx.notify();
        }
    }

    /// Opens the assignment menu beside the selected key, with focus in it. Focus returns to
    /// the board when the menu closes.
    fn open_menu(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some((pos, current)) = self.editable(cx) else {
            return;
        };
        let is_default = self
            .document
            .read(cx)
            .keymap()
            .loaded()
            .is_some_and(|k| k.is_default(self.layer, pos));
        let page = cx.entity();
        let shown = menu_value(current);
        let rem = window.rem_size();
        let board_focus = self.board_focus.clone();
        let view = PopupMenu::build(window, cx, move |menu, window, cx| {
            let item = |action: KeyAction, window: &mut Window| {
                PopupMenuItem::new(label(action))
                    .checked(action == shown)
                    .on_click(
                        window.listener_for(&page, move |this: &mut Self, _, _, cx| {
                            this.choose(action, cx)
                        }),
                    )
            };
            let restore =
                PopupMenuItem::new(t!("keymap.default_is", action = label(default_action(pos))))
                    .checked(is_default)
                    .on_click(
                        window.listener_for(&page, |this: &mut Self, _, window, cx| {
                            this.restore_default(&RestoreDefault, window, cx)
                        }),
                    );
            let mut menu = menu
                .action_context(board_focus.clone())
                .min_w(rems(12.).to_pixels(rem))
                .item(restore)
                .separator();
            for category in Category::ALL {
                if category == Category::General {
                    continue;
                }
                let page = page.clone();
                menu =
                    menu.submenu(category.title(), window, cx, move |submenu, window, _| {
                        // Long sections scroll rather than run off the window.
                        let submenu = submenu.scrollable(true).max_h(rems(24.).to_pixels(rem));
                        category
                            .actions()
                            .into_iter()
                            .fold(submenu, |submenu, action| {
                                submenu.item(
                                    PopupMenuItem::new(label(action))
                                        .checked(action == shown)
                                        .on_click(window.listener_for(
                                            &page,
                                            move |this: &mut Self, _, _, cx| {
                                                this.choose(action, cx)
                                            },
                                        )),
                                )
                            })
                    });
            }
            Category::General
                .actions()
                .into_iter()
                .fold(menu.separator(), |menu, action| {
                    menu.item(item(action, window))
                })
        });
        let dismiss = cx.subscribe(&view, |this, _, _: &DismissEvent, cx| this.close_menu(cx));
        view.focus_handle(cx).focus(window, cx);
        self.menu = Some(AssignmentMenu {
            layer: self.layer,
            pos,
            view,
            _dismiss: dismiss,
        });
        cx.notify();
    }

    fn assign(&mut self, pos: KeyPos, action: KeyAction, cx: &mut Context<Self>) {
        let layer = self.layer;
        self.document
            .update(cx, |document, cx| document.assign(layer, pos, action, cx));
    }

    /// Assigns what was picked. A key keeps the modifiers held with the key it replaces.
    fn choose(&mut self, action: KeyAction, cx: &mut Context<Self>) {
        let Some((pos, current)) = self.editable(cx) else {
            return;
        };
        let next = match (action, split_modifiers(current)) {
            (KeyAction::Key(key), Some((_, modifiers))) => with_modifiers(key, &modifiers),
            _ => action,
        };
        self.assign(pos, next, cx);
    }

    fn set_modifier(&mut self, modifier: Usage, held: bool, cx: &mut Context<Self>) {
        let Some((pos, current)) = self.editable(cx) else {
            return;
        };
        let Some((key, mut modifiers)) = split_modifiers(current) else {
            return;
        };
        modifiers.retain(|m| *m != modifier);
        if held {
            modifiers.push(modifier);
        }
        self.assign(pos, with_modifiers(key, &modifiers), cx);
    }

    fn restore_default(&mut self, _: &RestoreDefault, _: &mut Window, cx: &mut Context<Self>) {
        if let Some((pos, _)) = self.editable(cx) {
            self.assign(pos, default_action(pos), cx);
        }
    }

    fn undo(&mut self, _: &Undo, _: &mut Window, cx: &mut Context<Self>) {
        let undone = self.document.update(cx, |document, cx| document.undo(cx));
        // Show the key that changed back.
        if let Some((layer, pos)) = undone {
            if self.layer != layer {
                self.set_layer(layer, cx);
            }
            self.select_key(pos, cx);
        }
    }

    fn clear_selection(&mut self, _: &ClearSelection, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected.is_none() {
            cx.propagate();
            return;
        }
        self.selected = None;
        self.close_menu(cx);
        // The editor goes away with the selection; keep focus where the board's keys work.
        if self.is_loaded(cx) {
            self.board_focus.focus(window, cx);
        }
        cx.notify();
    }

    fn move_selection(&mut self, direction: Direction, cx: &mut Context<Self>) {
        let Some(from) = self.selected else {
            self.select_key(LAYOUT[0].pos, cx);
            return;
        };
        if let Some(next) = keyboard::neighbor(from, direction) {
            self.select_key(next, cx);
        }
    }

    fn choose_assignment(
        &mut self,
        _: &ChooseAssignment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_menu(window, cx);
    }

    fn is_loaded(&self, cx: &App) -> bool {
        self.document.read(cx).keymap().loaded().is_some()
    }

    fn render_key(
        &self,
        key: &'static Key,
        focused: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let pos = key.pos;
        let keymap = self.document.read(cx).keymap().loaded();
        let action = keymap.map(|k| k.get(self.layer, pos));
        let changed = keymap.is_some_and(|k| !k.is_default(self.layer, pos));
        let fixed = keymap.is_some_and(|k| k.is_fixed(self.layer, pos));
        let selected = self.selected == Some(pos);
        let theme = cx.theme();
        let (muted, primary) = (theme.muted_foreground, theme.primary);
        let description: SharedString = match action {
            _ if fixed => format!("{}, {}", key.label, t!("keymap.fixed")).into(),
            Some(action) => format!("{}, {}", key.label, label(action)).into(),
            None => key.label.into(),
        };
        // A keycap fits only a short name; the tooltip gives the full one.
        let tooltip = action
            .filter(|_| changed && !fixed)
            .map(label)
            .filter(|full| action.is_some_and(|a| *full != short_label(a)));
        let menu = self
            .menu
            .as_ref()
            .filter(|menu| menu.pos == pos && menu.layer == self.layer)
            .map(|menu| menu.view.clone());
        let keycap = Button::new(key.id)
            .outline()
            .size_full()
            .p_0()
            .tab_stop(false)
            .accessibility_label(description)
            .selected(selected)
            .disabled(keymap.is_none())
            .when(selected, |this| this.border_color(primary))
            .when(focused && selected, |this| {
                this.focus_ring_style(window, cx)
            })
            .when_some(tooltip, |this, tooltip| this.tooltip(tooltip))
            .on_click(cx.listener(move |this, _, window, cx| {
                // Keycaps are not focusable, so the arrow keys continue from the clicked key.
                this.board_focus.focus(window, cx);
                this.select_key(pos, cx);
                this.open_menu(window, cx);
            }))
            .child(
                v_flex()
                    .size_full()
                    .min_w_0()
                    .px_1()
                    .items_center()
                    .justify_center()
                    .gap_0p5()
                    .child(div().text_xs().text_color(muted).child(key.label))
                    .when(fixed, |this| {
                        this.child(Icon::new(AppIconName::Lock).xsmall().text_color(muted))
                    })
                    .when_some(action.filter(|_| changed && !fixed), |this, action| {
                        this.child(
                            div()
                                .w_full()
                                .text_center()
                                .truncate()
                                .text_xs()
                                .font_weight(FontWeight::MEDIUM)
                                .child(short_label(action)),
                        )
                    }),
            );
        // The menu opens beside the key, its first entry level with it, and on the left when
        // there is no room on the right. Below the key, a menu taller than the space around the
        // key would be pushed back over it. It is placed by where the key was drawn in the last
        // frame, which a key has before it can be chosen.
        let anchor = menu.and_then(|menu| Some((menu, *self.key_bounds.borrow().get(&pos)?)));
        let key_bounds = self.key_bounds.clone();
        div()
            .relative()
            .size_full()
            .on_prepaint(move |bounds, _, _| {
                key_bounds.borrow_mut().insert(pos, bounds);
            })
            .child(keycap)
            .when_some(anchor, |this, (menu, bounds)| {
                this.child(
                    deferred(
                        gpui_kit::base::Positioner::side(bounds)
                            .placement(gpui_kit::base::Placement::Right)
                            .align(gpui_kit::base::Align::Start)
                            .offset(px(4.))
                            .margin(rems(0.5).to_pixels(window.rem_size()))
                            .occlude()
                            .child(menu),
                    )
                    .with_priority(gpui_kit::base::POPUP_PRIORITY),
                )
            })
            .into_any_element()
    }

    fn render_board(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let loaded = self.is_loaded(cx);
        let focused = self.board_focus.is_focused(window) && window.last_input_was_keyboard();
        let keyboard = KeyboardView::new("keymap-keyboard", |key| {
            self.render_key(key, focused, window, cx)
        });
        div()
            .id("keymap-board")
            .flex_none()
            .role(Role::Group)
            .aria_label(SharedString::from(t!("keymap.board")))
            .when(loaded, |this| {
                this.track_focus(&self.board_focus)
                    .key_context(BOARD_CONTEXT)
                    .on_action(cx.listener(|this, _: &FocusLeft, _, cx| {
                        this.move_selection(Direction::Left, cx)
                    }))
                    .on_action(cx.listener(|this, _: &FocusRight, _, cx| {
                        this.move_selection(Direction::Right, cx)
                    }))
                    .on_action(cx.listener(|this, _: &FocusUp, _, cx| {
                        this.move_selection(Direction::Up, cx)
                    }))
                    .on_action(cx.listener(|this, _: &FocusDown, _, cx| {
                        this.move_selection(Direction::Down, cx)
                    }))
                    .on_action(cx.listener(Self::choose_assignment))
                    .on_action(cx.listener(Self::restore_default))
            })
            .child(keyboard)
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let layer_ix = LAYERS.iter().position(|l| *l == self.layer).unwrap_or(0);
        let has_undo = self.document.read(cx).has_undo_history();
        h_flex()
            .w(keyboard::WIDTH)
            .gap_2()
            .child(
                TabBar::new("keymap-layer")
                    .segmented()
                    .small()
                    .selected_index(layer_ix)
                    .on_click(
                        cx.listener(|this, ix: &usize, _, cx| this.set_layer(LAYERS[*ix], cx)),
                    )
                    .children(LAYERS.map(|layer| Tab::new().label(layer_label(layer)))),
            )
            .child(div().flex_1())
            .child(
                Button::new("keymap-undo")
                    .small()
                    .icon(IconName::Undo2)
                    .label(t!("keymap.undo"))
                    .tooltip_with_action(t!("keymap.undo"), &Undo, Some(CONTEXT))
                    .disabled(!has_undo)
                    .on_click(cx.listener(|this, _, window, cx| this.undo(&Undo, window, cx))),
            )
    }

    /// Explains the lock on the keys the keyboard fixes, which only the Fn layer has.
    fn render_legend(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        (self.layer == Layer::Fn).then(|| {
            h_flex()
                .w(keyboard::WIDTH)
                .gap_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(Icon::new(AppIconName::Lock).xsmall())
                .child(t!("keymap.fixed"))
        })
    }

    /// A message about the whole keymap, with what can be done about it.
    fn render_notice(
        icon: Option<(IconName, Hsla)>,
        message: SharedString,
        detail: Option<SharedString>,
        action: Option<Button>,
        cx: &App,
    ) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        v_flex()
            .w(keyboard::WIDTH)
            .gap_2()
            .items_start()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        h_flex()
                            .gap_2()
                            .text_sm()
                            .when_some(icon, |this, (icon, color)| {
                                this.child(Icon::new(icon).small().text_color(color))
                            })
                            .child(message),
                    )
                    .when_some(detail, |this, detail| {
                        this.child(div().text_xs().text_color(muted).child(detail))
                    }),
            )
            .children(action)
            .into_any_element()
    }

    fn render_notices(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let document = self.document.read(cx);
        let Some(keymap) = document.keymap().loaded() else {
            return Vec::new();
        };
        let theme = cx.theme();
        let (danger, warning) = (theme.danger, theme.warning);
        let mut notices = Vec::new();
        if let Some(error) = document.write_error() {
            let retry = Button::new("keymap-write-again")
                .small()
                .label(t!("keymap.write_again"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.document
                        .update(cx, |document, cx| document.retry_write(cx))
                }));
            notices.push(Self::render_notice(
                Some((IconName::CircleX, danger)),
                t!("keymap.write_failed").into(),
                Some(error.clone()),
                Some(retry),
                cx,
            ));
        }
        if let Some(error) = document.save_error() {
            notices.push(Self::render_notice(
                Some((IconName::TriangleAlert, warning)),
                t!("keymap.save_failed").into(),
                Some(error.clone()),
                None,
                cx,
            ));
        }
        if !keymap.has_fn_key() {
            notices.push(Self::render_notice(
                Some((IconName::TriangleAlert, warning)),
                t!("keymap.no_fn_key").into(),
                None,
                None,
                cx,
            ));
        }
        if !keymap.is_stored() && !keymap.is_changed() {
            notices.push(
                div()
                    .w(keyboard::WIDTH)
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(t!("keymap.not_written"))
                    .into_any_element(),
            );
        }
        notices
    }

    fn render_modifiers(&self, key: Usage, held: &[Usage], cx: &mut Context<Self>) -> AnyElement {
        let full = held.len() >= MAX_COMBO_MODIFIERS;
        let boxes = COMBO_MODIFIERS.iter().enumerate().map(|(ix, modifier)| {
            let modifier = *modifier;
            let checked = held.contains(&modifier);
            Checkbox::new(("keymap-modifier", ix))
                .label(modifier_name(modifier))
                .checked(checked)
                .disabled(!checked && full)
                .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                    this.set_modifier(modifier, *checked, cx)
                }))
        });
        let note = t!(
            "keymap.modifiers.note",
            key = short_label(KeyAction::Key(key))
        );
        Field::new(
            SharedString::from(t!("keymap.modifiers")),
            h_flex().gap_4().children(boxes),
        )
        .note(Some(note.into()))
        .into_any_element()
    }

    fn render_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.theme().muted_foreground;
        let message = |text: SharedString| {
            div()
                .text_sm()
                .text_color(muted)
                .child(text)
                .into_any_element()
        };
        let Some(pos) = self.selected else {
            return message(t!("keymap.no_selection").into());
        };
        let Some((_, action)) = self.editable(cx) else {
            return message(t!("keymap.fixed_key").into());
        };
        let legend = pos.key().map_or("", |key| key.label);
        let default = default_action(pos);
        let is_default = self
            .document
            .read(cx)
            .keymap()
            .loaded()
            .is_some_and(|k| k.is_default(self.layer, pos));
        let name = t!("keymap.assignment_of", key = legend);
        let change = Button::new("keymap-change")
            .label(t!("keymap.change"))
            .tooltip_with_action(t!("keymap.change"), &ChooseAssignment, Some(BOARD_CONTEXT))
            .on_click(cx.listener(|this, _, window, cx| this.open_menu(window, cx)));
        let restore =
            Button::new("keymap-restore-default")
                .label(t!("keymap.restore_default"))
                .tooltip_with_action(
                    t!("keymap.restore_default"),
                    &RestoreDefault,
                    Some(BOARD_CONTEXT),
                )
                .disabled(is_default)
                .on_click(cx.listener(|this, _, window, cx| {
                    this.restore_default(&RestoreDefault, window, cx)
                }));
        let note = (!is_default).then(|| t!("keymap.default_is", action = label(default)).into());
        let modifiers = split_modifiers(action)
            .filter(|(key, _)| accepts_modifiers(*key))
            .map(|(key, held)| self.render_modifiers(key, &held, cx));
        v_flex()
            .w(keyboard::WIDTH)
            .gap_4()
            .child(
                Field::new(
                    SharedString::from(name),
                    h_flex()
                        .gap_2()
                        .child(
                            div()
                                .min_w_0()
                                .mr_2()
                                .text_sm()
                                .font_weight(FontWeight::MEDIUM)
                                .child(label(action)),
                        )
                        .child(change)
                        .child(restore),
                )
                .note(note),
            )
            .children(modifiers)
            .into_any_element()
    }

    fn render_status(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let muted = cx.theme().muted_foreground;
        let message = |text: SharedString| div().text_sm().text_color(muted).child(text);
        match self.document.read(cx).keymap() {
            Loadable::Loaded(_) => None,
            Loadable::Idle => Some(message(t!("keyboard.not_connected").into()).into_any_element()),
            Loadable::Loading => Some(
                h_flex()
                    .gap_2()
                    .child(Spinner::new().small().color(muted))
                    .child(message(t!("keymap.loading").into()))
                    .into_any_element(),
            ),
            Loadable::Failed(reason) => Some(Self::render_notice(
                None,
                t!("keymap.load_failed").into(),
                Some(reason.clone()),
                Some(
                    Button::new("keymap-reload")
                        .small()
                        .label(t!("keymap.retry"))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.document.update(cx, |document, cx| document.reload(cx))
                        })),
                ),
                cx,
            )),
        }
    }
}

impl Render for KeymapPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = self.render_status(cx);
        let loaded = status.is_none();

        // The header and board stay in place; only the editor scrolls when the window is short.
        v_flex()
            .id("keymap-page")
            .flex_1()
            .min_h_0()
            .w_full()
            .track_focus(&self.focus_handle)
            .key_context(CONTEXT)
            .on_action(cx.listener(Self::undo))
            .on_action(cx.listener(Self::clear_selection))
            .child(
                v_flex()
                    .flex_none()
                    .p(page::INSET)
                    .gap_6()
                    .child(PageHeader::new(t!("keymap.title")))
                    .children(status)
                    .child(
                        v_flex()
                            .gap_2()
                            .when(loaded, |this| this.child(self.render_toolbar(cx)))
                            .child(self.render_board(window, cx))
                            .when(loaded, |this| this.children(self.render_legend(cx))),
                    )
                    .when(loaded, |this| this.children(self.render_notices(cx))),
            )
            .when(loaded, |this| {
                this.child(
                    div()
                        .id("keymap-editor")
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
    use bridge75_core::matrix::KeyBitmap;
    use gpui_kit::component::kbd::Kbd;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AnyWindowHandle, Bounds, TestAppContext, WindowBounds, WindowOptions, px, size,
    };

    use super::*;

    fn pos(id: &str) -> KeyPos {
        id.parse().unwrap()
    }

    fn key(name: &str) -> KeyAction {
        KeyAction::Key(name.parse().unwrap())
    }

    /// The page in a headless window, over a keymap that never reaches a real keyboard. F1 is
    /// fixed on the Fn layer.
    fn open(
        cx: &mut TestAppContext,
    ) -> (AnyWindowHandle, Entity<KeymapPage>, Entity<KeymapDocument>) {
        let mut document = None;
        let (window, page) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            cx.set_global(crate::preferences::Preferences::default());
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(1180.), px(800.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                let fixed: KeyBitmap = [pos("f1")].into_iter().collect();
                let keymap =
                    cx.new(|cx| KeymapDocument::for_test(None, fixed, KeymapStore::none(), cx));
                document = Some(keymap.clone());
                cx.new(|cx| KeymapPage::new(keymap, window, cx))
            })
            .unwrap()
        });
        (window, page, document.unwrap())
    }

    fn assigned(document: &Entity<KeymapDocument>, layer: Layer, id: &str, cx: &App) -> KeyAction {
        document
            .read(cx)
            .keymap()
            .loaded()
            .unwrap()
            .get(layer, pos(id))
    }

    #[gpui_kit::test]
    fn keys_are_changed_and_restored_from_the_keyboard(cx: &mut TestAppContext) {
        let (window, page, document) = open(cx);
        let undo = if cx.update(|cx| cx.platform_info().is_mac()) {
            "cmd-z"
        } else {
            "ctrl-z"
        };
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            page.update(cx, |page, cx| page.board_focus.focus(window, cx));
            window.render_frame(cx);

            // The first arrow selects Esc; then down into the number row.
            window.press("right", cx);
            assert_eq!(page.read(cx).selected, Some(pos("esc")));
            window.press("down", cx);
            assert_eq!(page.read(cx).selected, Some(pos("grave")));

            window.click("c", cx);
            page.update(cx, |page, cx| {
                page.choose(key("v"), cx);
                page.set_modifier(Usage(0xE0), true, cx);
            });
            let paste: KeyAction = "combo:lctrl+v".parse().unwrap();
            assert_eq!(assigned(&document, Layer::Base, "c", cx), paste);
            // Picking another key keeps the modifier.
            page.update(cx, |page, cx| page.choose(key("x"), cx));
            let cut: KeyAction = "combo:lctrl+x".parse().unwrap();
            assert_eq!(assigned(&document, Layer::Base, "c", cx), cut);

            // Escape closes the menu the click opened; the board has focus again.
            window.press("escape", cx);
            window.render_frame(cx);
            window.press("delete", cx);
            assert_eq!(assigned(&document, Layer::Base, "c", cx), key("c"));
            window.press(undo, cx);
            assert_eq!(assigned(&document, Layer::Base, "c", cx), cut);

            window.press("escape", cx);
            assert_eq!(page.read(cx).selected, None);
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn layers_are_edited_separately_and_fixed_keys_stay(cx: &mut TestAppContext) {
        let (window, page, document) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("h", cx);
            page.update(cx, |page, cx| {
                page.set_layer(Layer::Fn, cx);
                page.choose(key("left"), cx);
            });
            assert_eq!(assigned(&document, Layer::Fn, "h", cx), key("left"));
            assert_eq!(assigned(&document, Layer::Base, "h", cx), key("h"));
            window.render_frame(cx);
            assert_ne!(window.find("keymap-restore-default").disabled(), Some(true));

            // F1 does what the keyboard fixes while Fn is held, so it has no menu.
            window.click("f1", cx);
            assert!(page.read(cx).menu.is_none());
            page.update(cx, |page, cx| page.choose(key("a"), cx));
            assert_eq!(assigned(&document, Layer::Fn, "f1", cx), key("f1"));
            window.render_frame(cx);
            assert!(window.try_find("keymap-restore-default").is_none());
            // The undo, restore and board shortcuts can be shown in tooltips.
            for (action, context) in [
                (&Undo as &dyn gpui_kit::Action, CONTEXT),
                (&RestoreDefault, BOARD_CONTEXT),
                (&ChooseAssignment, BOARD_CONTEXT),
            ] {
                assert!(Kbd::binding_for_action(action, Some(context), window).is_some());
            }
        })
        .unwrap();
    }

    /// Runs `f` in the window, renders and lets deferred work finish.
    fn step(
        cx: &mut TestAppContext,
        window: AnyWindowHandle,
        f: impl FnOnce(&mut Window, &mut App),
    ) {
        cx.update_window(window, |_, window, cx| {
            f(window, cx);
            window.render_frame(cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    /// The key whose menu is open, and whether the menu has focus.
    fn open_menu(page: &Entity<KeymapPage>, window: &Window, cx: &App) -> Option<(KeyPos, bool)> {
        let menu = page.read(cx).menu.as_ref()?;
        let focused = menu.view.focus_handle(cx).contains_focused(window, cx);
        Some((menu.pos, focused))
    }

    #[gpui_kit::test]
    fn a_clicked_key_opens_its_menu_beside_it(cx: &mut TestAppContext) {
        let (window, page, document) = open(cx);
        step(cx, window, |window, cx| window.click("c", cx));
        step(cx, window, |window, cx| {
            assert_eq!(open_menu(&page, window, cx), Some((pos("c"), true)));
            // Up wraps to the last entry, None.
            window.press("up", cx);
        });
        step(cx, window, |window, cx| window.press("enter", cx));
        step(cx, window, |window, cx| {
            assert_eq!(assigned(&document, Layer::Base, "c", cx), KeyAction::None);
            assert_eq!(open_menu(&page, window, cx), None);
            assert!(page.read(cx).board_focus.is_focused(window));
            // Enter opens the menu of the selected key again.
            window.press("enter", cx);
        });
        step(cx, window, |window, cx| {
            assert_eq!(open_menu(&page, window, cx), Some((pos("c"), true)));
            window.press("escape", cx);
        });
        step(cx, window, |window, cx| {
            assert_eq!(open_menu(&page, window, cx), None);
            assert_eq!(page.read(cx).selected, Some(pos("c")));
            assert!(page.read(cx).board_focus.is_focused(window));
            // Moving to another key leaves its menu closed.
            window.press("right", cx);
            assert_eq!(page.read(cx).selected, Some(pos("v")));
            assert!(page.read(cx).menu.is_none());
        });
    }

    #[gpui_kit::test]
    fn escape_closes_the_menu_then_clears_the_selection(cx: &mut TestAppContext) {
        let (window, page, _) = open(cx);
        step(cx, window, |window, cx| window.click("w", cx));
        step(cx, window, |window, cx| window.press("escape", cx));
        step(cx, window, |window, cx| {
            assert!(page.read(cx).menu.is_none());
            assert_eq!(page.read(cx).selected, Some(pos("w")));
            window.press("escape", cx);
        });
        step(cx, window, |window, cx| {
            assert_eq!(page.read(cx).selected, None);
            assert!(page.read(cx).board_focus.is_focused(window));
            window.press("right", cx);
            assert_eq!(page.read(cx).selected, Some(pos("esc")));
        });
    }

    #[gpui_kit::test]
    fn undo_shows_the_key_it_changed_back(cx: &mut TestAppContext) {
        let (window, page, document) = open(cx);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("j", cx);
            page.update(cx, |page, cx| {
                page.set_layer(Layer::Fn, cx);
                page.choose(key("down"), cx);
                page.set_layer(Layer::Base, cx);
                page.select_key(pos("esc"), cx);
                page.undo(&Undo, window, cx);
            });
            let page = page.read(cx);
            assert_eq!((page.layer, page.selected), (Layer::Fn, Some(pos("j"))));
            assert_eq!(assigned(&document, Layer::Fn, "j", cx), key("j"));
        })
        .unwrap();
    }
}
