use mecha_wayland::prelude::*;

use super::bus::MprisDiscovery;
use super::event::MprisEvent;
use super::interface::{METADATA, MPRIS_PATH, MPRIS_PREFIX, PAUSE, PLAY_PAUSE, PLAYER_INTERFACE};
use super::media_command::MediaCommand;
use super::media_signal::MediaSignals;
use super::players::MprisPlayers;
use super::types::{MediaPlayer, PlaybackPosition, PlaybackStatus, PlayerId, Track};

pub fn on_reply(app: &mut App, e: &MethodReply) {
    let Some(event) = MprisEvent::from_reply(app, e) else {
        return;
    };
    handle(app, event);
}

pub fn on_signal(app: &mut App, e: &SignalReceived) {
    let Some(event) = MprisEvent::from_signal(app, e) else {
        return;
    };
    handle(app, event);
}

fn handle(app: &mut App, event: MprisEvent) {
    match event {
        MprisEvent::PlayerNames(names) => {
            for name in names {
                if !name.starts_with(MPRIS_PREFIX) {
                    continue;
                }
                let Ok(name) = BusName::try_from(name) else {
                    continue;
                };
                MprisDiscovery::subscribe_player(app, name);
            }
        }
        MprisEvent::PlayerData(id, props) => {
            let player = MediaPlayer {
                id: id.clone(),
                status: PlaybackStatus::from(&props),
                position: PlaybackPosition::from(&props),
                track: Track::from(&props),
            };
            app.resource_mut::<MprisPlayers>().add_player(player);
            app.signal(MediaSignals::NotifyPlayerArrival(id));
        }
        MprisEvent::PlayerAppeared(name) => MprisDiscovery::subscribe_player(app, name),
        MprisEvent::PlayerLeft(name) => {
            MprisDiscovery::unsubscribe_player(app, &name);
            let id = PlayerId::from(name);
            app.resource_mut::<MprisPlayers>().remove_player(&id);
            app.signal(MediaSignals::PlayerRemoved(id));
        }
        MprisEvent::PlayerUpdated(id, changed) => {
            let updated = app
                .resource_mut::<MprisPlayers>()
                .player_data_changed(&id, &changed);

            if changed.contains_key(METADATA) {
                MprisDiscovery::request_position(app, &id);
            }
            if updated {
                app.signal(MediaSignals::PlayerChanged(id));
            }
        }
        MprisEvent::Position(id, at) => {
            let updated = app.resource_mut::<MprisPlayers>().set_position(&id, at);
            if updated {
                app.signal(MediaSignals::PositionChanged(id));
            }
        }
    }
}

pub fn on_command(app: &mut App, command: &MediaCommand) {
    match command {
        MediaCommand::Pause(id) => dbus_method_call(app, id, PAUSE),
        MediaCommand::PlayPause(id) => dbus_method_call(app, id, PLAY_PAUSE),
    }
}

fn dbus_method_call(app: &mut App, id: &PlayerId, method: &'static str) {
    app.resource_mut::<DBus>()
        .session()
        .object(id.0.clone(), MPRIS_PATH)
        .call_no_reply(PLAYER_INTERFACE, method, &());
}
