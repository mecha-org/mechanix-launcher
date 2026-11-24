use dbus::fake::{FakeBus, UNIQUE};

use super::*;

const POLKITD: &str = ":1.5";

/// The cookies [`AuthCancelled`] named.
struct Cancelled(Vec<String>);
impl Resource for Cancelled {}

fn record_cancelled(app: &mut App, e: &AuthCancelled) {
    app.resource_mut::<Cancelled>().0.push(e.cookie.clone());
}

fn agent() -> FakeBus {
    let mut bus = FakeBus::new();
    assert!(install(&mut bus.app, BusId::Session));
    set_polkitd(&mut bus.app, Some(BusName::new(POLKITD).unwrap()));
    bus.turn();
    bus.received();
    bus
}

fn call(member: &'static str) -> Message {
    call_from(POLKITD, member)
}

fn call_from(sender: &'static str, member: &'static str) -> Message {
    Message::method_call(AGENT_PATH, member)
        .with_interface(AGENT_IFACE)
        .with_destination(UNIQUE)
        .with_sender(sender)
}

fn begin(bus: &mut FakeBus, cookie: &str) -> u32 {
    let details = HashMap::from([("program".to_string(), "/usr/bin/true".to_string())]);
    let identities = vec![
        (
            "unix-user".to_string(),
            HashMap::from([("uid".to_string(), Value::U32(1000))]),
        ),
        (
            "unix-group".to_string(),
            HashMap::from([("gid".to_string(), Value::U32(27))]),
        ),
        ("unix-netgroup".to_string(), HashMap::new()),
    ];
    let serial = bus.send(call("BeginAuthentication").with_body(&(
        "org.freedesktop.policykit.exec",
        "Authentication is needed",
        "",
        details,
        cookie,
        identities,
    )));
    bus.turn();
    serial
}

fn cancel(bus: &mut FakeBus, cookie: &str) -> u32 {
    let serial = bus.send(call("CancelAuthentication").with_body(&(cookie,)));
    bus.turn();
    serial
}

/// The answer to the call with `serial`, if any.
fn answer(received: &[Message], serial: u32) -> Option<&Message> {
    received.iter().find(|m| m.reply_serial() == Some(serial))
}

fn error_name(m: &Message) -> &str {
    m.error_name().map_or("", |n| n.as_str())
}

fn active_cookie(bus: &FakeBus) -> Option<String> {
    let agent = bus.app.resource::<Agent>();
    agent.active.as_ref().map(|k| k.request.cookie.clone())
}

fn queued_cookies(bus: &FakeBus) -> Vec<String> {
    let agent = bus.app.resource::<Agent>();
    agent
        .queued
        .iter()
        .map(|k| k.request.cookie.clone())
        .collect()
}

#[test]
fn a_request_is_decoded_and_kept_unanswered() {
    let mut bus = agent();
    let serial = begin(&mut bus, "c1");
    assert!(answer(&bus.received(), serial).is_none());

    let agent = bus.app.resource::<Agent>();
    let request = &agent.active.as_ref().unwrap().request;
    assert_eq!(request.action_id, "org.freedesktop.policykit.exec");
    assert_eq!(request.message, "Authentication is needed");
    assert_eq!(request.details["program"], "/usr/bin/true");
    assert_eq!(request.cookie, "c1");
    assert_eq!(
        request.identities,
        [
            Identity::User(1000),
            Identity::Group(27),
            Identity::Other("unix-netgroup".into()),
        ]
    );
}

#[test]
fn finishing_answers_the_request() {
    let mut bus = agent();
    let serial = begin(&mut bus, "c1");
    bus.received();

    // Another cookie changes nothing.
    bus.app.signal(AuthFinished {
        cookie: "other".into(),
        result: Ok(()),
    });
    bus.turn();
    assert!(answer(&bus.received(), serial).is_none());

    bus.app.signal(AuthFinished {
        cookie: "c1".into(),
        result: Ok(()),
    });
    bus.turn();
    let received = bus.received();
    let reply = answer(&received, serial).unwrap();
    assert_eq!(reply.kind(), MessageKind::MethodReturn);
    assert_eq!(active_cookie(&bus), None);
}

#[test]
fn a_failure_answers_failed() {
    let mut bus = agent();
    let serial = begin(&mut bus, "c1");
    bus.received();
    bus.app.signal(AuthFinished {
        cookie: "c1".into(),
        result: Err("Wrong password".into()),
    });
    bus.turn();
    let received = bus.received();
    assert_eq!(error_name(answer(&received, serial).unwrap()), FAILED);
}

#[test]
fn cancelling_answers_cancelled() {
    let mut bus = agent();
    let begun = begin(&mut bus, "c1");
    bus.received();

    // An unknown cookie is refused and the request stays.
    let wrong = cancel(&mut bus, "other");
    let received = bus.received();
    assert_eq!(error_name(answer(&received, wrong).unwrap()), FAILED);
    assert!(answer(&received, begun).is_none());

    let cancelled = cancel(&mut bus, "c1");
    let received = bus.received();
    assert_eq!(error_name(answer(&received, begun).unwrap()), CANCELLED);
    assert_eq!(
        answer(&received, cancelled).unwrap().kind(),
        MessageKind::MethodReturn
    );
    assert_eq!(active_cookie(&bus), None);
}

#[test]
fn requests_wait_their_turn() {
    let mut bus = agent();
    let first = begin(&mut bus, "c1");
    let second = begin(&mut bus, "c2");
    let third = begin(&mut bus, "c3");
    let received = bus.received();
    for serial in [first, second, third] {
        assert!(answer(&received, serial).is_none());
    }
    assert_eq!(active_cookie(&bus).as_deref(), Some("c1"));
    assert_eq!(queued_cookies(&bus), ["c2", "c3"]);

    // Finishing a queued one does nothing: only the active is answered.
    bus.app.signal(AuthFinished {
        cookie: "c2".into(),
        result: Ok(()),
    });
    bus.turn();
    assert!(answer(&bus.received(), second).is_none());

    // The first done, the next in line becomes active.
    bus.app.signal(AuthFinished {
        cookie: "c1".into(),
        result: Ok(()),
    });
    bus.turn();
    assert!(answer(&bus.received(), first).is_some());
    assert_eq!(active_cookie(&bus).as_deref(), Some("c2"));
    assert_eq!(queued_cookies(&bus), ["c3"]);
}

#[test]
fn a_queued_request_can_be_cancelled() {
    let mut bus = agent();
    begin(&mut bus, "c1");
    let second = begin(&mut bus, "c2");
    begin(&mut bus, "c3");
    bus.received();

    cancel(&mut bus, "c2");
    let received = bus.received();
    assert_eq!(error_name(answer(&received, second).unwrap()), CANCELLED);
    assert_eq!(active_cookie(&bus).as_deref(), Some("c1"));
    assert_eq!(queued_cookies(&bus), ["c3"]);

    // Cancelling the active one moves the queue on.
    cancel(&mut bus, "c1");
    assert_eq!(active_cookie(&bus).as_deref(), Some("c3"));
    assert!(queued_cookies(&bus).is_empty());
}

#[test]
fn only_polkitd_may_call() {
    let mut bus = agent();
    let begun = begin(&mut bus, "c1");

    let stranger = bus.send(call_from(":1.66", "CancelAuthentication").with_body(&("c1",)));
    bus.turn();
    let received = bus.received();
    assert_eq!(
        error_name(answer(&received, stranger).unwrap()),
        ACCESS_DENIED
    );
    // The request is untouched.
    assert!(answer(&received, begun).is_none());
    assert_eq!(active_cookie(&bus).as_deref(), Some("c1"));

    let details: HashMap<String, String> = HashMap::new();
    let identities: Vec<(String, HashMap<String, Value>)> = Vec::new();
    let fake = bus.send(call_from(":1.66", "BeginAuthentication").with_body(&(
        "org.freedesktop.policykit.exec",
        "Give me your password",
        "",
        details,
        "c2",
        identities,
    )));
    bus.turn();
    let received = bus.received();
    assert_eq!(error_name(answer(&received, fake).unwrap()), ACCESS_DENIED);
    assert!(queued_cookies(&bus).is_empty());
}

#[test]
fn unregistered_takes_no_calls() {
    let mut bus = agent();
    set_polkitd(&mut bus.app, None);
    let serial = begin(&mut bus, "c1");
    let received = bus.received();
    assert_eq!(
        error_name(answer(&received, serial).unwrap()),
        ACCESS_DENIED
    );
    assert_eq!(active_cookie(&bus), None);
}

#[test]
fn polkitd_going_away_drops_its_requests() {
    let mut bus = agent();
    begin(&mut bus, "c1");
    begin(&mut bus, "c2");
    bus.received();

    bus.app.insert_resource(Cancelled(Vec::new()));
    bus.app.system(record_cancelled);

    set_polkitd(&mut bus.app, None);
    bus.turn();
    assert_eq!(active_cookie(&bus), None);
    assert!(queued_cookies(&bus).is_empty());
    assert_eq!(bus.app.resource::<Cancelled>().0, ["c1"]);
}

#[test]
fn without_the_bus_there_is_no_agent() {
    // The fake connects only the session bus.
    let mut bus = FakeBus::new();
    assert!(!install(&mut bus.app, BusId::System));
    // Installing again says the same.
    assert!(!install(&mut bus.app, BusId::System));
}

#[test]
fn the_bus_going_away_cancels_the_active_request() {
    let mut bus = agent();
    begin(&mut bus, "c1");
    begin(&mut bus, "c2");
    bus.received();

    bus.app.insert_resource(Cancelled(Vec::new()));
    bus.app.system(record_cancelled);

    bus.app.signal(BusDisconnected {
        bus: BusId::Session,
    });
    bus.turn();
    assert_eq!(active_cookie(&bus), None);
    assert!(queued_cookies(&bus).is_empty());
    assert_eq!(bus.app.resource::<Cancelled>().0, ["c1"]);
    assert_eq!(polkitd(&bus.app), None);
}
