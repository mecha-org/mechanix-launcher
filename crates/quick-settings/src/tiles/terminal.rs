use mecha_wayland::prelude::*;

use super::build_image_tile;

mod svg {
    pub const ICON: &[u8] = include_bytes!("../../assets/terminal.svg");
}

pub struct Terminal;

pub struct TerminalBuilder;

impl Build for TerminalBuilder {
    type Widget = Terminal;
}

impl Widget for Terminal {
    type Builder = TerminalBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        build_image_tile(me, s, svg::ICON);
        Terminal
    }
}
