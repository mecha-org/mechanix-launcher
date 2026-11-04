use mecha_wayland::prelude::*;

use super::build_image_tile;

mod svg {
    pub const ICON: &[u8] = include_bytes!("../../assets/settings.svg");
}

pub struct Settings;

pub struct SettingsBuilder;

impl Build for SettingsBuilder {
    type Widget = Settings;
}

impl Widget for Settings {
    type Builder = SettingsBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        build_image_tile(me, s, svg::ICON);
        Settings
    }
}
