use assets::icons::wifi as icons;
use mecha_wayland::mechanix_widgets::prelude::{Text, TextContextExt, text};
use mecha_wayland::prelude::*;

use super::{DOT_RADIUS, GAP, build_label, build_tile, load_icon};
use services::{APUpdate, StateUpdate, WifiState};

pub struct Wifi {
    state: WifiState,
    ssid: String,
    strength: u8,
}

#[derive(Clone, Copy)]
struct WifiSprites {
    x: SpriteId,
    slash: SpriteId,
    connecting: SpriteId,
    none: SpriteId,
    low: SpriteId,
    medium: SpriteId,
    high: SpriteId,
}

pub struct WifiBuilder;

impl Build for WifiBuilder {
    type Widget = Wifi;
}

impl Widget for Wifi {
    type Builder = WifiBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        let icon_handle = build_tile(me, s, icons::X, GAP);
        let atlas = &mut s.resource_mut::<Atlas>();
        let sprites = WifiSprites {
            x: load_icon(atlas, icons::X),
            slash: load_icon(atlas, icons::SLASH),
            connecting: load_icon(atlas, icons::CONNECTING),
            none: load_icon(atlas, icons::NONE),
            low: load_icon(atlas, icons::LOW),
            medium: load_icon(atlas, icons::MEDIUM),
            high: load_icon(atlas, icons::HIGH),
        };
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

        s.on::<APUpdate>(me, move |ctx, e| {
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

            render(ctx, icon_handle, ssid_handle, dot);
        });

        Wifi {
            state: WifiState::Unavailable,
            ssid: "".to_string(),
            strength: 0,
        }
    }
}

// make changes to tile
fn render<W: Widget>(
    ctx: &mut Context<'_, W>,
    icon_handle: Handle<Icon>,
    ssid_handle: Handle<Text>,
    dot: Handle<Div>,
) {
}
