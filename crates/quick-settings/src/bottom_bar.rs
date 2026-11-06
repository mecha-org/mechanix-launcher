use mecha_wayland::prelude::*;

use crate::tiles::{
    BatteryBuilder, BluetoothBuilder, TILE_HEIGHT, WifiBuilder, action_tile, borders, svg,
};

const BOTTOM_BAR_HEIGHT: f32 = TILE_HEIGHT * 2.0;

pub struct BottomBar;

pub struct BottomBarBuilder;

impl Build for BottomBarBuilder {
    type Widget = BottomBar;
}

impl Widget for BottomBar {
    type Builder = BottomBarBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .height(px(BOTTOM_BAR_HEIGHT))
            .column()
            .shrink(0.0);

        let row = LayoutStyle::default()
            .height(px(TILE_HEIGHT))
            .row()
            .shrink(0.0);
        let top = s.spawn(me, div().style(row.clone()));
        // Edge tiles drop the border on their screen-edge side.
        s.spawn(
            top,
            WifiBuilder {
                borders: borders::NO_LEFT,
            },
        );
        s.spawn(
            top,
            BluetoothBuilder {
                borders: borders::ALL,
            },
        );
        s.spawn(top, action_tile(svg::DISPLAY, borders::ALL));
        s.spawn(top, action_tile(svg::TERMINAL, borders::ALL));
        s.spawn(top, action_tile(svg::MICROPHONE, borders::NO_RIGHT));

        let bottom = s.spawn(me, div().style(row));
        s.spawn(bottom, action_tile(svg::RECORD, borders::NO_LEFT));
        s.spawn(bottom, action_tile(svg::SETTINGS, borders::ALL));
        s.spawn(bottom, action_tile(svg::CAMERA, borders::ALL));
        s.spawn(bottom, action_tile(svg::ROTATE, borders::ALL));
        s.spawn(
            bottom,
            BatteryBuilder {
                borders: borders::NO_RIGHT,
            },
        );

        BottomBar
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tiles::{battery::Battery, bluetooth::Bluetooth};
    use mecha_wayland::mechanix_widgets::NativeText;
    use mecha_wayland::mechanix_widgets::prelude::{FontBook, Text};
    use services::{BatteryPercentageChanged, BluetoothState, BluetoothStateChanged};

    #[test]
    fn edge_tiles_drop_their_screen_edge_border() {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark());
        let mut atlas = Atlas::new();
        let font = atlas.add_font(assets::fonts::GEIST_MONO).unwrap();
        app.insert_resource(atlas);
        app.insert_resource(FontBook::new(font));
        let bar = app.spawn(app.root(), BottomBarBuilder);
        app.tick();

        let border = |app: &App, tile: NodeId| match app.component::<Paint>(tile).unwrap() {
            Paint::Quad(quad) => quad.border,
            other => panic!("tile is not a quad: {other:?}"),
        };
        for row in app.children(bar.id()).unwrap().to_vec() {
            let tiles = app.children(row).unwrap().to_vec();
            let (first, last) = (tiles[0], *tiles.last().unwrap());
            assert_eq!(border(&app, first), borders::NO_LEFT);
            assert_eq!(border(&app, last), borders::NO_RIGHT);
            for &tile in &tiles[1..tiles.len() - 1] {
                assert_eq!(border(&app, tile), borders::ALL);
            }
        }

        // Wi-Fi repaints on state changes; the trimmed side must survive that.
        let wifi = app.children(app.children(bar.id()).unwrap()[0]).unwrap()[0];
        app.emit(
            services::StateUpdate {
                state: services::WifiState::Connected,
            },
            app.descendants(app.root()).collect::<Vec<_>>(),
        );
        app.tick();
        assert_eq!(border(&app, wifi), borders::NO_LEFT);
    }

    #[test]
    fn battery_service_updates_bottom_bar_label() {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark());
        let mut atlas = Atlas::new();
        let font = atlas.add_font(assets::fonts::GEIST_MONO).unwrap();
        app.insert_resource(atlas);
        app.insert_resource(FontBook::new(font));
        app.spawn(app.root(), BottomBarBuilder);
        app.tick();

        let (_, battery) = app.widgets::<Battery>().next().unwrap();
        let percentage = battery.percentage;
        for (value, label) in [(Some(42), "42%"), (None, "N/A")] {
            app.emit(
                BatteryPercentageChanged { percentage: value },
                app.descendants(app.root()).collect::<Vec<_>>(),
            );
            app.tick();
            let native = app.widget::<Text>(percentage).unwrap().native();
            assert_eq!(app.widget::<NativeText>(native).unwrap().string, label);
        }
    }

    #[test]
    fn bluetooth_service_updates_bottom_bar_tile() {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark());
        let mut atlas = Atlas::new();
        let font = atlas.add_font(assets::fonts::GEIST_MONO).unwrap();
        app.insert_resource(atlas);
        app.insert_resource(FontBook::new(font));
        app.spawn(app.root(), BottomBarBuilder);
        app.tick();

        let (_, bluetooth) = app.widgets::<Bluetooth>().next().unwrap();
        let text_handle = bluetooth.name;
        let dot_handle = bluetooth.dot;

        for (state, label, is_on) in [
            (BluetoothState::On, None, true),
            (
                BluetoothState::Connected {
                    count: 1,
                    name: Some("Headphones".into()),
                },
                Some("Headp..."),
                true,
            ),
            (
                BluetoothState::Connected {
                    count: 1,
                    name: None,
                },
                Some("Connected"),
                true,
            ),
            (
                BluetoothState::Connected {
                    count: 2,
                    name: None,
                },
                Some("2 devices"),
                true,
            ),
            (BluetoothState::Off, None, false),
        ] {
            app.emit(
                BluetoothStateChanged { state },
                app.descendants(app.root()).collect::<Vec<_>>(),
            );
            app.tick();
            let display = app
                .component::<LayoutStyle>(text_handle.id())
                .unwrap()
                .display;
            match label {
                Some(label) => {
                    assert_eq!(display, Display::Flex);
                    let native = app.widget::<Text>(text_handle).unwrap().native();
                    assert_eq!(app.widget::<NativeText>(native).unwrap().string, label);
                }
                None => assert_eq!(display, Display::Hidden),
            }
            let dot_paint = app.component::<Paint>(dot_handle.id()).unwrap();
            if is_on {
                assert!(matches!(dot_paint, Paint::Quad(_)));
            } else {
                assert_eq!(dot_paint, &Paint::None);
            }
        }
    }
}
