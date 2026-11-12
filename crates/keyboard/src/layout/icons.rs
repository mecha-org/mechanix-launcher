//! Bundled SVG icons, rasterized into the atlas at load and keyed by the name
//! layouts refer to them by.

use mecha_wayland::prelude::*;
use std::collections::HashMap;

macro_rules! include_icon {
    ($name:literal) => {
        (
            $name,
            include_bytes!(concat!("../../resources/icons/", $name, ".svg")),
        )
    };
}

/// Every bundled SVG icon, keyed by the name layouts refer to it by.
const BUNDLED: &[(&str, &[u8])] = &[
    // Shared with the lock screen's numpad.
    ("edit-clear-symbolic", assets::icons::BACKSPACE),
    include_icon!("face-smile-symbolic"),
    include_icon!("go-down-symbolic"),
    include_icon!("go-next-symbolic"),
    include_icon!("go-previous-symbolic"),
    include_icon!("go-up-symbolic"),
    ("key-enter-symbolic", assets::icons::ENTER),
    include_icon!("key-shift-symbolic"),
    include_icon!("preferences-system-symbolic"),
    include_icon!("view-reserve-bottom-symbolic"),
];

/// The rasterized icon sprites, looked up by the name a layout uses.
#[derive(Debug)]
pub struct Icons {
    icons: HashMap<String, SpriteId>,
}

impl Resource for Icons {}

impl Icons {
    /// Rasterize every bundled icon at `size` and insert it into the atlas.
    pub fn load(atlas: &mut Atlas, size: u32) -> Self {
        let mut icons: HashMap<String, SpriteId> = HashMap::new();
        for &(name, bytes) in BUNDLED {
            let sprite = atlas
                .insert_svg(bytes, size)
                .unwrap_or_else(|e| panic!("bundled icon {name:?} loads: {e}"));
            icons.insert(name.to_owned(), sprite);
        }
        Self { icons }
    }

    /// The sprite for a named icon, or `None` if no bundled icon has that name.
    pub fn get(&self, name: &str) -> Option<SpriteId> {
        self.icons.get(name).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_icons_load() {
        let icons = Icons::load(&mut Atlas::new(), 24);
        assert_eq!(icons.icons.len(), BUNDLED.len());
        assert!(icons.get("key-shift-symbolic").is_some());
    }
}
