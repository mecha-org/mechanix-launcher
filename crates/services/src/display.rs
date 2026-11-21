//! Brightness control through mxbrightnessd on the session bus.
mod backend;
mod interface;

#[cfg(test)]
mod tests;

pub use backend::{BrightnessChanged, MAX_BRIGHTNESS, MIN_BRIGHTNESS, SetBrightness};

use backend::DisplayStatus;
use interface::{BRIGHTNESS, INTERFACE, PATH};
use mecha_wayland::prelude::*;

/// Install after `RingModule` and `DBusModule::new().session()`.
pub struct DisplayModule;
impl Module for DisplayModule {
    fn install(self, app: &mut App) {
        if app.has_resource::<DisplayStatus>() {
            return;
        }
        let (owner_changed, properties) = {
            let mut dbus = app.resource_mut::<DBus>();
            let mut bus = dbus.session();
            (
                bus.add_match(interface::owner_changed()),
                bus.watch_properties(BRIGHTNESS, PATH, INTERFACE),
            )
        };
        app.insert_resource(DisplayStatus {
            owner_changed,
            properties,
            read: None,
            read_stale: false,
            write: None,
            pending: None,
            value: MIN_BRIGHTNESS,
            minimum: MIN_BRIGHTNESS,
        });
        app.system(backend::on_reply)
            .system(backend::on_set_brightness)
            .system(backend::on_properties_changed)
            .system(backend::on_owner_changed)
            .system(backend::on_disconnect);
        backend::read_properties(app);
    }
}
