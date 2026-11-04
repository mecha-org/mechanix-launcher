use mecha_wayland::prelude::*;

use super::build_image_tile;

mod svg {
    pub const ICON: &[u8] = include_bytes!("../../assets/display.svg");
}

pub struct DisplayTile;

pub struct DisplayTileBuilder;

impl Build for DisplayTileBuilder {
    type Widget = DisplayTile;
}

impl Widget for DisplayTile {
    type Builder = DisplayTileBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        build_image_tile(me, s, svg::ICON);
        DisplayTile
    }
}
