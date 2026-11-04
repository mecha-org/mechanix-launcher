use mecha_wayland::mechanix_widgets::prelude::{Text, TextContextExt};
use mecha_wayland::prelude::*;

use super::{GAP, build_label, build_tile};
use crate::util::{load_icon, short_name};
use services::{APUpdate, SetWifiEnabled, StateUpdate, WifiState};

// Inclusive lower bounds for the signal-strength icons (0–100%).
const WIFI_MID_MIN_PERCENT: u8 = 25;
const WIFI_HIGH_MIN_PERCENT: u8 = 75;

mod svg {
    pub use assets::icons::AIRPLANE;
    pub use assets::icons::wifi::{HIGH, SLASH, X};
    // The tile has three strength levels, drawn with the 1-arc and dot-only icons.
    pub use assets::icons::wifi::{LOW as MID, NONE as LOW};
}

pub struct Wifi {
    pub icon: Handle<Icon>,
    pub name: Handle<Text>,
    pub dot: Handle<Div>,
    icon_high: SpriteId,
    icon_mid: SpriteId,
    icon_low: SpriteId,
    icon_slash: SpriteId,
    icon_x: SpriteId,
    icon_airplane: SpriteId,
    state: WifiState,
    /// Raw SSID bytes from the latest APUpdate; not necessarily UTF-8.
    ssid: Option<Vec<u8>>,
    /// Last non-empty SSID, kept so Disconnecting can still show it after
    /// NetworkManager clears its active access point.
    last_ssid: Option<Vec<u8>>,
    strength: u8,
    /// Set by a click until the next StateUpdate, so the dot reacts immediately.
    /// `None` means the dot follows the real state.
    optimistic_dot: Option<bool>,
    sprite: SpriteId,
}

pub struct WifiBuilder {
    pub name: String,
}

impl Build for WifiBuilder {
    type Widget = Wifi;
}

impl Widget for Wifi {
    type Builder = WifiBuilder;

    fn build(b: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        // `name` is kept for the builder's existing callers; the label comes from APUpdate.
        let _ = b.name;

        let icon_high = load_icon(&mut s.resource_mut::<Atlas>(), svg::HIGH);
        let icon_mid = load_icon(&mut s.resource_mut::<Atlas>(), svg::MID);
        let icon_low = load_icon(&mut s.resource_mut::<Atlas>(), svg::LOW);
        let icon_slash = load_icon(&mut s.resource_mut::<Atlas>(), svg::SLASH);
        let icon_x = load_icon(&mut s.resource_mut::<Atlas>(), svg::X);
        let icon_airplane = load_icon(&mut s.resource_mut::<Atlas>(), svg::AIRPLANE);

        let icon = build_tile(me, s, svg::X, GAP);
        let name = build_label(me, s, "");
        let dot = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .size(px(5.0), px(5.0))
                    .absolute()
                    .inset(Insets::new(px(8.0), px(10.0), auto(), auto())),
            ),
        );

        s.on::<Clicked>(me, |ctx, _| {
            ctx.toggle();
        });
        s.on::<StateUpdate>(me, |ctx, event| {
            ctx.set_state(event.state);
        });
        s.on::<APUpdate>(me, |ctx, event| {
            ctx.set_access_point(event);
        });

        Wifi {
            icon,
            name,
            dot,
            icon_high,
            icon_mid,
            icon_low,
            icon_slash,
            icon_x,
            icon_airplane,
            state: WifiState::Unavailable,
            ssid: None,
            last_ssid: None,
            strength: 0,
            optimistic_dot: None,
            sprite: icon_x,
        }
    }
}

pub trait WifiContext {
    fn set_state(&mut self, state: WifiState);
    fn set_access_point(&mut self, update: &APUpdate);
    fn toggle(&mut self);
    fn render(&mut self);
}

impl WifiContext for Context<'_, Wifi> {
    fn set_state(&mut self, state: WifiState) {
        {
            let me = self.me();
            me.state = state;
            me.optimistic_dot = None;
            // These states have no connection to name. Connected and
            // Disconnecting keep the latest APUpdate SSID.
            if !matches!(state, WifiState::Connected | WifiState::Disconnecting) {
                me.ssid = None;
                me.last_ssid = None;
            }
        }
        self.render();
    }

    fn set_access_point(&mut self, update: &APUpdate) {
        {
            let me = self.me();
            me.ssid = update.ssid.clone();
            if update.ssid.is_some() {
                me.last_ssid = update.ssid.clone();
            }
            me.strength = update.strength;
        }
        self.render();
    }

    fn toggle(&mut self) {
        // `Off` is the only state where NetworkManager reports
        // WirelessEnabled=false. `Unavailable` has no meaningful software
        // switch to toggle: networking or the radio may already be blocked.
        let enabled = match self.me().state {
            WifiState::Off => true,
            WifiState::Unavailable => return,
            WifiState::On
            | WifiState::Disconnected
            | WifiState::Connecting
            | WifiState::Connected
            | WifiState::Disconnecting => false,
        };
        self.me().optimistic_dot = Some(enabled);
        self.render();
        self.signal(SetWifiEnabled(enabled));
    }

    fn render(&mut self) {
        let (
            state,
            ssid,
            last_ssid,
            strength,
            optimistic_dot,
            icon_handle,
            text_handle,
            dot_handle,
            icon_high,
            icon_mid,
            icon_low,
            icon_slash,
            icon_x,
            icon_airplane,
        ) = {
            let me = self.me();
            (
                me.state,
                me.ssid.clone(),
                me.last_ssid.clone(),
                me.strength,
                me.optimistic_dot,
                me.icon,
                me.name,
                me.dot,
                me.icon_high,
                me.icon_mid,
                me.icon_low,
                me.icon_slash,
                me.icon_x,
                me.icon_airplane,
            )
        };

        let sprite = match state {
            WifiState::Unavailable => icon_x,
            WifiState::Off | WifiState::Disconnected => icon_slash,
            WifiState::On | WifiState::Connecting => icon_airplane,
            WifiState::Connected => {
                if strength >= WIFI_HIGH_MIN_PERCENT {
                    icon_high
                } else if strength >= WIFI_MID_MIN_PERCENT {
                    icon_mid
                } else {
                    icon_low
                }
            }
            WifiState::Disconnecting => icon_low,
        };
        let label = match state {
            WifiState::Connected => ssid.as_deref(),
            WifiState::Disconnecting => ssid.as_deref().or(last_ssid.as_deref()),
            _ => None,
        }
        .map(|ssid| short_name(&String::from_utf8_lossy(ssid)))
        .unwrap_or_default();
        // The tint follows the real state. Only the dot is optimistic: a click
        // turns it off until the next StateUpdate clears the flag.
        let state_enabled = matches!(
            state,
            WifiState::On | WifiState::Connecting | WifiState::Connected
        );
        let dot_enabled = optimistic_dot.unwrap_or(state_enabled);

        let bg_color = if state_enabled {
            self.color(ColorRole::Surface)
        } else {
            Color::TRANSPARENT
        };
        let border_color = self.color(ColorRole::SecondaryFixedDim);
        let icon_color = self.color(ColorRole::OnSurface);
        let text_color = self.color(ColorRole::OnSurfaceVariant);
        let dot_paint = if dot_enabled {
            Paint::Quad(Quad::new(self.color(ColorRole::Primary)).radius(5.0))
        } else {
            Paint::None
        };

        self.set_paint(Paint::Quad(Quad::new(bg_color).border(1.0, border_color)));

        if let Some(mut icon) = self.at(icon_handle) {
            icon.set_sprite(sprite);
            icon.set_color(icon_color);
        }
        if let Some(mut text) = self.at(text_handle) {
            text.set_text(label);
            text.set_color(text_color);
        }
        if let Some(mut dot) = self.at(dot_handle) {
            dot.set_paint(dot_paint);
        }

        self.me().sprite = sprite;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mecha_wayland::mechanix_widgets::NativeText;
    use mecha_wayland::mechanix_widgets::prelude::FontBook;

    #[derive(Default)]
    struct WifiSignals(Vec<bool>);
    impl Resource for WifiSignals {}

    fn record_set_wifi(app: &mut App, event: &SetWifiEnabled) {
        app.resource_mut::<WifiSignals>().0.push(event.0);
    }

    fn setup() -> (App, Handle<Wifi>) {
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark())
            .insert_resource(Atlas::new());
        app.init_resource::<WifiSignals>().system(record_set_wifi);
        let font = app
            .resource_mut::<Atlas>()
            .add_font(assets::fonts::GEIST_MONO)
            .unwrap();
        app.insert_resource(FontBook::new(font));
        let wifi = app.spawn(
            app.root(),
            WifiBuilder {
                name: String::new(),
            },
        );
        app.tick();
        (app, wifi)
    }

    fn emit<E: Event>(app: &mut App, event: E) {
        app.emit(event, app.descendants(app.root()).collect::<Vec<_>>());
        app.tick();
    }

    fn label(app: &App, wifi: &Wifi) -> String {
        let native = app.widget::<Text>(wifi.name).unwrap().native();
        app.widget::<NativeText>(native).unwrap().string.clone()
    }

    fn dot_enabled(app: &App, wifi: &Wifi) -> bool {
        matches!(
            app.component::<Paint>(wifi.dot.id()),
            Some(Paint::Quad(quad)) if quad.color != Color::TRANSPARENT
        )
    }

    fn tinted(app: &App, wifi: Handle<Wifi>) -> bool {
        matches!(
            app.component::<Paint>(wifi.id()),
            Some(Paint::Quad(quad)) if quad.color != Color::TRANSPARENT
        )
    }

    #[test]
    fn state_update_sets_icon_label_dot_and_tint() {
        let (mut app, handle) = setup();
        let (x, slash, airplane, low) = {
            let wifi = app.widget::<Wifi>(handle).unwrap();
            (
                wifi.icon_x,
                wifi.icon_slash,
                wifi.icon_airplane,
                wifi.icon_low,
            )
        };

        for (state, sprite, expected_label, enabled) in [
            (WifiState::Unavailable, x, "", false),
            (WifiState::Off, slash, "", false),
            (WifiState::On, airplane, "", true),
            (WifiState::Disconnected, slash, "", false),
            (WifiState::Connecting, airplane, "", true),
            (WifiState::Connected, low, "", true),
            (WifiState::Disconnecting, low, "", false),
        ] {
            emit(&mut app, StateUpdate { state });
            let wifi = app.widget::<Wifi>(handle).unwrap();
            assert_eq!(wifi.sprite, sprite, "wrong icon for {state:?}");
            assert_eq!(
                label(&app, wifi),
                expected_label,
                "wrong label for {state:?}"
            );
            assert_eq!(dot_enabled(&app, wifi), enabled, "wrong dot for {state:?}");
            assert_eq!(tinted(&app, handle), enabled, "wrong tint for {state:?}");
        }
    }

    #[test]
    fn access_point_uses_strength_only_while_connected() {
        let (mut app, handle) = setup();
        let (x, high, mid, low) = {
            let wifi = app.widget::<Wifi>(handle).unwrap();
            (wifi.icon_x, wifi.icon_high, wifi.icon_mid, wifi.icon_low)
        };
        let office = Some(b"Office".to_vec());

        // APUpdate can arrive before the state. It must not claim the icon
        // while the state is still Unavailable.
        emit(
            &mut app,
            APUpdate {
                ssid: office.clone(),
                strength: 80,
            },
        );
        assert_eq!(app.widget::<Wifi>(handle).unwrap().sprite, x);
        assert_eq!(label(&app, app.widget::<Wifi>(handle).unwrap()), "");

        emit(
            &mut app,
            StateUpdate {
                state: WifiState::Connected,
            },
        );
        assert_eq!(app.widget::<Wifi>(handle).unwrap().sprite, high);
        assert_eq!(label(&app, app.widget::<Wifi>(handle).unwrap()), "Office");

        for (strength, sprite) in [
            (100, high),
            (75, high),
            (74, mid),
            (25, mid),
            (24, low),
            (0, low),
        ] {
            emit(
                &mut app,
                APUpdate {
                    ssid: office.clone(),
                    strength,
                },
            );
            assert_eq!(
                app.widget::<Wifi>(handle).unwrap().sprite,
                sprite,
                "wrong icon for {strength}%"
            );
        }

        emit(
            &mut app,
            APUpdate {
                ssid: None,
                strength: 0,
            },
        );
        assert_eq!(label(&app, app.widget::<Wifi>(handle).unwrap()), "");
    }

    #[test]
    fn disconnecting_keeps_ssid_and_other_states_clear_it() {
        let (mut app, handle) = setup();
        let (slash, airplane, low) = {
            let wifi = app.widget::<Wifi>(handle).unwrap();
            (wifi.icon_slash, wifi.icon_airplane, wifi.icon_low)
        };

        emit(
            &mut app,
            APUpdate {
                ssid: Some(b"Cafe".to_vec()),
                strength: 60,
            },
        );
        emit(
            &mut app,
            StateUpdate {
                state: WifiState::Connected,
            },
        );
        // The AP clearing first must not erase the name that Disconnecting
        // then needs to keep showing.
        emit(
            &mut app,
            APUpdate {
                ssid: None,
                strength: 0,
            },
        );
        assert_eq!(label(&app, app.widget::<Wifi>(handle).unwrap()), "");
        emit(
            &mut app,
            StateUpdate {
                state: WifiState::Disconnecting,
            },
        );
        {
            let wifi = app.widget::<Wifi>(handle).unwrap();
            assert_eq!(wifi.sprite, low);
            assert_eq!(label(&app, wifi), "Cafe");
            assert!(!dot_enabled(&app, wifi));
            assert!(!tinted(&app, handle));
        }
        // A late clear while Disconnecting still keeps the name.
        emit(
            &mut app,
            APUpdate {
                ssid: None,
                strength: 0,
            },
        );
        assert_eq!(label(&app, app.widget::<Wifi>(handle).unwrap()), "Cafe");

        for (state, sprite) in [
            (WifiState::Disconnected, slash),
            (WifiState::Connecting, airplane),
            (WifiState::On, airplane),
            (WifiState::Off, slash),
        ] {
            emit(&mut app, StateUpdate { state });
            let wifi = app.widget::<Wifi>(handle).unwrap();
            assert_eq!(wifi.sprite, sprite);
            assert_eq!(label(&app, wifi), "", "label not cleared for {state:?}");
        }

        // The SSID was cleared, so a later Connected without an APUpdate
        // stays blank.
        emit(
            &mut app,
            StateUpdate {
                state: WifiState::Connected,
            },
        );
        assert_eq!(label(&app, app.widget::<Wifi>(handle).unwrap()), "");
    }

    #[test]
    fn click_optimistically_disables_dot_until_next_state_update() {
        let (mut app, handle) = setup();
        emit(
            &mut app,
            APUpdate {
                ssid: Some(b"Office".to_vec()),
                strength: 90,
            },
        );
        emit(
            &mut app,
            StateUpdate {
                state: WifiState::Connected,
            },
        );
        assert!(dot_enabled(&app, app.widget::<Wifi>(handle).unwrap()));
        assert!(tinted(&app, handle));

        emit(
            &mut app,
            Clicked {
                contact: ContactId::Mouse,
                position: Point::ZERO,
            },
        );
        {
            let wifi = app.widget::<Wifi>(handle).unwrap();
            assert!(!dot_enabled(&app, wifi));
            // The tint reflects the actual state, not the optimistic click.
            assert!(tinted(&app, handle));
            assert_eq!(label(&app, wifi), "Office");
        }
        assert_eq!(app.resource::<WifiSignals>().0, vec![false]);

        emit(
            &mut app,
            StateUpdate {
                state: WifiState::Connected,
            },
        );
        assert!(dot_enabled(&app, app.widget::<Wifi>(handle).unwrap()));
        assert!(tinted(&app, handle));
    }

    #[test]
    fn click_toggles_wifi_based_on_the_radio_state() {
        let (mut app, handle) = setup();
        let click = |app: &mut App| {
            emit(
                app,
                Clicked {
                    contact: ContactId::Mouse,
                    position: Point::ZERO,
                },
            );
        };

        emit(
            &mut app,
            StateUpdate {
                state: WifiState::Off,
            },
        );
        assert!(!dot_enabled(&app, app.widget::<Wifi>(handle).unwrap()));
        click(&mut app);
        assert!(dot_enabled(&app, app.widget::<Wifi>(handle).unwrap()));
        assert_eq!(app.resource::<WifiSignals>().0, vec![true]);

        // The next state update replaces the optimistic dot.
        emit(
            &mut app,
            StateUpdate {
                state: WifiState::On,
            },
        );
        assert!(dot_enabled(&app, app.widget::<Wifi>(handle).unwrap()));
        click(&mut app);
        assert!(!dot_enabled(&app, app.widget::<Wifi>(handle).unwrap()));
        assert_eq!(app.resource::<WifiSignals>().0, vec![true, false]);

        emit(
            &mut app,
            StateUpdate {
                state: WifiState::Off,
            },
        );
        assert!(!dot_enabled(&app, app.widget::<Wifi>(handle).unwrap()));
    }

    #[test]
    fn click_while_disconnected_disables_the_radio_but_unavailable_is_a_no_op() {
        let (mut app, handle) = setup();
        let click = |app: &mut App| {
            emit(
                app,
                Clicked {
                    contact: ContactId::Mouse,
                    position: Point::ZERO,
                },
            );
        };

        // Disconnected means the radio is on, so the click turns it off.
        emit(
            &mut app,
            StateUpdate {
                state: WifiState::Disconnected,
            },
        );
        click(&mut app);
        assert_eq!(app.resource::<WifiSignals>().0, vec![false]);

        // Unavailable has no meaningful software switch to toggle.
        emit(
            &mut app,
            StateUpdate {
                state: WifiState::Unavailable,
            },
        );
        click(&mut app);
        assert_eq!(app.resource::<WifiSignals>().0, vec![false]);
        assert!(!dot_enabled(&app, app.widget::<Wifi>(handle).unwrap()));
    }
}
