use ::services::{APUpdate, BatteryPercentageChanged, BluetoothStateChanged, StateUpdate};
use mecha_wayland::mechanix_widgets::prelude::{Clock, ClockInterval, clock};
use mecha_wayland::prelude::*;

use crate::indicators::*;
use crate::{DEFAULT_BAR_HEIGHT, StatusIcons};

/// 24-hour time, without seconds.
pub const CLOCK_FORMAT: &str = "%H:%M";

#[derive(Debug, Clone, Copy)]
pub struct StatusBar {
    pub glyph: Handle<GlyphIndicator>,
    pub clock: Handle<Clock>,
    pub airplane: Handle<AirplaneIndicator>,
    pub bluetooth: Handle<BluetoothIndicator>,
    pub cellular: Handle<CellularIndicator>,
    pub wifi: Handle<WifiIndicator>,
    pub battery: Handle<BatteryPercentage>,
}

pub fn status_bar() -> StatusBarBuilder {
    StatusBarBuilder
}

pub struct StatusBarBuilder;

impl Build for StatusBarBuilder {
    type Widget = StatusBar;
}

impl Widget for StatusBar {
    type Builder = StatusBarBuilder;

    fn build(_b: StatusBarBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        // Outer status bar: height 36px, full width, padded, space-between left glyph & right cluster
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .row()
            .center()
            .width(percent(100.0))
            .height(px(DEFAULT_BAR_HEIGHT))
            .padding(Insets::symmetric(px(20.0), px(8.0)))
            .justify(Justify::SpaceBetween);

        // Pitch black background
        *s.component_mut::<Paint>(me).unwrap() = Paint::Quad(Quad::new(Color::BLACK));

        // Left cluster container: flex row with gap between items (e.g. [glyph] [clock])
        let left_cluster = s.spawn(
            me,
            div().style(LayoutStyle::default().row().center().gap(px(8.0))),
        );
        let glyph = s.spawn(left_cluster, glyph_indicator());
        let clock = s.spawn(
            left_cluster,
            clock(CLOCK_FORMAT)
                .interval(ClockInterval::Second)
                .size(14.0)
                .color(ColorRole::OnSecondaryContainer),
        );

        // Right cluster container: flex row with gap between items
        let right_cluster = s.spawn(
            me,
            div().style(LayoutStyle::default().row().center().gap(px(8.0))),
        );

        let icons = *s.resource::<StatusIcons>();

        // Layout order (left-to-right in the right cluster):
        // [airplane] [bluetooth] [cellular] [wifi] [battery_percentage]
        // This places battery percentage on the rightmost side,
        // and to the left of it: wifi, cellular, bluetooth, airplane.
        let airplane = s.spawn(right_cluster, airplane_indicator(icons.airplane));
        let bluetooth = s.spawn(
            right_cluster,
            bluetooth_indicator(icons.bluetooth).state(BluetoothState::Off),
        );
        let cellular = s.spawn(
            right_cluster,
            cellular_indicator(icons.cellular).state(CellularState::Disabled),
        );
        let wifi = s.spawn(
            right_cluster,
            wifi_indicator(icons.wifi).state(WifiState::Disabled),
        );
        let battery = s.spawn(right_cluster, battery_percentage());

        s.on::<BatteryPercentageChanged>(me, move |ctx, event| {
            ctx.at(battery).unwrap().set_percentage(event.percentage);
        });
        s.on::<BluetoothStateChanged>(me, move |ctx, event| {
            ctx.at(bluetooth).unwrap().set_state((&event.state).into());
        });
        s.on::<StateUpdate>(me, move |ctx, event| {
            ctx.at(wifi).unwrap().set_connection(event.state);
        });
        s.on::<APUpdate>(me, move |ctx, event| {
            ctx.at(wifi).unwrap().set_strength(event.strength);
        });

        StatusBar {
            glyph,
            clock,
            airplane,
            bluetooth,
            cellular,
            wifi,
            battery,
        }
    }
}
