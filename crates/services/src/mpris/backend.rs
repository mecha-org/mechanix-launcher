use mecha_wayland::prelude::*;

use super::bus::{MprisBus, MprisDiscovery, PendingCall};
use super::interface::{MPRIS_PATH, MPRIS_PREFIX, PLAYER_INTERFACE};
use super::media_signal::MediaSignals;
use super::types::{MediaPlayer, MprisPlayers, PlaybackStatus, PlayerId, Track};

pub fn on_reply(app: &mut App, e: &MethodReply) {
    let pending = app.resource_mut::<MprisDiscovery>().pending.remove(&e.call);
    let Some(call) = pending else {
        return;
    };

    match call {
        PendingCall::ListNames => on_names(app, e),
        PendingCall::GetAll(name) => on_data(app, name, e),
    }
}

fn on_names(app: &mut App, e: &MethodReply) {
    let msg = match &e.result {
        Ok(msg) => msg,
        Err(error) => {
            eprintln!("[mpris] ListNames failed: {error:?}");
            return;
        }
    };

    let Ok((names,)) = msg.body::<(Vec<String>,)>() else {
        eprintln!("[mpris] ListNames: unexpected reply");
        return;
    };

    for name in names {
        if !name.starts_with(MPRIS_PREFIX) {
            continue;
        }
        let Ok(name) = BusName::try_from(name) else {
            continue;
        };
        request_data(app, &name);
        app.resource_mut::<MprisBus>().names.push(name);
    }
}

pub fn on_signal(app: &mut App, e: &SignalReceived) {
    if !e
        .matches
        .contains(&app.resource::<MprisDiscovery>().owner_match)
    {
        return;
    }

    let Ok((name, _old, new)) = e.msg.body::<(String, String, String)>() else {
        return;
    };
    let Ok(name) = BusName::try_from(name) else {
        return;
    };

    if new.is_empty() {
        app.resource_mut::<MprisBus>().names.retain(|n| *n != name);
        app.resource_mut::<MprisPlayers>()
            .players
            .retain(|p| p.id.0 != name.as_str());
    } else if !app.resource::<MprisBus>().names.contains(&name) {
        request_data(app, &name);
        app.resource_mut::<MprisBus>().names.push(name);
    }
}

fn request_data(app: &mut App, name: &BusName) {
    let call = app
        .resource_mut::<DBus>()
        .session()
        .object(name.clone(), MPRIS_PATH)
        .get_all(PLAYER_INTERFACE);
    app.resource_mut::<MprisDiscovery>()
        .pending
        .insert(call, PendingCall::GetAll(name.as_str().to_string()));
}

fn on_data(app: &mut App, name: String, e: &MethodReply) {
    let msg = match &e.result {
        Ok(msg) => msg,
        Err(error) => {
            eprintln!("[mpris] GetAll {name} failed: {error:?}");
            return;
        }
    };

    let Ok((props,)) = msg.body::<(PropMap,)>() else {
        eprintln!("[mpris] GetAll {name}: unexpected reply");
        return;
    };

    let player = MediaPlayer {
        id: PlayerId(name),
        status: PlaybackStatus::from(&props),
        track: Track::from(&props),
    };

    let mut players = app.resource_mut::<MprisPlayers>();

    players.players.retain(|p| p.id != player.id);
    let id = player.id.clone();
    players.players.push(player);
    drop(players);

    // TODO: show all of the mpris servers that are active??
    app.signal(MediaSignals::NotifyPlayerArrival(id));
}
