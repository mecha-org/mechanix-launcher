use mecha_wayland::prelude::*;

use super::build_image_tile;

mod svg {
    pub const ICON: &[u8] = include_bytes!("../../assets/camera.svg");
}

pub struct Camera;

pub struct CameraBuilder;

impl Build for CameraBuilder {
    type Widget = Camera;
}

impl Widget for Camera {
    type Builder = CameraBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        build_image_tile(me, s, svg::ICON);
        Camera
    }
}
