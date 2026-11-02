//! Connection state and what the keyboard reports about itself.

use gpui_kit::component::description_list::DescriptionList;
use gpui_kit::component::{ActiveTheme as _, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    App, IntoElement, ParentElement as _, RenderOnce, SharedString, Styled as _, Window, div, rems,
};
use rust_i18n::t;

use crate::page::{self, PageHeader};
use crate::session::{KeyboardInfo, SearchState, SessionState};

#[derive(IntoElement)]
pub struct OverviewPage {
    state: SessionState,
}

impl OverviewPage {
    pub fn new(state: SessionState) -> Self {
        OverviewPage { state }
    }

    fn render_info(info: &KeyboardInfo) -> impl IntoElement {
        let firmware = &info.device.firmware;
        let build = &info.device.build;
        let on_off = |on: bool| -> SharedString {
            if on {
                t!("overview.on")
            } else {
                t!("overview.off")
            }
            .into()
        };
        let list = DescriptionList::horizontal()
            .bordered(true)
            .label_width(rems(12.))
            .columns(1)
            .item(
                SharedString::from(t!("overview.firmware")),
                format!("{} ({})", firmware.version, firmware.product),
                1,
            )
            .item(
                SharedString::from(t!("overview.build")),
                format!("{} {}", build.date, build.time),
                1,
            )
            .item(
                SharedString::from(t!("overview.serial")),
                info.serial
                    .clone()
                    .map_or_else(|| t!("overview.serial_unknown").into(), SharedString::from),
                1,
            )
            .item(
                SharedString::from(t!("overview.calibration")),
                SharedString::from(if info.heartbeat.calibrated {
                    t!("overview.calibrated")
                } else {
                    t!("overview.not_calibrated")
                }),
                1,
            )
            .item(
                SharedString::from(t!("overview.win_lock")),
                on_off(info.heartbeat.win_lock),
                1,
            );
        // Short values; a full-width list would put them far from their labels.
        div().w_full().max_w_128().child(list)
    }
}

impl RenderOnce for OverviewPage {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        let body = match &self.state {
            SessionState::Connected { info, .. } => Self::render_info(info).into_any_element(),
            SessionState::Searching(_) | SessionState::Stopped => {
                let (message, detail): (SharedString, Option<&String>) = match &self.state {
                    SessionState::Searching(SearchState::Bootloader) => {
                        (t!("overview.bootloader").into(), None)
                    }
                    SessionState::Searching(SearchState::Failed(reason)) => {
                        (t!("overview.unavailable").into(), Some(reason))
                    }
                    SessionState::Searching(SearchState::Rejected(reason)) => {
                        (t!("overview.rejected").into(), Some(reason))
                    }
                    SessionState::Stopped => (t!("overview.stopped").into(), None),
                    _ => (t!("overview.searching").into(), None),
                };
                v_flex()
                    .gap_1()
                    .child(div().text_sm().child(message))
                    .when_some(detail, |this, detail| {
                        this.child(div().text_xs().text_color(muted).child(detail.clone()))
                    })
                    .into_any_element()
            }
        };
        v_flex()
            .w_full()
            .p(page::INSET)
            .gap_6()
            .child(PageHeader::new(t!("overview.title")))
            .child(body)
    }
}
