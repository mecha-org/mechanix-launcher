//! Wi-Fi tile presentation driven by NetworkManager events.

use assets::icons::wifi as icons;
use mecha_wayland::mechanix_widgets::prelude::{
    Text, TextContextExt, TextOverflow, TextVariant, text,
};
use mecha_wayland::prelude::*;

use services::{APUpdate, SetWifiEnabled, StateUpdate, WifiState};

use super::{Borders, DOT_RADIUS, build_tile, label_spacer, tile_quad};
use crate::util::{load_icon, short_name};

/// Latest Wi-Fi details used to render the tile.
pub struct Wifi {
    enabled: bool,
    state: WifiState,
    ssid: String,
    strength: u8,
    borders: Borders,
    spacer: Handle<Div>,
    // TODO: maybe store sprites in the widget itself ?
}

/// Atlas sprites shared by the tile's event handlers.
#[derive(Clone, Copy)]
struct WifiSprites {
    /// Shown while Wi-Fi is off or on but not connected; only its tint changes.
    wifi: SpriteId,
    connecting: SpriteId,
    none: SpriteId,
    low: SpriteId,
    medium: SpriteId,
    high: SpriteId,
}

/// Builds the Wi-Fi tile and subscribes it to updates.
pub struct WifiBuilder {
    /// Which sides get a border, e.g. [`borders::ALL`](super::borders::ALL).
    pub borders: Borders,
}

impl Build for WifiBuilder {
    type Widget = Wifi;
}

impl Widget for Wifi {
    type Builder = WifiBuilder;

    fn build(b: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        let icon_handle = build_tile(me, s, icons::HIGH, b.borders);
        let spacer = label_spacer(me, s);
        // Load sprites once; event handlers only switch between their IDs.
        let atlas = &mut s.resource_mut::<Atlas>();
        let sprites = WifiSprites {
            wifi: load_icon(atlas, icons::HIGH),
            connecting: load_icon(atlas, icons::CONNECTING),
            none: load_icon(atlas, icons::NONE),
            low: load_icon(atlas, icons::LOW),
            medium: load_icon(atlas, icons::MEDIUM),
            high: load_icon(atlas, icons::HIGH),
        };
        // Hidden until connected so the icon sits centred; showing it pushes the icon up.
        // Same text styling as the Bluetooth tile; ellipsis overflow is a fallback for narrow tiles.
        let ssid_handle = s.spawn(
            me,
            text("")
                .variant(TextVariant::LabelLarge)
                .max_lines(1)
                .overflow(TextOverflow::Ellipsis)
                .style(
                    LayoutStyle::default()
                        .max_width(percent(100.0))
                        .shrink(0.0)
                        .hidden(),
                )
                .color(ColorRole::OnSurfaceVariant),
        );
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
            // Cache AP details, then render using the current connection state.
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
            // A state change can show or hide the cached AP.
            tracing::info!("Wifi state changed {:?}", e.state);
            let wifi = ctx.me();
            wifi.state = e.state;
            // Keep the toggle in sync with the real radio state.
            wifi.enabled = !matches!(e.state, WifiState::Unavailable | WifiState::Off);
            render(ctx, icon_handle, ssid_handle, dot, sprites);
        });

        // Send the toggle request; the service reports the resulting state.
        s.on::<Clicked>(me, |ctx, _| {
            let wifi = ctx.me();
            wifi.enabled = !wifi.enabled;
            let enabled = wifi.enabled;
            ctx.signal(SetWifiEnabled(enabled));
        });

        Wifi {
            enabled: false,
            state: WifiState::Unavailable,
            ssid: "".to_string(),
            strength: 0,
            borders: b.borders,
            spacer,
        }
    }
}

/// Resolves the latest state and AP details into one tile appearance.
fn render(
    ctx: &mut Context<'_, Wifi>,
    icon_handle: Handle<Icon>,
    ssid_handle: Handle<Text>,
    dot: Handle<Div>,
    sprites: WifiSprites,
) {
    let border_colour = ctx.color(ColorRole::SecondaryFixedDim);
    let off_icon_colour = ctx.color(ColorRole::OnSecondaryFixedVariant);
    let on_icon_colour = ctx.color(ColorRole::OnSurface);

    let wifi = ctx.me();
    let wifi_borders = wifi.borders;
    let mut ssid = String::new();

    // Connection state takes precedence over any cached SSID or strength.
    let (sprite, on, connected) = match wifi.state {
        WifiState::Unavailable | WifiState::Off => (sprites.wifi, false, false),
        // Enabled but not connected (On is the fallback before a device is known).
        WifiState::On | WifiState::Disconnected => (sprites.wifi, true, false),
        WifiState::Connecting | WifiState::Disconnecting => (sprites.connecting, true, false),
        WifiState::Connected => {
            // Only a connected AP supplies the visible SSID and signal icon.
            ssid = short_name(&wifi.ssid);
            let sprite = match wifi.strength {
                0..25 => sprites.none,
                25..50 => sprites.low,
                50..75 => sprites.medium,
                _ => sprites.high,
            };
            (sprite, true, true)
        }
    };

    let background = if on {
        ctx.color(ColorRole::Surface)
    } else {
        Color::TRANSPARENT
    };
    let tile = tile_quad(background, border_colour, wifi_borders);
    let icon_colour = if on { on_icon_colour } else { off_icon_colour };
    let dot_paint = if on {
        Paint::Quad(Quad::new(ctx.color(ColorRole::Primary)).radius(DOT_RADIUS))
    } else {
        Paint::None
    };

    // Apply the appearance together after selecting it above.
    ctx.set_paint(Paint::Quad(tile));
    let spacer = ctx.me().spacer;
    ctx.at(spacer).unwrap().set_display(if connected {
        Display::Flex
    } else {
        Display::Hidden
    });
    let mut label = ctx.at(ssid_handle).unwrap();
    label.set_display(if connected {
        Display::Flex
    } else {
        Display::Hidden
    });
    label.set_text(ssid);
    let mut icon = ctx.at(icon_handle).unwrap();
    icon.set_sprite(sprite);
    icon.set_color(icon_colour);
    ctx.at(dot).unwrap().set_paint(dot_paint);
}
