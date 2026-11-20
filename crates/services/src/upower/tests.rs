use super::UpowerModule;
use super::backend::{
    BatteryPercentageChanged, DEVICE_INTERFACE, DISPLAY_DEVICE, UPOWER, UpowerBattery,
    battery_percentage, battery_valid, upower_types,
};
use mecha_wayland::prelude::*;
use std::collections::VecDeque;
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;

#[derive(Default)]
struct Updates(Vec<Option<u8>>);
impl Resource for Updates {}

fn record_update(app: &mut App, event: &Emitted<BatteryPercentageChanged>) {
    app.resource_mut::<Updates>().0.push(event.event.percentage);
}

/// A private socket pair supplies only the D-Bus startup handshake. Tests inject
/// decoded replies/signals through App, and inspect real outbound requests.
/// No system bus, UPower daemon, Wayland connection, or rendering is involved.
struct Harness {
    app: App,
    peer: UnixStream,
}

impl Harness {
    fn new() -> Self {
        // The binary Hello is left for requests().
        let (mut app, peer) = crate::test_bus::system_bus();
        app.init_resource::<Updates>().system(record_update);
        peer.set_nonblocking(true).unwrap();
        app.add_module(UpowerModule);
        Self { app, peer }
    }

    fn call(&self) -> CallId {
        self.app.resource::<UpowerBattery>().property_read
    }

    fn reply(&mut self, call: CallId, properties: PropMap) {
        self.app.signal(MethodReply {
            call,
            result: Ok(Message::method_return(call.serial).with_body(&(properties,))),
        });
        self.app.flush();
    }

    fn change(&mut self, changed: PropMap, invalidated: &[&str]) {
        let watch = self.app.resource::<UpowerBattery>().property_changed;
        self.app.signal(SignalReceived {
            bus: BusId::System,
            matches: [watch].into_iter().collect(),
            msg: Message::signal(
                DISPLAY_DEVICE,
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
            )
            .with_body(&(
                DEVICE_INTERFACE,
                changed,
                invalidated
                    .iter()
                    .map(|name| name.to_string())
                    .collect::<Vec<_>>(),
            )),
        });
        self.app.flush();
    }

    fn updates(&mut self) -> Vec<Option<u8>> {
        std::mem::take(&mut self.app.resource_mut::<Updates>().0)
    }

    fn requests(&mut self) -> Vec<Message> {
        // A trailing call marks the end of the output we want to inspect.
        let barrier = self
            .app
            .resource_mut::<DBus>()
            .system()
            .send(MethodCall::new(
                "org.example.Test",
                "/org/example/Test",
                "org.example.Test",
                "Barrier",
            ));
        let mut bytes = Vec::new();
        let mut messages = Vec::new();
        let mut chunk = [0; 4096];
        for turn in 0..64 {
            // Ensure each ring turn has incoming work even if a send completion
            // was already consumed. The unregistered signal has no UI effect.
            let (wake, _) = Message::signal("/org/example/Test", "org.example.Test", "Wake")
                .with_sender(":1.2")
                .encode(turn + 2);
            self.peer.write_all(&wake).unwrap();
            Ring::turn(&mut self.app);

            loop {
                match self.peer.read(&mut chunk) {
                    Ok(0) => panic!("test bus unexpectedly closed"),
                    Ok(n) => bytes.extend_from_slice(&chunk[..n]),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) => panic!("reading test bus: {e}"),
                }
            }
            while let Some(len) = Message::frame_len(&bytes).unwrap() {
                let message = Message::parse(&bytes[..len], &mut VecDeque::new()).unwrap();
                bytes.drain(..len);
                if message.serial() == barrier.serial {
                    return messages;
                }
                messages.push(message);
            }
        }
        panic!("D-Bus output failed to drain");
    }
}

fn props(entries: &[(&str, Value)]) -> PropMap {
    entries
        .iter()
        .map(|(key, value)| (key.to_string(), value.clone()))
        .collect()
}

fn snapshot(present: bool, device_type: u32, percentage: f64) -> PropMap {
    props(&[
        ("IsPresent", Value::Bool(present)),
        ("Type", Value::U32(device_type)),
        ("Percentage", Value::F64(percentage)),
    ])
}

fn assert_get_all(message: &Message, call: CallId) {
    assert_eq!(call.bus, BusId::System);
    assert_eq!(message.serial(), call.serial);
    assert_eq!(message.destination().unwrap().as_str(), UPOWER);
    assert_eq!(message.path().unwrap().as_str(), DISPLAY_DEVICE);
    assert_eq!(
        message.interface().unwrap().as_str(),
        "org.freedesktop.DBus.Properties"
    );
    assert_eq!(message.member().unwrap().as_str(), "GetAll");
    assert_eq!(message.body::<(String,)>().unwrap().0, DEVICE_INTERFACE);
}

#[test]
fn percentage_missing_is_none() {
    assert_eq!(battery_percentage(&PropMap::new()), None);
}

#[test]
fn percentage_rounds_and_clamps_without_confusing_zero_with_absence() {
    for (input, expected) in [
        (-10.0, 0),
        (0.0, 0),
        (0.49, 0),
        (0.5, 1),
        (42.49, 42),
        (42.5, 43),
        (99.9, 100),
        (100.0, 100),
        (150.0, 100),
    ] {
        assert_eq!(
            battery_percentage(&props(&[("Percentage", Value::F64(input))])),
            Some(expected),
            "input: {input}"
        );
    }
}

#[test]
#[should_panic(expected = "UPower: invalid Percentage")]
fn percentage_rejects_wrong_type() {
    battery_percentage(&props(&[("Percentage", Value::U32(42))]));
}

#[test]
#[should_panic(expected = "UPower: non-finite Percentage")]
fn percentage_rejects_nan() {
    battery_percentage(&props(&[("Percentage", Value::F64(f64::NAN))]));
}

#[test]
#[should_panic(expected = "UPower: non-finite Percentage")]
fn percentage_rejects_positive_infinity() {
    battery_percentage(&props(&[("Percentage", Value::F64(f64::INFINITY))]));
}

#[test]
#[should_panic(expected = "UPower: non-finite Percentage")]
fn percentage_rejects_negative_infinity() {
    battery_percentage(&props(&[("Percentage", Value::F64(f64::NEG_INFINITY))]));
}

#[test]
fn installation_subscribes_before_requesting_the_initial_snapshot() {
    let mut h = Harness::new();
    let battery = h.app.resource::<UpowerBattery>();
    assert!(!battery.is_present);
    assert_eq!(battery.device_type, upower_types::UNKNOWN);
    assert!(!battery_valid(&h.app));

    let requests = h.requests();
    let watch = requests
        .iter()
        .position(|m| m.member().unwrap().as_str() == "AddMatch")
        .unwrap();
    let read = requests
        .iter()
        .position(|m| m.member().unwrap().as_str() == "GetAll")
        .unwrap();
    assert!(watch < read);
    let (rule,) = requests[watch].body::<(String,)>().unwrap();
    for part in [
        "type='signal'",
        "sender='org.freedesktop.UPower'",
        "path='/org/freedesktop/UPower/devices/DisplayDevice'",
        "interface='org.freedesktop.DBus.Properties'",
        "member='PropertiesChanged'",
        "arg0='org.freedesktop.UPower.Device'",
    ] {
        assert!(rule.contains(part), "missing {part} in {rule}");
    }
    assert_get_all(&requests[read], h.call());
    assert!(h.updates().is_empty());
}

#[test]
fn second_installation_is_a_no_op() {
    let mut h = Harness::new();
    let call = h.call();
    h.app.add_module(UpowerModule);
    assert_eq!(h.call(), call);

    let members = |h: &mut Harness| -> Vec<String> {
        h.requests()
            .iter()
            .map(|m| m.member().unwrap().as_str().to_owned())
            .collect()
    };
    assert_eq!(members(&mut h), members(&mut Harness::new()));

    h.reply(call, snapshot(true, 2, 42.0));
    assert_eq!(h.updates(), [Some(42)]);
}

#[test]
fn eligibility_requires_both_presence_and_battery_type() {
    let mut h = Harness::new();
    for (present, device_type, expected) in [
        (true, 2, true),
        (false, 2, false),
        (true, 0, false),
        (true, 1, false),
        (true, 3, false),
        (false, 3, false),
    ] {
        {
            let mut battery = h.app.resource_mut::<UpowerBattery>();
            battery.is_present = present;
            battery.device_type = device_type;
        }
        assert_eq!(battery_valid(&h.app), expected);
    }
}

#[test]
fn reply_updates_eligibility_and_emits_percentage() {
    let mut h = Harness::new();
    h.reply(h.call(), snapshot(true, 2, 42.5));
    assert!(battery_valid(&h.app));
    assert_eq!(h.updates(), [Some(43)]);
}

#[test]
fn ineligible_replies_emit_none_without_requiring_percentage() {
    let mut h = Harness::new();
    for (present, device_type) in [(false, 2), (true, 0), (true, 3)] {
        h.reply(
            h.call(),
            props(&[
                ("IsPresent", Value::Bool(present)),
                ("Type", Value::U32(device_type)),
            ]),
        );
        assert!(!battery_valid(&h.app));
        assert_eq!(h.updates(), [None]);
    }
}

#[test]
fn unrelated_reply_ids_and_buses_are_ignored_before_decoding() {
    let mut h = Harness::new();
    let call = h.call();
    for unrelated in [
        CallId {
            serial: call.serial + 1,
            ..call
        },
        CallId {
            bus: BusId::Session,
            ..call
        },
    ] {
        // An error would panic if the guard did not ignore it.
        h.app.signal(MethodReply {
            call: unrelated,
            result: Err(CallError::Timeout),
        });
        h.app.flush();
        assert!(!battery_valid(&h.app));
        assert!(h.updates().is_empty());
    }
}

#[test]
#[should_panic(expected = "UPower: GetAll failed")]
fn matching_reply_error_panics() {
    let mut h = Harness::new();
    h.app.signal(MethodReply {
        call: h.call(),
        result: Err(CallError::Timeout),
    });
    h.app.flush();
}

#[test]
#[should_panic(expected = "UPower: invalid GetAll reply")]
fn malformed_reply_body_panics() {
    let mut h = Harness::new();
    let call = h.call();
    h.app.signal(MethodReply {
        call,
        result: Ok(Message::method_return(call.serial).with_body(&("not a property map",))),
    });
    h.app.flush();
}

#[test]
#[should_panic(expected = "UPower: missing IsPresent")]
fn reply_requires_presence() {
    let mut h = Harness::new();
    let mut properties = snapshot(true, 2, 42.0);
    properties.remove("IsPresent");
    h.reply(h.call(), properties);
}

#[test]
#[should_panic(expected = "UPower: invalid IsPresent")]
fn reply_rejects_wrong_presence_type() {
    let mut h = Harness::new();
    let mut properties = snapshot(true, 2, 42.0);
    properties.insert("IsPresent".into(), Value::U32(1));
    h.reply(h.call(), properties);
}

#[test]
#[should_panic(expected = "UPower: missing Type")]
fn reply_requires_device_type() {
    let mut h = Harness::new();
    let mut properties = snapshot(true, 2, 42.0);
    properties.remove("Type");
    h.reply(h.call(), properties);
}

#[test]
#[should_panic(expected = "UPower: invalid Type")]
fn reply_rejects_wrong_device_type() {
    let mut h = Harness::new();
    let mut properties = snapshot(true, 2, 42.0);
    properties.insert("Type".into(), Value::Str("battery".into()));
    h.reply(h.call(), properties);
}

#[test]
#[should_panic(expected = "UPower: missing Percentage")]
fn eligible_reply_requires_percentage() {
    let mut h = Harness::new();
    h.reply(
        h.call(),
        props(&[("IsPresent", Value::Bool(true)), ("Type", Value::U32(2))]),
    );
}

#[test]
fn percentage_signal_emits_even_when_other_properties_are_absent() {
    let mut h = Harness::new();
    h.reply(h.call(), snapshot(true, 2, 80.0));
    h.updates();
    let call = h.call();
    for (percentage, expected) in [(42.5, 43), (0.0, 0)] {
        h.change(props(&[("Percentage", Value::F64(percentage))]), &[]);
        assert_eq!(h.updates(), [Some(expected)]);
        assert_eq!(h.call(), call);
        assert!(battery_valid(&h.app));
    }
}

#[test]
fn missing_percentage_and_unrelated_invalidations_leave_label_unchanged() {
    let mut h = Harness::new();
    h.reply(h.call(), snapshot(true, 2, 80.0));
    assert_eq!(h.updates(), [Some(80)]);
    let call = h.call();
    h.change(PropMap::new(), &[]);
    h.change(props(&[("TimeToEmpty", Value::I64(120))]), &["State"]);
    assert_eq!(h.call(), call);
    assert!(h.updates().is_empty());
}

#[test]
fn unrelated_invalidation_does_not_suppress_percentage() {
    let mut h = Harness::new();
    h.reply(h.call(), snapshot(true, 2, 80.0));
    h.updates();
    let call = h.call();
    h.change(props(&[("Percentage", Value::F64(55.0))]), &["TimeToEmpty"]);
    assert_eq!(h.call(), call);
    assert_eq!(h.updates(), [Some(55)]);
}

#[test]
fn explicit_eligibility_changes_refresh_snapshot() {
    let mut h = Harness::new();
    h.reply(h.call(), snapshot(true, 2, 80.0));
    h.updates();
    let call = h.call();
    h.change(
        props(&[("IsPresent", Value::Bool(false)), ("Type", Value::U32(3))]),
        &[],
    );
    assert_ne!(h.call(), call);
    assert!(battery_valid(&h.app));
    assert!(h.updates().is_empty());
    h.reply(h.call(), snapshot(false, 3, 80.0));
    assert!(!battery_valid(&h.app));
    assert_eq!(h.updates(), [None]);
}

#[test]
fn percentage_signal_on_ineligible_device_does_not_show_battery() {
    let mut h = Harness::new();
    h.reply(h.call(), snapshot(false, 2, 40.0));
    assert_eq!(h.updates(), [None]);
    h.change(props(&[("Percentage", Value::F64(60.0))]), &[]);
    assert!(h.updates().is_empty());
}

#[test]
fn unrelated_subscription_is_ignored_before_parsing() {
    let mut h = Harness::new();
    let other = h
        .app
        .resource_mut::<DBus>()
        .system()
        .add_match(MatchRule::signal().interface("org.example.Other"));
    let call = h.call();
    h.app.signal(SignalReceived {
        bus: BusId::System,
        matches: [other].into_iter().collect(),
        msg: Message::signal(DISPLAY_DEVICE, "org.example.Other", "Changed"),
    });
    h.app.flush();
    assert_eq!(h.call(), call);
    assert!(h.updates().is_empty());
}

#[test]
fn matching_subscription_need_not_be_first_in_matches() {
    let mut h = Harness::new();
    h.reply(h.call(), snapshot(true, 2, 80.0));
    h.updates();
    let other = h
        .app
        .resource_mut::<DBus>()
        .system()
        .add_match(MatchRule::signal());
    let watch = h.app.resource::<UpowerBattery>().property_changed;
    h.app.signal(SignalReceived {
        bus: BusId::System,
        matches: [other, watch].into_iter().collect(),
        msg: Message::signal(
            DISPLAY_DEVICE,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
        )
        .with_body(&(
            DEVICE_INTERFACE,
            props(&[("Percentage", Value::F64(25.0))]),
            Vec::<String>::new(),
        )),
    });
    h.app.flush();
    assert_eq!(h.updates(), [Some(25)]);
}

#[test]
#[should_panic(expected = "UPower: invalid PropertiesChanged signal")]
fn matching_malformed_signal_panics() {
    let mut h = Harness::new();
    let watch = h.app.resource::<UpowerBattery>().property_changed;
    h.app.signal(SignalReceived {
        bus: BusId::System,
        matches: [watch].into_iter().collect(),
        msg: Message::signal(
            DISPLAY_DEVICE,
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
        )
        .with_body(&("not a properties-changed body",)),
    });
    h.app.flush();
}

#[test]
fn each_relevant_invalidation_requests_get_all_without_emitting() {
    let mut h = Harness::new();
    h.requests();
    for name in ["IsPresent", "Type", "Percentage"] {
        let previous = h.call();
        h.change(PropMap::new(), &[name]);
        assert_ne!(h.call(), previous, "invalidation of {name}");
        assert!(h.updates().is_empty());
        let requests = h.requests();
        assert_eq!(requests.len(), 1);
        assert_get_all(&requests[0], h.call());
    }
}

#[test]
fn invalidation_takes_precedence_and_batches_into_one_refresh() {
    let mut h = Harness::new();
    h.requests();
    // Deliberately malformed: percentage must not even be decoded on this path.
    h.change(
        props(&[("Percentage", Value::Str("ignore me".into()))]),
        &["IsPresent", "Type", "Percentage"],
    );
    assert!(h.updates().is_empty());
    let requests = h.requests();
    assert_eq!(requests.len(), 1);
    assert_get_all(&requests[0], h.call());
}

#[test]
fn newest_refresh_wins_and_can_transition_between_percentage_and_na() {
    let mut h = Harness::new();
    let initial = h.call();
    h.reply(initial, snapshot(true, 2, 70.0));
    assert_eq!(h.updates(), [Some(70)]);

    h.change(PropMap::new(), &["Percentage"]);
    let superseded = h.call();
    h.change(PropMap::new(), &["IsPresent"]);
    let latest = h.call();
    assert_ne!(latest, superseded);

    h.reply(initial, snapshot(true, 2, 90.0));
    h.reply(superseded, snapshot(true, 2, 80.0));
    assert!(h.updates().is_empty());

    h.reply(latest, snapshot(false, 2, 0.0));
    assert_eq!(h.updates(), [None]);
    assert!(!battery_valid(&h.app));

    h.change(PropMap::new(), &["IsPresent"]);
    h.reply(h.call(), snapshot(true, 2, 65.0));
    assert_eq!(h.updates(), [Some(65)]);
    assert!(battery_valid(&h.app));
}
