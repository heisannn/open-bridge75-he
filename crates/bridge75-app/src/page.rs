//! Parts every page shares, so the pages line up with one another.

use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};
use gpui_kit::{
    AnyElement, App, FontWeight, IntoElement, ParentElement as _, Rems, RenderOnce, SharedString,
    Styled as _, Window, div, rems,
};

/// Space between the edge of a page and its content.
pub const INSET: Rems = rems(1.5);

/// Title at the top of a page, with a description when the page needs one.
#[derive(IntoElement)]
pub struct PageHeader {
    title: SharedString,
    description: Option<SharedString>,
}

impl PageHeader {
    pub fn new(title: impl Into<SharedString>) -> Self {
        PageHeader {
            title: title.into(),
            description: None,
        }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }
}

impl RenderOnce for PageHeader {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        v_flex()
            .gap_1()
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.title),
            )
            .children(self.description.map(|description| {
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(description)
            }))
    }
}

/// A labeled control: the label above, the control below at the gap all pages use.
#[derive(IntoElement)]
pub struct Field {
    label: SharedString,
    /// Shown after the label in muted text, such as a note that keys differ.
    note: Option<SharedString>,
    control: AnyElement,
}

impl Field {
    pub fn new(label: impl Into<SharedString>, control: impl IntoElement) -> Self {
        Field {
            label: label.into(),
            note: None,
            control: control.into_any_element(),
        }
    }

    pub fn note(mut self, note: Option<SharedString>) -> Self {
        self.note = note;
        self
    }
}

impl RenderOnce for Field {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let muted = cx.theme().muted_foreground;
        v_flex()
            .gap_2()
            .child(
                h_flex()
                    .gap_2()
                    .text_sm()
                    .child(div().font_weight(FontWeight::MEDIUM).child(self.label))
                    .children(
                        self.note
                            .map(|note| div().text_xs().text_color(muted).child(note)),
                    ),
            )
            .child(self.control)
    }
}
