pub mod airplane;
pub mod battery;
pub mod bluetooth;
pub mod cellular;
pub mod glyph;
pub mod wifi;

pub use airplane::{
    AirplaneIcons, AirplaneIndicator, AirplaneIndicatorBuilder, AirplaneIndicatorContext,
    AirplaneState, airplane_indicator,
};
pub use battery::{
    BatteryPercentage, BatteryPercentageBuilder, BatteryPercentageContext, battery_percentage,
};
pub use bluetooth::{
    BluetoothIcons, BluetoothIndicator, BluetoothIndicatorBuilder, BluetoothIndicatorContext,
    BluetoothState, bluetooth_indicator,
};
pub use cellular::{
    CellularIcons, CellularIndicator, CellularIndicatorBuilder, CellularIndicatorContext,
    CellularState, cellular_indicator,
};
pub use glyph::{
    GLYPH_COLS, GLYPH_DOT_GAP, GLYPH_DOT_RADIUS, GLYPH_DOT_SIZE, GLYPH_ROWS, GLYPH_TOTAL_DOTS,
    GlyphIndicator, GlyphIndicatorBuilder, GlyphIndicatorContext, GlyphState, glyph_indicator,
};
pub use wifi::{
    WifiIcons, WifiIndicator, WifiIndicatorBuilder, WifiIndicatorContext, WifiState, wifi_indicator,
};

use mecha_wayland::prelude::*;

#[inline]
pub(crate) fn indicator_layout(visible: bool) -> LayoutStyle {
    if visible {
        let size = crate::DEFAULT_ICON_SIZE as f32;
        // Centered so a sprite smaller than the slot stays aligned.
        LayoutStyle::default().size(px(size), px(size)).center()
    } else {
        LayoutStyle::default().hidden()
    }
}

#[inline]
pub(crate) fn load_svg_icon(
    atlas: &mut Atlas,
    bytes: &'static [u8],
    master: u32,
) -> Result<SpriteId, Error> {
    atlas.insert_svg(bytes, master)
}
