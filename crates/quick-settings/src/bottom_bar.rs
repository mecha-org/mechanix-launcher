use mecha_wayland::prelude::*;

use crate::tiles::{
    BatteryBuilder, BluetoothBuilder, KeepAwakeTileBuilder, MAX_TILE_HEIGHT, WifiBuilder,
    action_tile, borders, svg,
};

const BOTTOM_BAR_HEIGHT: f32 = MAX_TILE_HEIGHT * 2.0;
// In portrait, leave room in the body for the sliders as the panel gets shorter.
const PORTRAIT_BODY_PERCENT: f32 = 55.0;

pub struct BottomBar;

pub struct BottomBarBuilder;

impl Build for BottomBarBuilder {
    type Widget = BottomBar;
}

impl BottomBar {
    /// Portrait: full width under the sliders.
    /// Landscape: half the width, beside the sliders.
    pub fn style(landscape: bool) -> LayoutStyle {
        let style = LayoutStyle::default().column().shrink(0.0);
        if landscape {
            style
                .height(px(BOTTOM_BAR_HEIGHT))
                .grow(1.0)
                .flex_basis(px(0.0))
                .min_width(px(0.0))
        } else {
            style
                .width(percent(100.0))
                .height(percent(PORTRAIT_BODY_PERCENT))
                .max_height(px(BOTTOM_BAR_HEIGHT))
        }
    }
}

impl Widget for BottomBar {
    type Builder = BottomBarBuilder;

    fn build(_: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = BottomBar::style(false);

        let row = LayoutStyle::default()
            .height(percent(50.0))
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
        s.spawn(
            top,
            KeepAwakeTileBuilder {
                borders: borders::ALL,
            },
        );
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
    fn portrait_rows_follow_the_bar_height_and_keep_their_tiles_inside() {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(WindowModule)
            .add_module(MechanixTheme::dark());
        let mut atlas = Atlas::new();
        let font = atlas.add_font(assets::fonts::GEIST_MONO).unwrap();
        app.insert_resource(atlas);
        app.insert_resource(FontBook::new(font));

        let size = |height| LayoutStyle::default().size(px(308.0), px(height));
        let window = app.spawn(app.root(), window().layout(size(878.0)));
        let bar = app.spawn(window, BottomBarBuilder);
        for (parent_height, expected_height) in [(878.0, 212.0), (150.0, 90.0)] {
            *app.component_mut::<LayoutStyle>(window).unwrap() = size(parent_height);
            app.tick();

            let rect = |id| app.component::<ComputedLayout>(id).unwrap().rect;
            let bar_rect = rect(bar.id());
            assert!(
                (bar_rect.height() - expected_height).abs() <= 1.0,
                "{bar_rect:?}"
            );
            let rows = app.children(bar.id()).unwrap();
            let top = rect(rows[0]);
            let bottom = rect(rows[1]);
            assert!(top.height() > 0.0 && bottom.height() > 0.0);
            assert!(top.origin.y + top.height() <= bottom.origin.y);
            assert!(bottom.origin.y + bottom.height() <= bar_rect.origin.y + bar_rect.height());
            for &row in rows {
                let row_rect = rect(row);
                for &tile in app.children(row).unwrap() {
                    let tile_rect = rect(tile);
                    assert_eq!(tile_rect.origin.y, row_rect.origin.y);
                    assert_eq!(tile_rect.height(), row_rect.height());
                }
            }
        }
    }

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
    fn battery_service_updates_bottom_bar_label_and_icon_colour() {
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

        let (battery_id, battery) = app.widgets::<Battery>().next().unwrap();
        let percentage = battery.percentage;
        let icon = battery.icon;
        let spacer = app.children(battery_id).unwrap()[1];
        let assert_state = |app: &App, label: &str, display, colour| {
            let native = app.widget::<Text>(percentage).unwrap().native();
            assert_eq!(app.widget::<NativeText>(native).unwrap().string, label);
            for id in [percentage.id(), spacer] {
                assert_eq!(app.component::<LayoutStyle>(id).unwrap().display, display);
            }
            assert_eq!(app.widget::<Icon>(icon).unwrap().color(), app.color(colour));
        };
        assert_state(
            &app,
            "",
            Display::Hidden,
            ColorRole::OnSecondaryFixedVariant,
        );
        for (value, label) in [(Some(42), "42%"), (None, ""), (Some(0), "0%"), (None, "")] {
            app.emit(
                BatteryPercentageChanged { percentage: value },
                app.descendants(app.root()).collect::<Vec<_>>(),
            );
            app.tick();
            let (display, colour) = if value.is_some() {
                (Display::Flex, ColorRole::OnSurface)
            } else {
                (Display::Hidden, ColorRole::OnSecondaryFixedVariant)
            };
            assert_state(&app, label, display, colour);
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
