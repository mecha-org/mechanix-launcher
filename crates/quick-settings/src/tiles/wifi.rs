use assets::icons::wifi as icons;
use mecha_wayland::mechanix_widgets::prelude::{Text, text};
use mecha_wayland::prelude::*;

use super::{DOT_RADIUS, GAP, build_label, build_tile};

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
        let ssid = build_label(me, s, "");
        let dot = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .size(px(DOT_RADIUS), px(DOT_RADIUS))
                    .absolute()
                    .inset(Insets::new(px(8.0), px(10.0), auto(), auto())),
            ),
        );

        // temporarily draw the dot immediately
        *s.component_mut::<Paint>(dot).unwrap() =
            Paint::Quad(Quad::new(s.color(ColorRole::Primary)).radius(5.0));
        Wifi
    }
}
