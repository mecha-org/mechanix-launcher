use mecha_wayland::prelude::*;

use super::bus::{MprisBus, MprisDiscovery};

pub fn on_reply(app: &mut App, e: &MethodReply) {
    if e.call != app.resource::<MprisDiscovery>().list_call {
        return;
    }

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
        if name.starts_with("org.mpris.MediaPlayer2.") {
            app.resource_mut::<MprisBus>().names.push(name);
        }
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

    let mut bus = app.resource_mut::<MprisBus>();
    if new.is_empty() {
        bus.names.retain(|n| *n != name);
    } else if !bus.names.contains(&name) {
        bus.names.push(name);
    }
}
