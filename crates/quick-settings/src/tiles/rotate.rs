use mecha_wayland::prelude::*;

use super::build_image_tile;

mod svg {
    pub const ICON: &[u8] = include_bytes!("../../assets/rotate.svg");
}

pub struct Rotate;

pub struct RotateBuilder;

impl Build for RotateBuilder {
    type Widget = Rotate;
}

impl Widget for Rotate {
    type Builder = RotateBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        build_image_tile(me, s, svg::ICON);
        Rotate
    }
}
