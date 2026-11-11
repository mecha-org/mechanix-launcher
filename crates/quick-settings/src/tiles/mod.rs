use mecha_wayland::prelude::*;

use crate::util::load_icon;

pub mod action;
pub mod battery;
pub mod bluetooth;
pub mod keep_awake;
pub mod wifi;

pub use action::{action_tile, svg};
pub use battery::BatteryBuilder;
pub use bluetooth::BluetoothBuilder;
pub use keep_awake::KeepAwakeTileBuilder;
pub use wifi::WifiBuilder;

pub(crate) const MAX_TILE_HEIGHT: f32 = 106.0;
pub(crate) const DOT_RADIUS: f32 = 5.0;

/// Preferred icon/label spacing; the spacer yields before either content item.
const ICON_LABEL_GAP: f32 = 10.0;

/// Starts hidden; each tile controls visibility alongside its label.
pub(super) fn label_spacer<W: Widget>(me: Handle<W>, s: &mut Spawner<'_, W>) -> Handle<Div> {
    s.spawn(
        me,
        div().style(
            LayoutStyle::default()
                .height(px(ICON_LABEL_GAP))
                .min_height(px(0.0))
                .shrink(1.0)
                .hidden(),
        ),
    )
}

/// Border width per side of a tile, passed to every tile builder.
pub type Borders = Insets<f32>;

/// Common [`Borders`] presets.
pub mod borders {
    use super::Borders;

    const W: f32 = 1.0;
    /// Every side.
    pub const ALL: Borders = Borders::new(W, W, W, W);
    /// Every side but the left, for tiles on the left screen edge.
    pub const NO_LEFT: Borders = Borders::new(W, W, W, 0.0);
    /// Every side but the right, for tiles on the right screen edge.
    pub const NO_RIGHT: Borders = Borders::new(W, 0.0, W, W);
}

/// A tile's background quad. Tiles that repaint call this with their stored
/// [`Borders`] so the per-side widths survive.
pub(super) fn tile_quad(background: Color, border_color: Color, borders: Borders) -> Quad {
    Quad::new(background)
        .border(1.0, border_color)
        .border_widths(borders)
}

/// Applies the frame every tile shares (size, column layout, border) to `me`.
pub(super) fn frame<W: Widget>(me: Handle<W>, s: &mut Spawner<'_, W>, borders: Borders) {
    *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
        .height(percent(100.0))
        .flex_basis(px(0.0))
        .grow(1.0)
        .shrink(0.0)
        .min_width(px(0.0))
        .max_width(percent(20.0))
        .column()
        .center();
    let border_color = s.color(ColorRole::SecondaryFixedDim);
    *s.component_mut::<Paint>(me).unwrap() =
        Paint::Quad(tile_quad(Color::TRANSPARENT, border_color, borders));
}

/// Frames `me` as a tile and adds a one-colour icon tinted with OnSurface.
/// The returned `Icon` can be updated with `set_sprite` / `set_color`.
pub(super) fn build_tile<W: Widget>(
    me: Handle<W>,
    s: &mut Spawner<'_, W>,
    svg: &'static [u8],
    borders: Borders,
) -> Handle<Icon> {
    frame(me, s, borders);
    let sprite = load_icon(&mut s.resource_mut::<Atlas>(), svg);
    s.spawn(
        me,
        icon(sprite)
            .style(LayoutStyle::default().shrink(0.0))
            .color(s.color(ColorRole::OnSurface)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_wayland::mechanix_widgets::prelude::FontBook;
    use services::{
        APUpdate, BatteryPercentageChanged, BluetoothState, BluetoothStateChanged, StateUpdate,
        WifiState,
    };

    #[test]
    fn label_spacers_yield_to_contents_and_disappear_with_hidden_labels() {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(WindowModule)
            .add_module(MechanixTheme::dark());
        let mut atlas = Atlas::new();
        let font = atlas.add_font(assets::fonts::GEIST_MONO).unwrap();
        app.insert_resource(atlas);
        app.insert_resource(FontBook::new(font));
        let size = |height| LayoutStyle::default().row().size(px(308.0), px(height));
        let window = app.spawn(app.root(), window().layout(size(106.0)));
        let wifi = app.spawn(
            window,
            WifiBuilder {
                borders: borders::ALL,
            },
        );
        let bluetooth = app.spawn(
            window,
            BluetoothBuilder {
                borders: borders::ALL,
            },
        );
        let battery = app.spawn(
            window,
            BatteryBuilder {
                borders: borders::ALL,
            },
        );
        app.emit(
            APUpdate {
                ssid: Some(b"Network".to_vec()),
                strength: 80,
            },
            wifi,
        );
        app.emit(
            StateUpdate {
                state: WifiState::Connected,
            },
            wifi,
        );
        app.emit(
            BluetoothStateChanged {
                state: BluetoothState::Connected {
                    count: 1,
                    name: Some("Speaker".into()),
                },
            },
            bluetooth,
        );
        app.emit(
            BatteryPercentageChanged {
                percentage: Some(42),
            },
            battery,
        );
        let settle = |app: &mut App| {
            for _ in 0..3 {
                app.tick();
            }
        };
        settle(&mut app);
        let rect = |app: &App, id| app.component::<ComputedLayout>(id).unwrap().rect;
        for tile in [wifi.id(), bluetooth.id(), battery.id()] {
            let children = app.children(tile).unwrap();
            let (icon, spacer, label) = (children[0], children[1], children[2]);
            let label_height = rect(&app, label).height();
            let minimum = 32.0 + label_height;
            for height in [106.0, 57.0, minimum, 106.0] {
                *app.component_mut::<LayoutStyle>(window).unwrap() = size(height);
                settle(&mut app);
                let tile = rect(&app, tile);
                let icon = rect(&app, icon);
                let gap = rect(&app, spacer);
                let label = rect(&app, label);
                assert_eq!(icon.height(), 32.0);
                assert_eq!(label.height(), label_height);
                let expected_gap = (height - minimum).clamp(0.0, ICON_LABEL_GAP);
                assert!((gap.height() - expected_gap).abs() <= 1.0, "{gap:?}");
                assert!(icon.y() >= tile.y(), "{tile:?} {icon:?}");
                assert!(icon.bottom() <= label.y(), "{icon:?} {label:?}");
                assert!(label.bottom() <= tile.bottom(), "{tile:?} {label:?}");
                let above = icon.y() - tile.y();
                let below = tile.bottom() - label.bottom();
                assert!((above - below).abs() <= 1.0);
            }
        }
        for connected in [false, true, false] {
            app.emit(
                StateUpdate {
                    state: if connected {
                        WifiState::Connected
                    } else {
                        WifiState::Disconnected
                    },
                },
                wifi,
            );
            app.emit(
                BluetoothStateChanged {
                    state: if connected {
                        BluetoothState::Connected {
                            count: 1,
                            name: Some("Speaker".into()),
                        }
                    } else {
                        BluetoothState::On
                    },
                },
                bluetooth,
            );
            app.emit(
                BatteryPercentageChanged {
                    percentage: connected.then_some(42),
                },
                battery,
            );
            settle(&mut app);
            for tile in [wifi.id(), bluetooth.id(), battery.id()] {
                let children = app.children(tile).unwrap();
                for &child in &children[1..=2] {
                    assert_eq!(
                        app.component::<LayoutStyle>(child).unwrap().display,
                        if connected {
                            Display::Flex
                        } else {
                            Display::Hidden
                        }
                    );
                }
                if !connected {
                    let tile_rect = rect(&app, tile);
                    let icon = rect(&app, children[0]);
                    assert!(
                        (icon.y() - tile_rect.y() - (tile_rect.height() - icon.height()) / 2.0)
                            .abs()
                            <= 1.0
                    );
                }
            }
        }
    }
}
