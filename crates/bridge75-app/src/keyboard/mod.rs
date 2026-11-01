//! The keyboard drawn to scale, with a page-supplied element in each key.

mod navigation;
mod selection;

use bridge75_core::matrix::{BOARD_HEIGHT, BOARD_WIDTH, Key, KeyRect, LAYOUT};
use gpui_kit::{
    AnyElement, App, ElementId, InteractiveElement as _, IntoElement, ParentElement as _, Rems,
    RenderOnce, Styled as _, Window, div, rems,
};

pub use navigation::{Direction, neighbor};
pub use selection::KeySelection;

/// Length of 1u in rem, so the board follows the application zoom.
const UNIT: f32 = 3.0;

/// Width of the drawn board.
pub const WIDTH: Rems = quarters(BOARD_WIDTH);

const fn quarters(q: u8) -> Rems {
    rems(q as f32 * UNIT / 4.0)
}

#[derive(IntoElement)]
pub struct KeyboardView {
    id: ElementId,
    keys: Vec<(KeyRect, AnyElement)>,
}

impl KeyboardView {
    /// `render_key` supplies the contents of each keycap, which fill the keycap's area.
    pub fn new(
        id: impl Into<ElementId>,
        mut render_key: impl FnMut(&'static Key) -> AnyElement,
    ) -> Self {
        KeyboardView {
            id: id.into(),
            keys: LAYOUT
                .iter()
                .map(|key| (key.rect, render_key(key)))
                .collect(),
        }
    }
}

impl RenderOnce for KeyboardView {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .id(self.id)
            .relative()
            .flex_none()
            .w(WIDTH)
            .h(quarters(BOARD_HEIGHT))
            .children(self.keys.into_iter().map(|(rect, element)| {
                div()
                    .absolute()
                    .left(quarters(rect.x))
                    .top(quarters(rect.y))
                    .w(quarters(rect.w))
                    .h(quarters(rect.h))
                    .p_0p5()
                    .child(element)
            }))
    }
}
