mod backend;
mod bus;
mod convert;
mod event;
mod interface;
mod media_command;
mod media_signal;
mod players;
mod types;

#[cfg(test)]
mod tests;

use backend::{on_command, on_reply, on_signal};
use bus::MprisDiscovery;
use mecha_wayland::prelude::*;
pub use media_command::MediaCommand;
pub use media_signal::MediaSignals;
pub use players::MprisPlayers;
pub use types::{MediaPlayer, PlaybackStatus, PlayerId, Track};

pub struct MprisModule;
impl Module for MprisModule {
    fn install(self, app: &mut App) {
        if app.has_resource::<MprisPlayers>() {
            return;
        }

        let discovery = MprisDiscovery::new(&mut app.resource_mut::<DBus>().session());
        app.insert_resource(discovery);
        app.insert_resource(MprisPlayers::default());
        app.system(on_reply).system(on_signal).system(on_command);
    }
}
