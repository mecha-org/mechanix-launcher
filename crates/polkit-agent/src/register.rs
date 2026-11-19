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

use std::collections::HashMap;

use mecha_wayland::prelude::*;

use crate::agent::AGENT_PATH;

const POLKIT: &str = "org.freedesktop.PolicyKit1";
const AUTHORITY_PATH: &str = "/org/freedesktop/PolicyKit1/Authority";
const AUTHORITY: &str = "org.freedesktop.PolicyKit1.Authority";

const LOGIND: &str = "org.freedesktop.login1";
const LOGIND_PATH: &str = "/org/freedesktop/login1";
const LOGIND_MANAGER: &str = "org.freedesktop.login1.Manager";
const LOGIND_SESSION: &str = "org.freedesktop.login1.Session";
const PROPERTIES: &str = "org.freedesktop.DBus.Properties";

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
}
impl Resource for Registration {}

/// Register the agent, exported on `bus`, with polkitd for `session`, or
/// for logind's `auto` session if `None`.
pub(crate) fn install(app: &mut App, bus: BusId, session: Option<&str>) {
    if app.has_resource::<Registration>() {
        return;
    }
    app.insert_resource(Registration { bus, call: None });
    app.system(on_reply).system(on_disconnect);
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
        Step::Register => tracing::info!("polkit: agent registered"),
    }
}

fn on_disconnect(app: &mut App, e: &BusDisconnected) {
    let mut reg = app.resource_mut::<Registration>();
    if e.bus == reg.bus {
        reg.call = None;
    }
}

#[cfg(test)]
mod tests {
    use dbus::fake::FakeBus;

    use super::*;
    use crate::agent;

    type RegisterBody = ((String, HashMap<String, Value>), String, String);

    fn installed(session: Option<&str>) -> FakeBus {
        let mut bus = FakeBus::new();
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

    #[test]
    fn registers_for_the_given_session() {
        let mut bus = installed(Some("3"));
        let call = expect_register(&mut bus, "3");
        bus.reply(&call, &());
        bus.turn();
        assert_eq!(bus.app.resource::<Registration>().call, None);
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
