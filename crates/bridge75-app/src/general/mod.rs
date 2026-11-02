//! Settings that apply to the whole keyboard: debounce, polling rate and sleep time.
//!
//! The keyboard can report all of them, so they are read when it connects. The section is
//! shown on the keyboard page, under what the keyboard reports about itself.

mod document;
mod settings;

use bridge75_core::he::PollingRate;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::radio::RadioGroup;
use gpui_kit::component::select::{Select, SelectEvent, SelectItem, SelectState};
use gpui_kit::component::spinner::Spinner;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, IconName, Sizable as _, h_flex, v_flex,
};
use gpui_kit::{
    AnyElement, AppContext as _, Context, Entity, IntoElement, ParentElement as _, Rems, Render,
    SharedString, Styled as _, Subscription, Window, div, rems,
};
use rust_i18n::t;

use crate::document::Loadable;
use crate::page::Field;
use crate::preferences::Preferences;
pub use document::{GeneralDocument, new};
pub use settings::GeneralSettings;
use settings::{Change, sleep_choices};

/// The polling rates in the order of their buttons.
const POLLING_RATES: [PollingRate; 4] = [
    PollingRate::Khz1,
    PollingRate::Khz2,
    PollingRate::Khz4,
    PollingRate::Khz8,
];

fn polling_rate_label(rate: PollingRate) -> SharedString {
    t!("unit.khz", value = rate.khz()).into()
}

fn sleep_label(seconds: u16) -> SharedString {
    if seconds.is_multiple_of(60) {
        t!("general.sleep.minutes", minutes = seconds / 60)
    } else {
        t!("general.sleep.seconds", seconds = seconds)
    }
    .into()
}

/// A sleep time in the menu.
#[derive(Clone)]
struct SleepChoice {
    seconds: u16,
    title: SharedString,
}

impl SelectItem for SleepChoice {
    type Value = u16;

    fn title(&self) -> SharedString {
        self.title.clone()
    }

    fn value(&self) -> &u16 {
        &self.seconds
    }
}

type SleepSelect = SelectState<Vec<SleepChoice>>;

/// Width of the sleep menu and of the button that opens it, so the menu lines up under it.
const SLEEP_WIDTH: Rems = rems(12.);

pub struct GeneralSection {
    document: Entity<GeneralDocument>,
    sleep: Entity<SleepSelect>,
    /// The sleep time the menu was last built for, so it is rebuilt only when that changes.
    sleep_shown: Option<u16>,
    _subscriptions: Vec<Subscription>,
}

impl GeneralSection {
    pub fn new(
        document: Entity<GeneralDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let sleep = cx.new(|cx| SleepSelect::new(Vec::new(), None, window, cx));
        let subscriptions = vec![
            cx.observe_in(&document, window, |this, _, window, cx| {
                this.sync_sleep(false, window, cx);
                cx.notify();
            }),
            // The menu holds its texts, so they follow the language chosen in the preferences.
            cx.observe_global_in::<Preferences>(window, |this, window, cx| {
                this.sync_sleep(true, window, cx)
            }),
            cx.subscribe(&sleep, |this, _, event: &SelectEvent<_>, cx| {
                if let SelectEvent::Confirm(Some(seconds)) = event {
                    this.set_sleep(*seconds, cx);
                }
            }),
        ];
        let mut section = GeneralSection {
            document,
            sleep,
            sleep_shown: None,
            _subscriptions: subscriptions,
        };
        section.sync_sleep(false, window, cx);
        section
    }

    fn settings(&self, cx: &gpui_kit::App) -> Option<GeneralSettings> {
        self.document.read(cx).settings().loaded().copied()
    }

    fn set(&mut self, change: Change, cx: &mut Context<Self>) {
        self.document
            .update(cx, |document, cx| document.set(change, cx));
    }

    fn step_debounce(&mut self, delta: i16, cx: &mut Context<Self>) {
        if let Some(debounce) = self.settings(cx).and_then(|s| s.stepped_debounce(delta)) {
            self.set(Change::Debounce(debounce), cx);
        }
    }

    fn set_sleep(&mut self, seconds: u16, cx: &mut Context<Self>) {
        if let Some(settings) = self.settings(cx) {
            self.set(Change::Sleep(settings.sleep_after(seconds)), cx);
        }
    }

    /// Builds the sleep menu around the time the keyboard has, unless it already shows that
    /// time. `retitle` rebuilds it regardless, for new texts.
    fn sync_sleep(&mut self, retitle: bool, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.settings(cx).map(|s| s.sleep.keyboard);
        if current == self.sleep_shown && !retitle {
            return;
        }
        self.sleep_shown = current;
        let Some(current) = current else {
            return;
        };
        let choices = sleep_choices(current)
            .into_iter()
            .map(|seconds| SleepChoice {
                seconds,
                title: sleep_label(seconds),
            })
            .collect();
        self.sleep.update(cx, |select, cx| {
            select.set_items(choices, window, cx);
            select.set_selected_value(&current, window, cx);
        });
    }

    fn render_debounce(&self, settings: &GeneralSettings, cx: &mut Context<Self>) -> AnyElement {
        let label: SharedString = t!("general.debounce").into();
        let step = |delta: i16, cx: &mut Context<Self>| {
            let (id, icon, name) = if delta < 0 {
                (
                    "debounce-decrease",
                    IconName::Minus,
                    t!("general.debounce.decrease"),
                )
            } else {
                (
                    "debounce-increase",
                    IconName::Plus,
                    t!("general.debounce.increase"),
                )
            };
            Button::new(id)
                .ghost()
                .small()
                .icon(icon)
                .accessibility_label(SharedString::from(name))
                .disabled(settings.stepped_debounce(delta).is_none())
                .on_click(cx.listener(move |this, _, _, cx| this.step_debounce(delta, cx)))
        };
        let control = h_flex()
            .gap_3()
            .child(step(-1, cx))
            .child(
                div()
                    .w_16()
                    .text_center()
                    .text_sm()
                    .child(settings.debounce.value().to_string()),
            )
            .child(step(1, cx));
        Field::new(label, control)
            .note(Some(
                t!(
                    "general.default",
                    value = GeneralSettings::DEFAULT.debounce.value()
                )
                .into(),
            ))
            .description(t!("general.debounce.description"))
            .into_any_element()
    }

    fn render_polling_rate(
        &self,
        settings: &GeneralSettings,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let selected = POLLING_RATES
            .iter()
            .position(|rate| *rate == settings.polling_rate);
        let control = RadioGroup::horizontal("polling-rate")
            .children(POLLING_RATES.map(polling_rate_label))
            .selected_index(selected)
            .on_change(cx.listener(|this, ix: &usize, _, cx| {
                this.set(Change::PollingRate(POLLING_RATES[*ix]), cx)
            }));
        let default = polling_rate_label(GeneralSettings::DEFAULT.polling_rate);
        Field::new(t!("general.polling_rate"), control)
            .note(Some(t!("general.default", value = default).into()))
            .description(t!("general.polling_rate.description"))
            .into_any_element()
    }

    fn render_sleep(&self) -> AnyElement {
        let label: SharedString = t!("general.sleep").into();
        let default = sleep_label(GeneralSettings::DEFAULT.sleep.keyboard);
        Field::new(
            label.clone(),
            Select::new(&self.sleep)
                .id("sleep")
                .w(SLEEP_WIDTH)
                .menu_width(SLEEP_WIDTH)
                .accessibility_label(label),
        )
        .note(Some(t!("general.default", value = default).into()))
        .into_any_element()
    }

    fn render_loaded(&self, settings: &GeneralSettings, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .gap_6()
            .child(self.render_debounce(settings, cx))
            .child(self.render_polling_rate(settings, cx))
            .child(self.render_sleep())
            .into_any_element()
    }
}

impl Render for GeneralSection {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let message = |text: SharedString| div().text_sm().text_color(muted).child(text);
        // Descriptions wrap at the width of the list above.
        let section = v_flex().w_full().max_w_128();
        match self.document.read(cx).settings() {
            Loadable::Loaded(settings) => {
                let settings = *settings;
                section.child(self.render_loaded(&settings, cx))
            }
            // The keyboard page says what to do when no keyboard is connected.
            Loadable::Idle => section,
            Loadable::Loading => section.child(
                h_flex()
                    .gap_2()
                    .child(Spinner::new().small().color(muted))
                    .child(message(t!("general.loading").into())),
            ),
            Loadable::Failed(reason) => section.child(
                v_flex()
                    .gap_2()
                    .items_start()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().text_sm().child(t!("general.load_failed")))
                            .child(div().text_xs().text_color(muted).child(reason.clone())),
                    )
                    .child(
                        Button::new("general-reload")
                            .small()
                            .label(t!("general.retry"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.document.update(cx, |document, cx| document.reload(cx))
                            })),
                    ),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use bridge75_core::he::Debounce;
    use bridge75_core::info::SleepTimes;
    use gpui_kit::test::TestWindowExt as _;
    use gpui_kit::{
        AnyWindowHandle, App, Bounds, TestAppContext, WindowBounds, WindowOptions, px, size,
    };

    use super::*;

    /// The section in a headless window, over settings that never reach a real keyboard.
    fn open(
        cx: &mut TestAppContext,
        settings: GeneralSettings,
    ) -> (
        AnyWindowHandle,
        Entity<GeneralSection>,
        Entity<GeneralDocument>,
    ) {
        let mut document = None;
        let (window, section) = cx.update(|cx| {
            gpui_kit::init(cx);
            cx.set_global(Preferences::default());
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds {
                    origin: Default::default(),
                    size: size(px(1180.), px(800.)),
                })),
                ..Default::default()
            };
            gpui_kit::open_window(options, cx, |window, cx| {
                let general = cx.new(|cx| GeneralDocument::for_test(settings, cx));
                document = Some(general.clone());
                cx.new(|cx| GeneralSection::new(general, window, cx))
            })
            .unwrap()
        });
        (window, section, document.unwrap())
    }

    fn shown(document: &Entity<GeneralDocument>, cx: &App) -> GeneralSettings {
        *document.read(cx).settings().loaded().unwrap()
    }

    #[gpui_kit::test]
    fn debounce_steps_with_its_buttons_and_stops_at_zero(cx: &mut TestAppContext) {
        let (window, _, document) = open(cx, GeneralSettings::DEFAULT);
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            window.click("debounce-increase", cx);
            assert_eq!(shown(&document, cx).debounce.value(), 4);
            for _ in 0..4 {
                window.render_frame(cx);
                window.click("debounce-decrease", cx);
            }
            assert_eq!(shown(&document, cx).debounce.value(), 0);
            // The button is disabled at zero.
            window.render_frame(cx);
            window.click("debounce-decrease", cx);
            assert_eq!(shown(&document, cx).debounce, Debounce::new(0).unwrap());
        })
        .unwrap();
    }

    #[gpui_kit::test]
    fn the_sleep_menu_offers_a_time_set_elsewhere(cx: &mut TestAppContext) {
        let settings = GeneralSettings {
            sleep: SleepTimes {
                light: 60,
                keyboard: 1000,
            },
            ..GeneralSettings::DEFAULT
        };
        let (window, section, document) = open(cx, settings);
        let sleep = cx.update(|cx| section.read(cx).sleep.clone());
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(sleep.read(cx).selected_value(), Some(&1000));
            // Picking a time in the menu.
            sleep.update(cx, |_, cx| cx.emit(SelectEvent::Confirm(Some(300))));
        })
        .unwrap();
        // The menu follows the document once its observers have run.
        cx.update_window(window, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(shown(&document, cx).sleep.keyboard, 300);
            assert_eq!(sleep.read(cx).selected_value(), Some(&300));
        })
        .unwrap();
    }
}
