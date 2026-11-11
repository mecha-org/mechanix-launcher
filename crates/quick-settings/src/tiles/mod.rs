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

pub(crate) const MAX_TILE_HEIGHT: f32 = 106.0;
pub(crate) const DOT_RADIUS: f32 = 5.0;

/// Vertical icon/label gap as a share of tile height: 12 units at full height.
pub(super) const GAP: Val = percent(100.0 * 12.0 / MAX_TILE_HEIGHT);
/// Wi-Fi, Bluetooth and battery: 20 units at full height, less in compact tiles.
pub(super) const WIDE_GAP: Val = percent(100.0 * 20.0 / MAX_TILE_HEIGHT);

/// Border width per side of a tile, passed to every tile builder.
pub type Borders = Insets<f32>;

/// Common [`Borders`] presets.
pub mod borders {
    use super::Borders;

    const W: f32 = 1.0;
    /// Every side.
    pub const ALL: Borders = Borders::new(W, W, W, W);
    /// Every side but the left, for tiles on the left screen edge.
    pub const NO_LEFT: Borders = Borders::new(W, W, W, 0.0);
    /// Every side but the right, for tiles on the right screen edge.
    pub const NO_RIGHT: Borders = Borders::new(W, 0.0, W, W);
}

/// A tile's background quad. Tiles that repaint call this with their stored
/// [`Borders`] so the per-side widths survive.
pub(super) fn tile_quad(background: Color, border_color: Color, borders: Borders) -> Quad {
    Quad::new(background)
        .border(1.0, border_color)
        .border_widths(borders)
}

/// Applies the frame every tile shares (size, column layout, border) to `me`.
pub(super) fn frame<W: Widget>(me: Handle<W>, s: &mut Spawner<'_, W>, gap: Val, borders: Borders) {
    *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
        .height(percent(100.0))
        .flex_basis(px(0.0))
        .grow(1.0)
        .shrink(0.0)
        .min_width(px(0.0))
        .max_width(percent(20.0))
        .column()
        .center()
        .row_gap(gap);
    let border_color = s.color(ColorRole::SecondaryFixedDim);
    *s.component_mut::<Paint>(me).unwrap() =
        Paint::Quad(tile_quad(Color::TRANSPARENT, border_color, borders));
}

/// Frames `me` as a tile and adds a one-colour icon tinted with OnSurface.
/// The returned `Icon` can be updated with `set_sprite` / `set_color`.
pub(super) fn build_tile<W: Widget>(
    me: Handle<W>,
    s: &mut Spawner<'_, W>,
    svg: &'static [u8],
    gap: Val,
    borders: Borders,
) -> Handle<Icon> {
    frame(me, s, gap, borders);
    let sprite = load_icon(&mut s.resource_mut::<Atlas>(), svg);
    s.spawn(
        me,
        icon(sprite)
            .style(LayoutStyle::default().shrink(0.0))
            .color(s.color(ColorRole::OnSurface)),
    )
}
