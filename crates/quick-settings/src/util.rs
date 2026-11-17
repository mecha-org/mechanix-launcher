use mecha_wayland::prelude::*;
// TODO: need to refactor this file
const ICON_SIZE: u32 = 24;

pub fn full_width(height: f32) -> LayoutStyle {
    LayoutStyle::default()
        .size(percent(100.0), px(height))
        .shrink(0.0)
}

/// Loads an SVG as a one-colour shape (its colours are dropped; the widget's
/// `color` fills it). The atlas keeps the SVG and redraws it when the display
/// scale changes, so it stays sharp.
pub fn load_icon(atlas: &mut Atlas, svg: &'static [u8]) -> SpriteId {
    load_icon_sized(atlas, svg, ICON_SIZE)
}

pub fn load_icon_sized(atlas: &mut Atlas, svg: &'static [u8], size: u32) -> SpriteId {
    atlas
        .insert_svg(svg, size)
        .expect("embedded icon SVG should render and fit in the atlas")
}
