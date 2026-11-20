//! Registering the agent with polkitd for the launcher's session.
//!
//! polkitd sends a session's authentication requests to the agent
//! registered for it, so the agent names its session in
//! `RegisterAuthenticationAgent`'s `unix-session` subject:
//!
//! - `XDG_SESSION_ID` when set.
//! - Otherwise logind's `auto` session, the user's display session: the
//!   launcher runs as a user service, outside the session's scope, and
//!   the user manager's environment need not carry the session's id. Its
//!   `Id` is read from logind.
//!
//! polkitd drops the registration by itself when the launcher leaves the
//! bus, so there is nothing to undo on exit. A failure (polkitd missing,
//! another agent already registered for the session) is logged, and the
//! launcher goes on without an agent.
//!
//! The unique name that answered the registration is polkitd's: the agent
//! takes calls only from it. `NameOwnerChanged` for polkitd's name says
//! when it goes away (its requests are dropped) and when one comes back
//! (the agent registers again, as a restarted polkitd knows no agents).

use std::collections::HashMap;

use mecha_wayland::prelude::*;

use crate::agent::{self, AGENT_PATH};

const POLKIT: &str = "org.freedesktop.PolicyKit1";
const AUTHORITY_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
const AUTHORITY: &str = "org.freedesktop.PolicyKit1.Authority";

const LOGIND: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";
const LOGIND_MANAGER: &str = "org.freedesktop.login1.Manager";
const LOGIND_SESSION: &str = "org.freedesktop.login1.Session";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

const DAEMON: &str = "org.freedesktop.DBus";

/// What a call in flight is for.
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
    /// The call in flight, until it is answered.
    call: Option<(CallId, Step)>,
    /// The session registered for, once known.
    session: Option<String>,
    /// polkitd's unique name, while the agent is registered with it.
    polkitd: Option<BusName>,
    /// `NameOwnerChanged` for polkitd's name.
    owner_changed: Option<MatchId>,
}
impl Resource for Registration {}

/// Register the agent, exported on `bus`, with polkitd for `session`, or
/// for logind's `auto` session if `None`.
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

/// The locale polkitd translates the requests' messages into.
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

/// polkitd's name changed hands: forget the polkitd that went away, and
/// register with the one that came.
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
    // A registration in flight already reaches the new owner: it is what
    // started polkitd, if polkitd was not running.
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
mod tests {
    use dbus::fake::FakeBus;

    const POLKITD: &str = ":1.5";

    use super::*;
    use crate::agent;

    type RegisterBody = ((String, HashMap<String, Value>), String, String);

    fn installed(session: Option<&str>) -> FakeBus {
        let mut bus = FakeBus::new();
        bus.owners.insert(POLKIT.into(), POLKITD.into());
        assert!(agent::install(&mut bus.app, BusId::Session));
        install(&mut bus.app, BusId::Session, session);
        bus.turn();
        bus
    }

    fn expect_register(bus: &mut FakeBus, session: &str) -> Message {
        let call = bus.expect_call("RegisterAuthenticationAgent");
        assert_eq!(call.destination().unwrap().as_str(), POLKIT);
        assert_eq!(call.path().unwrap().as_str(), AUTHORITY_PATH);
        assert_eq!(call.interface().unwrap().as_str(), AUTHORITY);
        let ((kind, details), _locale, path) = call.body::<RegisterBody>().unwrap();
        assert_eq!(kind, "unix-session");
        assert_eq!(
            details,
            HashMap::from([("session-id".into(), Value::Str(session.into()))])
        );
        assert_eq!(path, AGENT_PATH);
        call
    }

    fn owner_changed(bus: &mut FakeBus, old: &str, new: &str) {
        bus.send(
            Message::signal("/org/freedesktop/DBus", DAEMON, "NameOwnerChanged")
                .with_sender(DAEMON)
                .with_body(&(POLKIT, old, new)),
        );
        bus.turn();
    }

    fn agents_polkitd(bus: &FakeBus) -> Option<String> {
        agent::polkitd(&bus.app).map(|n| n.as_str().to_owned())
    }

    /// Installed for session 3 and registered with [`POLKITD`].
    fn registered() -> FakeBus {
        let mut bus = installed(Some("3"));
        let call = expect_register(&mut bus, "3");
        bus.reply(&call, &());
        bus.turn();
        bus.received();
        bus
    }

    #[test]
    fn registers_for_the_given_session() {
        let bus = registered();
        assert_eq!(bus.app.resource::<Registration>().call, None);
        // The agent takes calls from whoever answered.
        assert_eq!(agents_polkitd(&bus).as_deref(), Some(POLKITD));
    }

    #[test]
    fn polkitd_starting_for_the_registration_is_not_a_restart() {
        let mut bus = installed(Some("3"));
        let call = expect_register(&mut bus, "3");
        // The registration activated polkitd: its name appears first.
        owner_changed(&mut bus, "", POLKITD);
        bus.reply(&call, &());
        bus.turn();
        assert!(!bus.received().iter().any(|m| {
            m.member()
                .is_some_and(|m| m.as_str() == "RegisterAuthenticationAgent")
        }));
        assert_eq!(agents_polkitd(&bus).as_deref(), Some(POLKITD));
    }

    #[test]
    fn a_restarted_polkitd_is_registered_with_again() {
        let mut bus = registered();

        owner_changed(&mut bus, POLKITD, "");
        assert_eq!(agents_polkitd(&bus), None);
        assert!(bus.received().is_empty());

        bus.owners.insert(POLKIT.into(), ":1.9".into());
        owner_changed(&mut bus, "", ":1.9");
        let call = expect_register(&mut bus, "3");
        bus.reply(&call, &());
        bus.turn();
        assert_eq!(agents_polkitd(&bus).as_deref(), Some(":1.9"));
    }

    #[test]
    fn without_a_session_asks_logind_for_auto() {
        let mut bus = installed(None);
        let get = bus.expect_call("GetSession");
        assert_eq!(get.body::<(String,)>().unwrap().0, "auto");
        bus.reply(
            &get,
            &(ObjectPath::from("/org/freedesktop/login1/session/c2"),),
        );
        bus.turn();

        let id = bus.expect_call("Get");
        assert_eq!(
            id.path().unwrap().as_str(),
            "/org/freedesktop/login1/session/c2"
        );
        assert_eq!(
            id.body::<(String, String)>().unwrap(),
            (LOGIND_SESSION.into(), "Id".into())
        );
        bus.reply(&id, &(Value::Str("c2".into()),));
        bus.turn();

        expect_register(&mut bus, "c2");
    }

    #[test]
    fn a_refused_registration_is_given_up() {
        let mut bus = installed(Some("3"));
        let call = expect_register(&mut bus, "3");
        bus.received();
        bus.error(
            &call,
            "org.freedesktop.PolicyKit1.Error.Failed",
            "An authentication agent already exists for the given subject",
        );
        bus.turn();
        assert_eq!(bus.app.resource::<Registration>().call, None);
        assert!(bus.received().is_empty());
    }
}
