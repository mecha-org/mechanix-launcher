use mecha_wayland::prelude::*;

const MPRIS_DBUS_INTERFACE: &str = "org.mpris.MediaPlayer2";

#[derive(Default)]
pub struct MprisBus {
    names: Vec<String>,
}

impl Resource for MprisBus {}

pub struct MprisDiscovery {
    list_call: CallId,
    owner_match: MatchId,
}

impl Resource for MprisDiscovery {}

pub fn get_mpris_dbus_interface() -> MatchRule {
    MatchRule::signal()
        .sender("org.freedesktop.DBus")
        .interface("org.freedesktop.DBus")
        .member("NameOwnerChanged")
        .arg0_namespace(MPRIS_DBUS_INTERFACE)
}

impl MprisDiscovery {
    pub fn new(bus: &mut Bus) -> Self {
        let owner_match = bus.add_match(get_mpris_dbus_interface());
        let list_call = bus.list_names();
        Self {
            list_call,
            owner_match,
        }
    }
}

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
            println!("mpris: running {name}");
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
