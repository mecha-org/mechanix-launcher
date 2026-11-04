use assets::icons::wifi as icons;
use mecha_wayland::mechanix_widgets::prelude::{Text, TextContextExt};
use mecha_wayland::prelude::*;

use services::{APUpdate, StateUpdate, WifiState};

use super::{DOT_RADIUS, GAP, build_label, build_tile, load_icon, short_name};

pub struct Wifi {
    state: WifiState,
    ssid: String,
    strength: u8,
    // TODO: maybe store sprites in the widget itself ?
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

            render(ctx, icon_handle, ssid_handle, dot, sprites);
        });

        s.on::<StateUpdate>(me, move |ctx, e| {
            tracing::info!("Wifi state changed {:?}", e.state);
            ctx.me().state = e.state;
            render(ctx, icon_handle, ssid_handle, dot, sprites);
        });

        Wifi {
            state: WifiState::Unavailable,
            ssid: "".to_string(),
            strength: 0,
        }
    }
}

// make changes to tile
fn render(
    ctx: &mut Context<'_, Wifi>,
    icon_handle: Handle<Icon>,
    ssid_handle: Handle<Text>,
    dot: Handle<Div>,
    sprites: WifiSprites,
) {
    let off_colour = Color::TRANSPARENT;
    let on_colour = ctx.color(ColorRole::Surface);
    let border_colour = ctx.color(ColorRole::SecondaryFixedDim);

    let wifi = ctx.me();
    let sprite;
    let mut ssid: String = String::new();
    let dot_enabled;
    let background;

    match wifi.state {
        WifiState::Unavailable => {
            sprite = sprites.x;
            dot_enabled = false;
            background = off_colour;
        }
        WifiState::Off => {
            sprite = sprites.slash;
            dot_enabled = false;
            background = off_colour;
        }
        WifiState::On => {
            sprite = sprites.high;
            dot_enabled = false;
            background = on_colour;
        }
        WifiState::Disconnected => {
            // This WILL be the same as ON
            // ON is a fallback when no device yet but wirelessEnabled is there
            // Basically, this branch will never run because our network manager
            // service should never emit On
            sprite = sprites.high;
            dot_enabled = false;
            background = off_colour;
        }
        WifiState::Connecting => {
            sprite = sprites.connecting;
            dot_enabled = false;
            background = on_colour;
        }
        WifiState::Disconnecting => {
            sprite = sprites.connecting;
            dot_enabled = false;
            background = on_colour;
        }
        WifiState::Connected => {
            sprite = match wifi.strength {
                0..25 => sprites.none,
                25..50 => sprites.low,
                50..75 => sprites.medium,
                _ => sprites.high,
            };
            ssid = short_name(&wifi.ssid);
            dot_enabled = true;
            background = on_colour;
        }
    };

    ctx.at(ssid_handle).unwrap().set_text(ssid);
    ctx.at(icon_handle).unwrap().set_sprite(sprite);
    ctx.set_paint(Paint::Quad(
        Quad::new(background).border(1.0, border_colour),
    ));

    let dot_colour = if dot_enabled {
        Paint::Quad(Quad::new(ctx.color(ColorRole::Primary)).radius(DOT_RADIUS / 2.0))
    } else {
        Paint::None
    };
    ctx.at(dot).unwrap().set_paint(dot_colour);
}
