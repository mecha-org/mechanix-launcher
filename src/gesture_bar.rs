//! Android-style gesture bar: a black strip that reserves the bottom of the
//! screen and shows the home pill. The sizes are the Nothing Phone (4a)
//! stock firmware's (gestural navigation overlay and SystemUI), in dp,
//! which are logical px at scale 3.

use mecha_wayland::prelude::*;
use screen::Screen;

/// Height of the bar, reserved under apps: `navigation_bar_height`.
pub const NAV_BAR_HEIGHT: f32 = 24.0;
/// Height of the bottom swipe area: `navigation_bar_gesture_height`.
pub const GESTURE_HEIGHT: f32 = 32.0;

/// `navigation_home_handle_width`.
const PILL_WIDTH: f32 = 132.0;
/// `navigation_handle_radius`; the pill is twice it tall.
const PILL_RADIUS: f32 = 2.0;
/// Gap under the pill: `navigation_handle_bottom`.
const PILL_BOTTOM: f32 = 8.0;
/// `navigation_bar_home_handle_light_color` (#EBFFFFFF), for dark content.
const PILL_LIGHT: Color = Color::from_rgba8(0xff, 0xff, 0xff, 0xeb);

/// The bar's content: black, with the pill centred at the bottom.
pub struct GestureBarScreen;

impl Screen for GestureBarScreen {
    fn build(&self, app: &mut App, parent: NodeId) -> NodeId {
        let bar = app
            .spawn(
                parent,
                div().background(Color::BLACK).style(
                    LayoutStyle::default()
                        .row()
                        .size(percent(100.0), percent(100.0))
                        .justify(Justify::Center)
                        .align_items(Align::End)
                        .padding(Insets::new(px(0.0), px(0.0), px(PILL_BOTTOM), px(0.0))),
                ),
            )
            .id();
        app.spawn(
            bar,
            div()
                .background(PILL_LIGHT)
                .radius(PILL_RADIUS)
                .style(LayoutStyle::default().size(px(PILL_WIDTH), px(2.0 * PILL_RADIUS))),
        );
        bar
    }
}
