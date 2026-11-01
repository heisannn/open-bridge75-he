//! The main window: title bar, navigation and the selected page. Holds no feature logic.

mod nav;

use gpui_kit::component::notification::Notification;
use gpui_kit::component::scroll::ScrollableElement as _;
use gpui_kit::component::sidebar::{Sidebar, SidebarCollapsible, SidebarMenu, SidebarMenuItem};
use gpui_kit::component::tag::Tag;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, Theme, TitleBar, WindowExt as _, h_flex, v_flex,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, FontWeight, Hsla,
    InteractiveElement as _, IntoElement, KeyBinding, ParentElement as _, Pixels, Rems, Render,
    SharedString, Styled as _, Subscription, Window, actions, div, px, rems,
};
use rust_i18n::t;

use crate::document::{DocumentEvent, KeyboardDocument};
use crate::keyboard;
use crate::monitor::MonitorPage;
use crate::overview::OverviewPage;
use crate::page;
use crate::preferences::{Appearance, Preferences, PreferencesPage};
use crate::session::{SearchState, Session, SessionState};
use crate::trigger::TriggerPage;
pub use nav::Page;

actions!(
    shell,
    [ShowOverview, ShowTrigger, ShowMonitor, ShowPreferences]
);

const CONTEXT: &str = "Shell";

/// Width of the expanded sidebar.
const SIDEBAR_WIDTH: Rems = rems(14.);
/// Smallest window width. The board fits beside the sidebar collapsed to icons; the test below
/// keeps the two in step.
pub const MIN_WINDOW_WIDTH: Pixels = px(960.);

/// Width a page needs to show the whole board.
fn page_width() -> Rems {
    rems(keyboard::WIDTH.0 + 2. * page::INSET.0)
}

/// Binds the navigation shortcuts.
pub fn init(cx: &mut App) {
    let bindings = if cx.platform_info().is_mac() {
        [
            KeyBinding::new("cmd-1", ShowOverview, Some(CONTEXT)),
            KeyBinding::new("cmd-2", ShowTrigger, Some(CONTEXT)),
            KeyBinding::new("cmd-3", ShowMonitor, Some(CONTEXT)),
            KeyBinding::new("cmd-4", ShowPreferences, Some(CONTEXT)),
        ]
    } else {
        [
            KeyBinding::new("ctrl-1", ShowOverview, Some(CONTEXT)),
            KeyBinding::new("ctrl-2", ShowTrigger, Some(CONTEXT)),
            KeyBinding::new("ctrl-3", ShowMonitor, Some(CONTEXT)),
            KeyBinding::new("ctrl-4", ShowPreferences, Some(CONTEXT)),
        ]
    };
    cx.bind_keys(bindings);
}

/// Repeated failures of one kind share one notification.
struct LoadFailed;
struct WriteFailed;

pub struct Shell {
    session: Entity<Session>,
    page: Page,
    trigger: Entity<TriggerPage>,
    monitor: Entity<MonitorPage>,
    /// Keeps the navigation shortcuts working when nothing inside a page has focus.
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl Shell {
    pub fn new(
        session: Entity<Session>,
        document: Entity<KeyboardDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let trigger = cx.new(|cx| TriggerPage::new(document.clone(), window, cx));
        let monitor = cx.new(|cx| MonitorPage::new(session.clone(), cx));
        let subscriptions = vec![
            cx.observe(&session, |_, _, cx| cx.notify()),
            cx.subscribe_in(&document, window, |_, _, event, window, cx| {
                Self::notify_failure(event, window, cx)
            }),
            cx.observe_window_appearance(window, |_, window, cx| {
                if Preferences::global(cx).appearance == Appearance::System {
                    Theme::sync_system_appearance(Some(window), cx);
                }
            }),
        ];
        let focus_handle = cx.focus_handle();
        focus_handle.focus(window, cx);
        Shell {
            session,
            page: Page::Overview,
            trigger,
            monitor,
            focus_handle,
            _subscriptions: subscriptions,
        }
    }

    fn notify_failure(event: &DocumentEvent, window: &mut Window, cx: &mut App) {
        let (notification, detail) = match event {
            DocumentEvent::LoadFailed(detail) => (
                Notification::error(t!("error.load_failed.body"))
                    .id::<LoadFailed>()
                    .title(t!("error.load_failed")),
                detail.clone(),
            ),
            DocumentEvent::WriteFailed(detail) => (
                Notification::error(t!("error.write_failed.body"))
                    .id::<WriteFailed>()
                    .title(t!("error.write_failed")),
                detail.clone(),
            ),
        };
        let notification = notification.content(move |_, _, cx| {
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(detail.clone())
                .into_any_element()
        });
        window.push_notification(notification, cx);
    }

    fn open(&mut self, page: Page, window: &mut Window, cx: &mut Context<Self>) {
        if self.page == page {
            return;
        }
        self.page = page;
        self.monitor.update(cx, |monitor, cx| {
            monitor.set_active(page == Page::Monitor, cx)
        });
        // The previous page's focus target is gone; keep focus where its shortcuts still work.
        match page {
            Page::Trigger => self
                .trigger
                .update(cx, |trigger, cx| trigger.focus(window, cx)),
            _ => self.focus_handle.focus(window, cx),
        }
        cx.notify();
    }

    fn render_status(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        // The tag stays neutral so its text is readable; a colored icon marks a problem.
        let (icon, label): (Option<(IconName, Hsla)>, SharedString) =
            match self.session.read(cx).state() {
                SessionState::Connected { .. } => (None, t!("status.connected").into()),
                SessionState::Searching(SearchState::NotFound) => {
                    (None, t!("status.searching").into())
                }
                SessionState::Searching(SearchState::Bootloader) => (
                    Some((IconName::TriangleAlert, theme.warning)),
                    t!("status.bootloader").into(),
                ),
                SessionState::Searching(SearchState::Failed(_) | SearchState::Rejected(_))
                | SessionState::Stopped => (
                    Some((IconName::CircleX, theme.danger)),
                    t!("status.unavailable").into(),
                ),
            };
        Tag::secondary().child(
            h_flex()
                .gap_1()
                .when_some(icon, |this, (icon, color)| {
                    this.child(Icon::new(icon).xsmall().text_color(color))
                })
                .child(label),
        )
    }

    fn render_sidebar(&self, collapsed: bool, cx: &mut Context<Self>) -> impl IntoElement {
        Sidebar::new("navigation")
            .collapsible(SidebarCollapsible::Icon)
            .collapsed(collapsed)
            .w(SIDEBAR_WIDTH)
            .child(SidebarMenu::new().children(Page::ALL.map(|page| {
                SidebarMenuItem::new(page.label())
                    .icon(page.icon())
                    .active(page == self.page)
                    .on_click(cx.listener(move |this, _, window, cx| this.open(page, window, cx)))
            })))
    }

    fn render_page(&self, cx: &mut Context<Self>) -> AnyElement {
        let page = match self.page {
            Page::Overview => {
                OverviewPage::new(self.session.read(cx).state().clone()).into_any_element()
            }
            Page::Trigger => self.trigger.clone().into_any_element(),
            Page::Monitor => self.monitor.clone().into_any_element(),
            Page::Preferences => PreferencesPage.into_any_element(),
        };
        // Each page has its own scroll position.
        let container = v_flex()
            .id(("page", self.page.ix()))
            .flex_1()
            .min_w_0()
            .min_h_0();
        if self.page.scrolls_itself() {
            container.child(page).into_any_element()
        } else {
            container
                .overflow_y_scrollbar()
                .child(page)
                .into_any_element()
        }
    }
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Collapse the sidebar to icons before the board would be cut off.
        let expanded_width = rems(SIDEBAR_WIDTH.0 + page_width().0).to_pixels(window.rem_size());
        let narrow = window.viewport_size().width < expanded_width;

        v_flex()
            .id("shell")
            .size_full()
            .track_focus(&self.focus_handle)
            .key_context(CONTEXT)
            .on_action(cx.listener(|this, _: &ShowOverview, window, cx| {
                this.open(Page::Overview, window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &ShowTrigger, window, cx| {
                    this.open(Page::Trigger, window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &ShowMonitor, window, cx| {
                    this.open(Page::Monitor, window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &ShowPreferences, window, cx| {
                this.open(Page::Preferences, window, cx)
            }))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                TitleBar::new().child(
                    h_flex()
                        .gap_3()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(t!("app.title")),
                        )
                        .child(self.render_status(cx)),
                ),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .items_stretch()
                    .child(self.render_sidebar(narrow, cx))
                    .child(self.render_page(cx)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Width of the sidebar collapsed to icons, as GPUI Component draws it.
    const COLLAPSED_SIDEBAR_WIDTH: Pixels = px(48.);

    #[test]
    fn the_board_fits_the_smallest_window() {
        // Default theme font size, which is the rem.
        let rem = px(16.);
        let needed = COLLAPSED_SIDEBAR_WIDTH + page_width().to_pixels(rem);
        assert!(needed <= MIN_WINDOW_WIDTH, "{needed:?}");
    }
}

#[cfg(test)]
mod shortcut_tests {
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{Bounds, TestAppContext, WindowBounds, WindowOptions, size};

    use super::*;

    #[gpui_kit::test]
    fn pages_open_with_shortcuts(cx: &mut TestAppContext) {
        let (window, shell) = cx.update(|cx| {
            gpui_kit::init(cx);
            init(cx);
            crate::trigger::init(cx);
            cx.set_global(Preferences::default());
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(1180.), px(800.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                let document = cx.new(|cx| KeyboardDocument::for_test(Vec::new(), cx));
                let session = cx.new(|_| Session::detached());
                cx.new(|cx| Shell::new(session, document, window, cx))
            })
            .unwrap()
        });
        let modifier = if cx.update(|cx| cx.platform_info().is_mac()) {
            "cmd"
        } else {
            "ctrl"
        };
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            for (key, page) in [
                ("2", Page::Trigger),
                ("3", Page::Monitor),
                ("4", Page::Preferences),
                ("1", Page::Overview),
            ] {
                window.press(&format!("{modifier}-{key}"), cx);
                assert_eq!(shell.read(cx).page, page);
            }
        })
        .unwrap();
    }
}
