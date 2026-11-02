use mecha_wayland::prelude::*;

use crate::tiles::{
    BatteryBuilder, BluetoothBuilder, CameraBuilder, DisplayTileBuilder, MicrophoneBuilder,
    RecordBuilder, RotateBuilder, SettingsBuilder, TILE_HEIGHT, TerminalBuilder, WifiBuilder,
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
        s.spawn(
            top,
            WifiBuilder {
                name: "Office 01".into(),
            },
        );
        s.spawn(top, BluetoothBuilder);
        s.spawn(top, DisplayTileBuilder);
        s.spawn(top, TerminalBuilder);
        s.spawn(top, MicrophoneBuilder);

        let bottom = s.spawn(me, div().style(row));
        s.spawn(bottom, RecordBuilder);
        s.spawn(bottom, SettingsBuilder);
        s.spawn(bottom, CameraBuilder);
        s.spawn(bottom, RotateBuilder);
        s.spawn(bottom, BatteryBuilder);

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
            (BluetoothState::On, "On", true),
            (
                BluetoothState::Connected {
                    count: 1,
                    name: Some("Headphones".into()),
                },
                "Hea...",
                true,
            ),
            (
                BluetoothState::Connected {
                    count: 1,
                    name: None,
                },
                "Connected",
                true,
            ),
            (
                BluetoothState::Connected {
                    count: 2,
                    name: None,
                },
                "2 devices",
                true,
            ),
            (BluetoothState::Off, "Off", false),
        ] {
            app.emit(
                BluetoothStateChanged { state },
                app.descendants(app.root()).collect::<Vec<_>>(),
            );
            app.tick();
            let native = app.widget::<Text>(text_handle).unwrap().native();
            assert_eq!(app.widget::<NativeText>(native).unwrap().string, label);
            let dot_paint = app.component::<Paint>(dot_handle.id()).unwrap();
            if is_on {
                assert!(matches!(dot_paint, Paint::Quad(_)));
            } else {
                assert_eq!(dot_paint, &Paint::None);
            }
        }
    }
}
