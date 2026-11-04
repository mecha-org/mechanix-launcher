use assets::icons::wifi as icons;
use mecha_wayland::prelude::*;

use super::{GAP, build_label, build_tile};

pub struct Wifi;

pub struct WifiBuilder;

impl Build for WifiBuilder {
    type Widget = Wifi;
}

impl Widget for Wifi {
    type Builder = WifiBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        build_tile(me, s, icons::HIGH, GAP);
        build_label(me, s, "Placeholder SSID");
        Wifi
    }
}
