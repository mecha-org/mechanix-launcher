use mecha_wayland::prelude::*;

use super::build_image_tile;

mod svg {
    pub const ICON: &[u8] = include_bytes!("../../assets/record.svg");
}

pub struct Record;

pub struct RecordBuilder;

impl Build for RecordBuilder {
    type Widget = Record;
}

impl Widget for Record {
    type Builder = RecordBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        build_image_tile(me, s, svg::ICON);
        Record
    }
}
