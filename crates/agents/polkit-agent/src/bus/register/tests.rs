use dbus::fake::FakeBus;

const POLKITD: &str = ":1.5";

use super::*;
use crate::bus::agent;

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
