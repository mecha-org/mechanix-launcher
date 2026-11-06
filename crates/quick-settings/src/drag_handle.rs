use mecha_wayland::prelude::*;

use crate::util::full_width;

/// Height of the strip under the tiles that holds the handle.
pub const DRAG_HANDLE_HEIGHT: f32 = 50.0;
const BAR_WIDTH: f32 = 32.0;
const BAR_HEIGHT: f32 = 4.0;

/// The bar at the bottom of the panel that hints it can be dragged up to
/// close. Only visual: the panel's window already closes on an upward swipe
/// that starts anywhere in it, including here.
pub struct DragHandle;

pub struct DragHandleBuilder;

impl Build for DragHandleBuilder {
    type Widget = DragHandle;
}

impl Widget for DragHandle {
    type Builder = DragHandleBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = full_width(DRAG_HANDLE_HEIGHT).center();

        let bar = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .size(px(BAR_WIDTH), px(BAR_HEIGHT))
                    .shrink(0.0),
            ),
        );
        *s.component_mut::<Paint>(bar).unwrap() =
            Paint::Quad(Quad::new(s.color(ColorRole::Outline)).radius(BAR_HEIGHT / 2.0));

        DragHandle
    }
}
