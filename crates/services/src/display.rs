//! Brightness control through the Mechanix Display service on the system bus.
mod backend;
mod interface;

#[cfg(test)]
mod tests;

pub use backend::{BrightnessChanged, MAX_BRIGHTNESS, MIN_BRIGHTNESS, SetBrightness};

use backend::DisplayStatus;
use mecha_wayland::prelude::*;

pub struct DisplayModule;
impl Module for DisplayModule {
    fn install(self, app: &mut App) {
        if app.has_resource::<DisplayStatus>() {
            return;
        }
        let owner_changed = app
            .resource_mut::<DBus>()
            .system()
            .add_match(interface::owner_changed());
        app.insert_resource(DisplayStatus {
            owner_changed,
            read: None,
            write: None,
            pending: None,
        });
        app.system(backend::on_reply)
            .system(backend::on_set_brightness)
            .system(backend::on_owner_changed)
            .system(backend::on_disconnect);
        backend::read_brightness(app);
    }
}
