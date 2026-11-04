use mecha_wayland::mechanix_widgets::prelude::{Text, text};
use mecha_wayland::prelude::*;

use crate::util::load_icon;

pub mod action;
pub mod battery;
pub mod bluetooth;
pub mod wifi;

pub use action::{action_tile, svg};
pub use battery::BatteryBuilder;
pub use bluetooth::BluetoothBuilder;
pub use wifi::WifiBuilder;

pub(crate) const TILE_HEIGHT: f32 = 106.0;
pub(crate) const DOT_RADIUS: f32 = 10.0;

/// Default space between a tile's icon and its label.
pub(super) const GAP: f32 = 12.0;
/// Wider icon/label space used by Bluetooth and Battery.
pub(super) const WIDE_GAP: f32 = 20.0;

/// Applies the frame every tile shares (size, column layout, border) to `me`.
pub(super) fn frame<W: Widget>(me: Handle<W>, s: &mut Spawner<'_, W>, gap: f32) {
    *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
        .height(px(TILE_HEIGHT))
        .flex_basis(px(0.0))
        .grow(1.0)
        .shrink(0.0)
        .min_width(px(0.0))
        .max_width(percent(20.0))
        .column()
        .center()
        .gap(px(gap));
    *s.component_mut::<Paint>(me).unwrap() = Paint::Quad(
        Quad::new(Color::TRANSPARENT).border(1.0, s.color(ColorRole::SecondaryFixedDim)),
    );
}

/// Frames `me` as a tile and adds a one-colour icon tinted with OnSurface.
/// The returned `Icon` can be updated with `set_sprite` / `set_color`.
pub(super) fn build_tile<W: Widget>(
    me: Handle<W>,
    s: &mut Spawner<'_, W>,
    svg: &'static [u8],
    gap: f32,
) -> Handle<Icon> {
    frame(me, s, gap);
    let sprite = load_icon(&mut s.resource_mut::<Atlas>(), svg);
    s.spawn(
        me,
        icon(sprite)
            .style(LayoutStyle::default().shrink(0.0))
            .color(s.color(ColorRole::OnSurface)),
    )
}

pub(super) fn build_label<W: Widget>(
    me: Handle<W>,
    s: &mut Spawner<'_, W>,
    value: impl Into<String>,
) -> Handle<Text> {
    s.spawn(
        me,
        text(value)
            .size(18.0)
            .style(LayoutStyle::default().shrink(0.0))
            .color(ColorRole::OnSurfaceVariant),
    )
}
