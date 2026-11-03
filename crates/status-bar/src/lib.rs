pub mod indicators;
mod module;
pub mod status_bar;
pub use services::{BluetoothModule, BluetoothStatus, NetworkManagerModule, UpowerModule};

use mecha_wayland::prelude::*;

pub use indicators::*;
pub use module::{StatusBarModule, StatusBarScreen};
pub use status_bar::{StatusBar, StatusBarBuilder, status_bar};

/// Bundled GeistMono font for status bar numeric text (such as battery percentage).
pub use assets::fonts::GEIST_MONO as GEIST_MONO_BYTES;

/// Default icon dimension in pixels (matches 20x20 design specification).
pub const DEFAULT_ICON_SIZE: u32 = 20;

/// Logical px per mm on the Nothing Phone (4a) panel: 1224x2720 px over
/// 71x157 mm, shown at output scale 2.25.
const PX_PER_MM_X: f32 = 1224.0 / 71.0 / 2.25;
const PX_PER_MM_Y: f32 = 2720.0 / 157.0 / 2.25;

/// Where the bar's icons sit, in mm from the screen edges: their bottom
/// edge this far below the top, the clusters this far in from the sides.
const EDGE_INSET_MM: f32 = 7.0;

/// Horizontal inset of the bar's clusters, logical px.
pub const SIDE_INSET: f32 = EDGE_INSET_MM * PX_PER_MM_X;

/// Status bar height, logical px: the icons sit on its bottom edge, so
/// they end exactly [`EDGE_INSET_MM`] below the top of the screen.
pub const DEFAULT_BAR_HEIGHT: f32 = EDGE_INSET_MM * PX_PER_MM_Y;

/// Pre-rasterized sprite handles for all status bar icons in the Atlas.
#[derive(Debug, Clone, Copy)]
pub struct StatusIcons {
    pub airplane: AirplaneIcons,
    pub bluetooth: BluetoothIcons,
    pub cellular: CellularIcons,
    pub wifi: WifiIcons,
}

impl Resource for StatusIcons {}

impl StatusIcons {
    /// Rasterizes all embedded indicator SVGs at `master` pixel dimension (defaults to 20px)
    /// and inserts them into the Atlas as [`Class::Icon`] coverage tiles.
    pub fn load(atlas: &mut Atlas, master: u32) -> Result<Self, Error> {
        Ok(Self {
            airplane: AirplaneIcons::load(atlas, master)?,
            bluetooth: BluetoothIcons::load(atlas, master)?,
            cellular: CellularIcons::load(atlas, master)?,
            wifi: WifiIcons::load(atlas, master)?,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_wayland::mechanix_widgets::prelude::FontBook;

    #[test]
    fn test_load_status_icons() {
        let mut atlas = Atlas::new();
        let icons = StatusIcons::load(&mut atlas, DEFAULT_ICON_SIZE)
            .expect("All indicator SVGs must parse and pack into atlas");
        assert_eq!(icons.airplane.icon.0, 0);
    }

    #[test]
    fn test_status_bar_spawn() {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark())
            .insert_resource(Atlas::new());

        let font = app
            .resource_mut::<Atlas>()
            .add_font(GEIST_MONO_BYTES)
            .expect("GeistMono loads");
        app.insert_resource(FontBook::new(font));

        let icons = StatusIcons::load(&mut app.resource_mut::<Atlas>(), DEFAULT_ICON_SIZE)
            .expect("icons load");
        app.insert_resource(icons);

        let root = app.root();
        let bar = app.spawn(root, status_bar());
        app.tick();

        let bar_widget = app.widget::<StatusBar>(bar).unwrap();
        assert_eq!(
            app.widget::<GlyphIndicator>(bar_widget.glyph)
                .unwrap()
                .state(),
            GlyphState::default()
        );
        assert_eq!(
            app.widget::<WifiIndicator>(bar_widget.wifi)
                .unwrap()
                .state(),
            WifiState::Disabled
        );
        assert_eq!(
            app.widget::<BatteryPercentage>(bar_widget.battery)
                .unwrap()
                .percentage(),
            None
        );
        assert_eq!(
            app.widget::<BluetoothIndicator>(bar_widget.bluetooth)
                .unwrap()
                .state(),
            BluetoothState::Off
        );
        assert_eq!(
            app.widget::<CellularIndicator>(bar_widget.cellular)
                .unwrap()
                .state(),
            CellularState::Disabled
        );
        assert_eq!(
            app.widget::<AirplaneIndicator>(bar_widget.airplane)
                .unwrap()
                .state(),
            AirplaneState::Disabled
        );
    }

    #[test]
    fn test_indicator_visibility_rules() {
        assert!(!AirplaneState::Disabled.is_visible());
        assert!(AirplaneState::Enabled.is_visible());

        assert!(!BluetoothState::Off.is_visible());
        assert!(BluetoothState::On.is_visible());
        assert!(BluetoothState::Connected.is_visible());

        assert!(!WifiState::Disabled.is_visible());
        assert!(WifiState::High.is_visible());
        assert!(WifiState::Medium.is_visible());
        assert!(WifiState::Low.is_visible());
        assert!(WifiState::None.is_visible());

        assert!(!CellularState::Disabled.is_visible());
        assert!(!CellularState::None.is_visible());
        assert!(CellularState::Full.is_visible());
        assert!(CellularState::High.is_visible());
    }

    #[test]
    fn test_dynamic_visibility_transitions() {
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        struct ToggleTest;
        impl Event for ToggleTest {}

        struct TestHarness {
            bar: Handle<StatusBar>,
        }
        impl Build for TestHarness {
            type Widget = TestHarness;
        }
        impl Widget for TestHarness {
            type Builder = TestHarness;
            fn build(b: Self, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
                let bar = b.bar;
                s.on::<ToggleTest>(me, move |ctx, _| {
                    let bar_w = *ctx.widget::<StatusBar>(bar).unwrap();
                    ctx.at(bar_w.bluetooth)
                        .unwrap()
                        .set_state(BluetoothState::Off);
                    ctx.at(bar_w.wifi).unwrap().set_state(WifiState::Disabled);
                    ctx.at(bar_w.cellular)
                        .unwrap()
                        .set_state(CellularState::None);
                    ctx.at(bar_w.airplane)
                        .unwrap()
                        .set_state(AirplaneState::Enabled);
                });
                b
            }
        }

        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark())
            .insert_resource(Atlas::new());

        let font = app
            .resource_mut::<Atlas>()
            .add_font(GEIST_MONO_BYTES)
            .expect("GeistMono loads");
        app.insert_resource(FontBook::new(font));

        let icons = StatusIcons::load(&mut app.resource_mut::<Atlas>(), DEFAULT_ICON_SIZE)
            .expect("icons load");
        app.insert_resource(icons);

        let root = app.root();
        let bar = app.spawn(root, status_bar());
        let harness = app.spawn(root, TestHarness { bar });
        app.tick();

        let bar_widget = *app.widget::<StatusBar>(bar).unwrap();
        app.emit(
            ::services::BluetoothStateChanged {
                state: ::services::BluetoothState::On,
            },
            bar,
        );
        app.emit(
            ::services::StateUpdate {
                state: ::services::WifiState::Connected,
            },
            bar,
        );
        app.emit(
            ::services::APUpdate {
                ssid: Some(b"Office".to_vec()),
                strength: 100,
            },
            bar,
        );
        app.tick();

        // Initially:
        // Airplane is Disabled (Hidden)
        // Bluetooth is On (Visible)
        // Cellular is Disabled (Hidden)
        // Wifi is High (Visible)
        assert_eq!(
            app.component::<LayoutStyle>(bar_widget.airplane.id())
                .unwrap()
                .display,
            Display::Hidden
        );
        assert_ne!(
            app.component::<LayoutStyle>(bar_widget.bluetooth.id())
                .unwrap()
                .display,
            Display::Hidden
        );
        assert_ne!(
            app.component::<LayoutStyle>(bar_widget.wifi.id())
                .unwrap()
                .display,
            Display::Hidden
        );
        assert_eq!(
            app.component::<LayoutStyle>(bar_widget.cellular.id())
                .unwrap()
                .display,
            Display::Hidden
        );

        // Emit toggle event to update indicators via Context::at(...)
        app.emit(ToggleTest, harness.id());
        app.tick();

        // Now:
        // Bluetooth (Off) -> Hidden
        // Wifi (Disabled) -> Hidden
        // Cellular (None) -> Hidden
        // Airplane (Enabled) -> Visible
        assert_eq!(
            app.component::<LayoutStyle>(bar_widget.bluetooth.id())
                .unwrap()
                .display,
            Display::Hidden
        );
        assert_eq!(
            app.component::<LayoutStyle>(bar_widget.wifi.id())
                .unwrap()
                .display,
            Display::Hidden
        );
        assert_eq!(
            app.component::<LayoutStyle>(bar_widget.cellular.id())
                .unwrap()
                .display,
            Display::Hidden
        );
        assert_ne!(
            app.component::<LayoutStyle>(bar_widget.airplane.id())
                .unwrap()
                .display,
            Display::Hidden
        );
    }

    #[test]
    fn test_battery_percentage_label() {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark())
            .insert_resource(Atlas::new());

        let font = app
            .resource_mut::<Atlas>()
            .add_font(GEIST_MONO_BYTES)
            .expect("GeistMono loads");
        app.insert_resource(FontBook::new(font));

        let icons = StatusIcons::load(&mut app.resource_mut::<Atlas>(), DEFAULT_ICON_SIZE)
            .expect("icons load");
        app.insert_resource(icons);

        let root = app.root();
        let bar = app.spawn(root, status_bar());
        app.tick();

        let bar_widget = *app.widget::<StatusBar>(bar).unwrap();

        // Unavailable and empty are distinct; updates can restore a percentage.
        for (input, expected, label) in [
            (None, None, "N/A"),
            (Some(0), Some(0), "0%"),
            (Some(255), Some(100), "100%"),
            (None, None, "N/A"),
            (Some(42), Some(42), "42%"),
        ] {
            app.emit(
                ::services::BatteryPercentageChanged { percentage: input },
                bar,
            );
            app.tick();

            let battery = app.widget::<BatteryPercentage>(bar_widget.battery).unwrap();
            assert_eq!(battery.percentage(), expected);
            let text = app
                .widget::<mecha_wayland::mechanix_widgets::prelude::Text>(battery.text_node)
                .unwrap();
            let native = app
                .widget::<mecha_wayland::prelude::Text>(text.native())
                .unwrap();
            assert_eq!(native.string, label);
        }
    }

    #[test]
    fn test_shared_service_events() {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark())
            .insert_resource(Atlas::new());

        let font = app
            .resource_mut::<Atlas>()
            .add_font(GEIST_MONO_BYTES)
            .expect("GeistMono loads");
        app.insert_resource(FontBook::new(font));

        let icons = StatusIcons::load(&mut app.resource_mut::<Atlas>(), DEFAULT_ICON_SIZE)
            .expect("icons load");
        app.insert_resource(icons);

        let root = app.root();
        let bar = app.spawn(root, status_bar());
        app.tick();
        let bar_widget = *app.widget::<StatusBar>(bar).unwrap();

        for (state, expected) in [
            (::services::BluetoothState::On, BluetoothState::On),
            (
                ::services::BluetoothState::Connected {
                    count: 2,
                    name: None,
                },
                BluetoothState::Connected,
            ),
            (::services::BluetoothState::Off, BluetoothState::Off),
        ] {
            app.emit(::services::BluetoothStateChanged { state }, bar);
            app.tick();
            assert_eq!(
                app.widget::<BluetoothIndicator>(bar_widget.bluetooth)
                    .unwrap()
                    .state(),
                expected
            );
        }

        // Strength only shows once NetworkManager reports a connection.
        app.emit(
            ::services::APUpdate {
                ssid: Some(b"Office".to_vec()),
                strength: 100,
            },
            bar,
        );
        app.tick();
        assert_eq!(
            app.widget::<WifiIndicator>(bar_widget.wifi)
                .unwrap()
                .state(),
            WifiState::Disabled
        );
        app.emit(
            ::services::StateUpdate {
                state: ::services::WifiState::Connected,
            },
            bar,
        );
        for (strength, expected) in [
            (0, WifiState::None),
            (24, WifiState::None),
            (25, WifiState::Low),
            (49, WifiState::Low),
            (50, WifiState::Medium),
            (74, WifiState::Medium),
            (75, WifiState::High),
        ] {
            app.emit(
                ::services::APUpdate {
                    ssid: Some(b"Office".to_vec()),
                    strength,
                },
                bar,
            );
            app.tick();
            assert_eq!(
                app.widget::<WifiIndicator>(bar_widget.wifi)
                    .unwrap()
                    .state(),
                expected
            );
        }
        for (state, expected) in [
            (::services::WifiState::Disconnected, WifiState::Disconnected),
            (::services::WifiState::Connecting, WifiState::Disconnected),
            (::services::WifiState::Connected, WifiState::High),
            (::services::WifiState::Off, WifiState::Disabled),
            (::services::WifiState::Unavailable, WifiState::Disabled),
        ] {
            app.emit(::services::StateUpdate { state }, bar);
            app.tick();
            assert_eq!(
                app.widget::<WifiIndicator>(bar_widget.wifi)
                    .unwrap()
                    .state(),
                expected
            );
        }

        for percentage in [Some(42), None] {
            app.emit(::services::BatteryPercentageChanged { percentage }, bar);
            app.tick();
            assert_eq!(
                app.widget::<BatteryPercentage>(bar_widget.battery)
                    .unwrap()
                    .percentage(),
                percentage
            );
        }
    }

    #[test]
    fn test_glyph_state_and_widget() {
        // Test state representation and grid conversion
        // Row 0: [●, ●, ●, ○]
        // Row 1: [●, ○, ●, ●]
        let grid = [[true, true, true, false], [true, false, true, true]];
        let state = GlyphState::from(grid);
        assert_eq!(state, GlyphState::DEMO);
        assert_eq!(<[[bool; 4]; 2]>::from(state), grid);
        assert!(state.is_filled_2d(0, 0));
        assert!(state.is_filled_2d(0, 1));
        assert!(state.is_filled_2d(0, 2));
        assert!(!state.is_filled_2d(0, 3));
        assert!(state.is_filled_2d(1, 0));
        assert!(!state.is_filled_2d(1, 1));
        assert!(state.is_filled_2d(1, 2));
        assert!(state.is_filled_2d(1, 3));

        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark());

        let root = app.root();
        let glyph = app.spawn(root, glyph_indicator().state(state));
        app.tick();

        let widget = *app.widget::<GlyphIndicator>(glyph).unwrap();
        assert_eq!(widget.state(), GlyphState::DEMO);
        assert_eq!(widget.dots.len(), 8);

        // Verify the 8 dots exist and have paint components
        for &dot in &widget.dots {
            assert!(app.is_live(dot.id()));
            assert!(app.component::<Paint>(dot.id()).is_some());
        }
    }
}
