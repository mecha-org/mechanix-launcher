//! Registers the agent with polkitd for the launcher's session:
//! `XDG_SESSION_ID`, else logind's `auto` session.
//!
//! A failure is logged and the launcher runs without an agent. polkitd
//! drops the registration when the launcher leaves the bus. The agent
//! takes calls only from the polkitd that answered, and registers again
//! when polkitd restarts.

use std::collections::HashMap;

use mecha_wayland::prelude::*;

use super::agent::{self, AGENT_PATH};

const POLKIT: &str = "org.freedesktop.PolicyKit1";
const AUTHORITY_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
const AUTHORITY: &str = "org.freedesktop.PolicyKit1.Authority";

const LOGIND: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";
const LOGIND_MANAGER: &str = "org.freedesktop.login1.Manager";
const LOGIND_SESSION: &str = "org.freedesktop.login1.Session";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

const DAEMON: &str = "org.freedesktop.DBus";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Step {
    /// logind's `GetSession("auto")`.
    GetSession,
    /// That session's `Id` property.
    SessionId,
    /// polkitd's `RegisterAuthenticationAgent`.
    Register,
}

#[derive(Debug)]
struct Registration {
    bus: BusId,
    call: Option<(CallId, Step)>,
    session: Option<String>,
    /// polkitd's unique name, while registered.
    polkitd: Option<BusName>,
    owner_changed: Option<MatchId>,
}
impl Resource for Registration {}

/// Register for `session`, or logind's `auto` session if `None`.
pub(crate) fn install(app: &mut App, bus: BusId, session: Option<&str>) {
    if app.has_resource::<Registration>() {
        return;
    }
    let owner_changed = app.resource_mut::<DBus>().bus(bus).map(|mut b| {
        b.add_match(
            MatchRule::signal()
                .sender(DAEMON)
                .interface(DAEMON)
                .member("NameOwnerChanged")
                .arg(0, POLKIT),
        )
    });
    app.insert_resource(Registration {
        bus,
        call: None,
        session: None,
        polkitd: None,
        owner_changed,
    });
    app.system(on_reply)
        .system(on_owner_changed)
        .system(on_disconnect);
    match session {
        Some(id) => register(app, id),
        None => send(
            app,
            MethodCall::new(LOGIND, LOGIND_PATH, LOGIND_MANAGER, "GetSession").body(&("auto",)),
            Step::GetSession,
        ),
    }
}

fn send(app: &mut App, call: MethodCall, step: Step) {
    let bus = app.resource::<Registration>().bus;
    let id = {
        let mut dbus = app.resource_mut::<DBus>();
        let Some(mut b) = dbus.bus(bus) else {
            tracing::error!("polkit: {step:?}: the bus is not connected");
            return;
        };
        b.send(call)
    };
    app.resource_mut::<Registration>().call = Some((id, step));
}

fn register(app: &mut App, session: &str) {
    tracing::info!("polkit: registering the agent for session {session}");
    app.resource_mut::<Registration>().session = Some(session.into());
    let subject = (
        "unix-session",
        HashMap::from([("session-id", Value::Str(session.into()))]),
    );
    send(
        app,
        MethodCall::new(
            POLKIT,
            AUTHORITY_PATH,
            AUTHORITY,
            "RegisterAuthenticationAgent",
        )
        .body(&(subject, locale(), AGENT_PATH)),
        Step::Register,
    );
}

/// The locale for polkitd's messages.
fn locale() -> String {
    ["LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.is_empty())
        .unwrap_or_default()
}

fn on_reply(app: &mut App, e: &MethodReply) {
    let step = {
        let mut reg = app.resource_mut::<Registration>();
        match reg.call {
            Some((call, step)) if call == e.call => {
                reg.call = None;
                step
            }
            _ => return,
        }
    };
    let msg = match &e.result {
        Ok(msg) => msg,
        Err(error) => {
            tracing::error!("polkit: {step:?} failed, no agent for the session: {error:?}");
            return;
        }
    };
    match step {
        Step::GetSession => {
            let Ok((path,)) = msg.body::<(ObjectPath,)>() else {
                tracing::error!("polkit: GetSession: unexpected reply");
                return;
            };
            send(
                app,
                MethodCall::new(LOGIND, path, PROPERTIES, "Get").body(&(LOGIND_SESSION, "Id")),
                Step::SessionId,
            );
        }
        Step::SessionId => match msg.body::<(Value,)>() {
            Ok((Value::Str(id),)) => register(app, &id),
            _ => tracing::error!("polkit: the session's Id: unexpected reply"),
        },
        Step::Register => {
            let Some(polkitd) = msg.sender().cloned() else {
                tracing::error!("polkit: the registration's reply has no sender");
                return;
            };
            tracing::info!("polkit: agent registered with {polkitd}");
            app.resource_mut::<Registration>().polkitd = Some(polkitd.clone());
            agent::set_polkitd(app, Some(polkitd));
        }
    }
}

/// polkitd went away or came back: forget it, or register again.
fn on_owner_changed(app: &mut App, e: &SignalReceived) {
    let (bus, owner_changed) = {
        let reg = app.resource::<Registration>();
        (reg.bus, reg.owner_changed)
    };
    if e.bus != bus || owner_changed.is_none_or(|m| !e.matches.contains(&m)) {
        return;
    }
    let Ok((_, _, new_owner)) = e.msg.body::<(String, String, String)>() else {
        return;
    };
    let gone = {
        let mut reg = app.resource_mut::<Registration>();
        let gone = reg
            .polkitd
            .as_ref()
            .is_some_and(|p| p.as_str() != new_owner);
        if gone {
            reg.polkitd = None;
        }
        gone
    };
    if gone {
        tracing::warn!("polkit: polkitd went away, its requests are dropped");
        agent::set_polkitd(app, None);
    }
    // A registration in flight already reaches the new polkitd.
    let session = {
        let reg = app.resource::<Registration>();
        let idle = reg.polkitd.is_none() && reg.call.is_none();
        reg.session
            .clone()
            .filter(|_| idle && !new_owner.is_empty())
    };
    if let Some(session) = session {
        register(app, &session);
    }
}

fn on_disconnect(app: &mut App, e: &BusDisconnected) {
    let mut reg = app.resource_mut::<Registration>();
    if e.bus == reg.bus {
        reg.call = None;
        reg.polkitd = None;
    }
}

#[cfg(test)]
mod tests;
