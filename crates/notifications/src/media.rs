mod album_art;
mod media_card;
mod media_list;
mod play_button;
mod progress;
mod track_info;

use mecha_wayland::prelude::*;
use services::mpris::MediaSignals;

pub use media_card::{PlayerUpdated, PositionUpdated};
pub use media_list::{MediaList, MediaListBuilder, RemovePlayer, ShowPlayer};

pub fn on_media_signal(app: &mut App, signal: &MediaSignals) {
    let Some((list, _)) = app.widgets::<MediaList>().next() else {
        return;
    };
    match signal {
        MediaSignals::NotifyPlayerArrival(id) => app.emit(ShowPlayer(id.clone()), list),
        MediaSignals::PlayerChanged(id) => app.emit(PlayerUpdated(id.clone()), list),
        MediaSignals::PlayerRemoved(id) => app.emit(RemovePlayer(id.clone()), list),
        MediaSignals::PositionChanged(id) => app.emit(PositionUpdated(id.clone()), list),
    }
}
