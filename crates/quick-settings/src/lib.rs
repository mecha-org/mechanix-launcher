use mecha_wayland::prelude::*;
use screen::Screen;

mod bottom_bar;
mod header;
mod power_overlay;
mod quick_settings;
mod sliders;
mod tiles;
mod util;
mod volume_keys;

use bottom_bar::BottomBarBuilder;
use header::HeaderBuilder;
use power_overlay::PowerOverlayBuilder;
use quick_settings::QuickSettingsBuilder;
use services::{BluetoothModule, DisplayModule, NetworkManagerModule, PowerModule, UpowerModule};
use sliders::SlidersBuilder;
use volume_keys::VolumeKeysModule;

/// Installs the quick settings' services (UPower, Bluetooth, display
/// brightness and logind power) and the volume keys. It spawns no window;
/// place the panel with [`QuickSettingsScreen`].
///
/// Expects the host to have installed `RingModule`, `PulseAudioModule` and
/// `VolumeControlModule` (for the volume slider and keys), `WaylandModule`
/// (before this module, for the volume keys), `TimerModule` (for the header's
/// date) and `DBusModule::new().system()`, and to have inserted the [`Atlas`]
/// and `FontBook`.
pub struct QuickSettingsModule;

impl Module for QuickSettingsModule {
    fn install(self, app: &mut App) {
        app.add_module(UpowerModule)
            .add_module(NetworkManagerModule)
            .add_module(BluetoothModule)
            .add_module(DisplayModule)
            .add_module(PowerModule)
            .add_module(VolumeKeysModule);
    }
}

/// The quick settings panel as content: builds the panel and its power
/// overlay under any parent node.
pub struct QuickSettingsScreen;

impl Screen for QuickSettingsScreen {
    fn build(&self, app: &mut App, parent: NodeId) -> NodeId {
        let root = app.root();
        app.spawn(parent, ShellBuilder { root }).id()
    }
}

struct Shell;

struct ShellBuilder {
    root: NodeId,
}

impl Build for ShellBuilder {
    type Widget = Shell;
}

impl Widget for Shell {
    type Builder = ShellBuilder;

    fn build(b: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .width(percent(100.0))
            .height(percent(100.0));
        *s.component_mut::<Paint>(me).unwrap() = Paint::Quad(Quad::new(Color::BLACK));

        let content = s.spawn(
            me,
            QuickSettingsBuilder {
                header: HeaderBuilder,
                bottom_bar: BottomBarBuilder,
                sliders: SlidersBuilder,
            },
        );
        s.spawn(
            me,
            PowerOverlayBuilder {
                root: b.root,
                content,
            },
        );
        Shell
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::power_overlay::PowerOverlay;
    use crate::quick_settings::QuickSettings;
    use mecha_wayland::mechanix_widgets::prelude::{Clock, ClockInterval, FontBook, Text};

    #[test]
    fn screen_builds_panel_and_overlay_without_a_window() {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(WindowModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark())
            .add_module(RingModule::default())
            .add_module(TimerModule);
        let mut atlas = Atlas::new();
        let font = atlas.add_font(assets::fonts::GEIST_MONO).unwrap();
        app.insert_resource(atlas);
        app.insert_resource(FontBook::new(font));

        let root = app.root();
        let shell = QuickSettingsScreen.build(&mut app, root);
        app.tick();

        assert!(app.widget::<Shell>(shell).is_some());
        assert_eq!(app.widgets::<QuickSettings>().count(), 1);
        assert_eq!(app.widgets::<PowerOverlay>().count(), 1);
        assert_eq!(app.resource::<Windows>().iter().count(), 0);

        let (interval, date_text) = {
            let (_, date) = app
                .widgets::<Clock>()
                .next()
                .expect("header shows the date");
            (date.interval(), date.text())
        };
        assert_eq!(interval, ClockInterval::Second);
        let native = app.widget::<Text>(date_text).unwrap().native();
        let label = &app
            .widget::<mecha_wayland::prelude::Text>(native)
            .unwrap()
            .string;
        // DATE_FORMAT, e.g. "Monday,2 Nov".
        let (weekday, day_month) = label.split_once(',').expect(label);
        assert!(weekday.ends_with("day"), "{label}");
        let (day, month) = day_month.split_once(' ').expect(label);
        assert!(day.parse::<u8>().is_ok() && month.len() == 3, "{label}");
    }
}
