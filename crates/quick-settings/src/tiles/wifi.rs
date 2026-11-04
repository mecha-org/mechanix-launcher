use assets::icons::wifi as icons;
use mecha_wayland::mechanix_widgets::prelude::{Text, TextContextExt, text};
use mecha_wayland::prelude::*;

use super::{DOT_RADIUS, GAP, build_label, build_tile};
use services::{APUpdate, StateUpdate, WifiState};

pub struct Wifi {
    state: WifiState,
    ssid: String,
    strength: u8,
}

pub struct WifiBuilder;

impl Build for WifiBuilder {
    type Widget = Wifi;
}

impl Widget for Wifi {
    type Builder = WifiBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        build_tile(me, s, icons::HIGH, GAP);
        let ssid_handle = build_label(me, s, "");
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

        s.on::<APUpdate>(me, |ctx, e| {
            let label = e
                .ssid
                .as_deref()
                .map(String::from_utf8_lossy)
                .unwrap_or_default()
                .into_owned(); // TODO: make sure this is Cow::Owned, otherwise this is a needless clone
            tracing::info!(
                "Received Access point update, SSID: {} strength: {}",
                &label,
                e.strength
            );
            ctx.me().ssid = label;
            ctx.me().strength = e.strength;
        });

        Wifi {
            state: WifiState::Unavailable,
            ssid: "".to_string(),
            strength: 0,
        }
    }
}
