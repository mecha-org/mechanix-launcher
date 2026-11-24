mod backend;
mod bus;
mod interface;
mod types;

#[cfg(test)]
mod tests;

use bus::{MprisBus, MprisDiscovery, on_reply, on_signal};
use mecha_wayland::prelude::*;
pub use types::{MediaPlayer, MprisPlayers, PlayerId};

pub struct MprisModule;
impl Module for MprisModule {
    fn install(self, app: &mut App) {
        if app.has_resource::<MprisPlayers>() {
            return;
        }

        let discovery = MprisDiscovery::new(&mut app.resource_mut::<DBus>().session());

        app.insert_resource(discovery);
        app.insert_resource(MprisBus::default());
        app.insert_resource(MprisPlayers::default());
        app.system(on_reply).system(on_signal);
    }
}
