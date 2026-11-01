//! Live travel of every key.
//!
//! The keyboard reports travel only while the page is shown: [`MonitorPage::set_active`]
//! registers all keys with the keyboard and clears them again when the page is left.

use std::collections::BTreeMap;

use bridge75_core::he::{MAX_TRAVEL, Travel};
use bridge75_core::matrix::{KeyBitmap, KeyPos};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    Context, Entity, IntoElement, ParentElement as _, Render, SharedString, Styled as _,
    Subscription, Window, div, relative,
};
use rust_i18n::t;

use crate::keyboard::KeyboardView;
use crate::page::{self, PageHeader};
use crate::session::{Session, SessionEvent};

pub struct MonitorPage {
    session: Entity<Session>,
    travel: BTreeMap<KeyPos, Travel>,
    active: bool,
    /// Why the keyboard did not accept the monitored keys.
    failure: Option<SharedString>,
    _subscriptions: Vec<Subscription>,
}

impl MonitorPage {
    pub fn new(session: Entity<Session>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::Travel(batch) => {
                if this.active {
                    this.travel.extend(batch.iter().copied());
                    cx.notify();
                }
            }
            SessionEvent::Connected(_) => {
                if this.active {
                    this.start(cx);
                }
            }
            SessionEvent::Disconnected => {
                this.travel.clear();
                this.failure = None;
                cx.notify();
            }
        });
        MonitorPage {
            session,
            travel: BTreeMap::new(),
            active: false,
            failure: None,
            _subscriptions: vec![subscription],
        }
    }

    fn is_connected(&self, cx: &Context<Self>) -> bool {
        self.session.read(cx).connection().is_some()
    }

    /// Starts or stops the travel reports. The shell calls this when the page is shown or left.
    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.active == active {
            return;
        }
        self.active = active;
        if active {
            // Without a keyboard, reports start when it connects.
            if self.is_connected(cx) {
                self.start(cx);
            }
        } else {
            self.travel.clear();
            if self.is_connected(cx) {
                let stop = self.session.update(cx, |session, cx| {
                    session.run(|device| device.clear_monitor(), cx)
                });
                // Nothing to show for a failure: the keyboard also stops reporting without
                // heartbeats.
                stop.detach();
            }
        }
        cx.notify();
    }

    fn start(&mut self, cx: &mut Context<Self>) {
        self.failure = None;
        let start = self.session.update(cx, |session, cx| {
            session.run(|device| device.set_monitor(&KeyBitmap::all()), cx)
        });
        cx.spawn(async move |this, cx| {
            if let Err(e) = start.await {
                let _ = this.update(cx, |this, cx| {
                    this.failure = Some(e.to_string().into());
                    cx.notify();
                });
            }
        })
        .detach();
    }
}

impl Render for MonitorPage {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let connected = self.is_connected(cx);
        let theme = cx.theme();
        let keyboard = KeyboardView::new("monitor-keyboard", |key| {
            let value = self
                .travel
                .get(&key.pos)
                .copied()
                .filter(|v| connected && v.raw() > 0);
            let ratio = value.map_or(0.0, |v| {
                (v.raw() as f32 / MAX_TRAVEL as f32).clamp(0.0, 1.0)
            });
            div()
                .relative()
                .size_full()
                .rounded(theme.radius)
                .border_1()
                .border_color(theme.border)
                .when(connected, |this| this.bg(theme.secondary))
                // Text stays clear of the bar, so it never sits on the fill.
                .child(
                    v_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .gap_0p5()
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .when(!connected, |this| this.opacity(0.5))
                                .child(key.label),
                        )
                        .when_some(value, |this, value| {
                            this.child(
                                div()
                                    .text_sm()
                                    .text_color(theme.foreground)
                                    .child(value.to_string()),
                            )
                        }),
                )
                // A rounded parent does not clip its children, so the bar rounds its own
                // bottom corners to follow the keycap.
                .when(ratio > 0.0, |this| {
                    this.child(
                        div()
                            .absolute()
                            .bottom_0()
                            .left_0()
                            .h_1p5()
                            .w(relative(ratio))
                            .rounded_b(theme.radius)
                            .bg(theme.progress_bar),
                    )
                })
                .into_any_element()
        });

        let muted = theme.muted_foreground;
        let description = if connected {
            t!("monitor.description")
        } else {
            t!("monitor.not_connected")
        };
        v_flex()
            .w_full()
            .p(page::INSET)
            .gap_6()
            .child(PageHeader::new(t!("monitor.title"), description))
            .when_some(self.failure.clone(), |this, failure| {
                this.child(
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex()
                                .gap_2()
                                .text_sm()
                                .child(
                                    Icon::new(IconName::CircleX)
                                        .small()
                                        .text_color(theme.danger),
                                )
                                .child(t!("monitor.start_failed")),
                        )
                        .child(div().text_xs().text_color(muted).child(failure)),
                )
            })
            .child(keyboard)
    }
}
