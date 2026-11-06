use mecha_wayland::prelude::*;
// TODO: need to refactor this file
const ICON_SIZE: u32 = 32;

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

/// Longest label a tile shows in full; anything longer keeps the first
/// [`TRUNCATED_LEN`] characters followed by `...`.
const MAX_LABEL_LEN: usize = 7;
const TRUNCATED_LEN: usize = 5;

/// Shortens a device / network name for the Wi-Fi and Bluetooth tiles.
/// Counts characters, not bytes, so multi-byte names are never split mid-glyph.
pub fn short_name(name: &str) -> String {
    if name.chars().count() <= MAX_LABEL_LEN {
        name.to_string()
    } else {
        format!(
            "{}...",
            name.chars().take(TRUNCATED_LEN).collect::<String>()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::short_name;

    #[test]
    fn names_longer_than_seven_characters_are_cut_to_five() {
        assert_eq!(short_name(""), "");
        assert_eq!(short_name("Buds"), "Buds");
        assert_eq!(short_name("Office1"), "Office1");
        assert_eq!(short_name("Office 01"), "Offic...");
        assert_eq!(short_name("Headphones"), "Headp...");
        assert_eq!(short_name("Ünïcødé!"), "Ünïcø...");
    }
}
