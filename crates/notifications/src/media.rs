mod media_card;
mod track_info;

use mecha_wayland::prelude::*;
use services::mpris::MediaSignals;

pub use media_card::{MediaCard, MediaCardBuilder, ShowPlayer};

pub fn on_media_signal(app: &mut App, signal: &MediaSignals) {
    match signal {
        MediaSignals::NotifyPlayerArrival(id) => {
            let Some((card, _)) = app.widgets::<MediaCard>().next() else {
                return;
            };
            app.emit(ShowPlayer(id.clone()), card);
        }
    }
}
