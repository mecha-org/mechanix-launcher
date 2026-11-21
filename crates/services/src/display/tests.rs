use super::DisplayModule;
use super::backend::{BrightnessChanged, SetBrightness};
use super::interface::{BRIGHTNESS, INTERFACE, PATH};
use dbus::fake::FakeBus;
use mecha_wayland::prelude::*;

const OWNER: &str = ":1.20";

#[derive(Default)]
struct Updates(Vec<(f32, f32)>);
impl Resource for Updates {}

fn record_update(app: &mut App, event: &Emitted<BrightnessChanged>) {
    app.resource_mut::<Updates>()
        .0
        .push((event.event.value, event.event.minimum));
}

/// A session bus where `org.mechanix.Brightness` is owned, with the module
/// installed and its startup requests drained.
fn bus() -> FakeBus {
    let mut bus = FakeBus::new();
    bus.owners.insert(BRIGHTNESS.into(), OWNER.into());
    bus.app.init_resource::<Updates>().system(record_update);
    bus.app.add_module(DisplayModule);
    bus.turn();
    bus
}

fn updates(bus: &mut FakeBus) -> Vec<(f32, f32)> {
    std::mem::take(&mut bus.app.resource_mut::<Updates>().0)
}

fn props(entries: &[(&str, f64)]) -> PropMap {
    entries
        .iter()
        .map(|(key, value)| (key.to_string(), Value::F64(*value)))
        .collect()
}

fn assert_get_all(call: &Message) {
    assert_eq!(call.destination().unwrap().as_str(), BRIGHTNESS);
    assert_eq!(call.path().unwrap().as_str(), PATH);
    assert_eq!(
        call.interface().unwrap().as_str(),
        "org.freedesktop.DBus.Properties"
    );
    assert_eq!(call.body::<(String,)>().unwrap().0, INTERFACE);
}

/// The value of a `Properties.Set` of `Brightness`.
fn set_value(call: &Message) -> f64 {
    assert_eq!(call.destination().unwrap().as_str(), BRIGHTNESS);
    assert_eq!(call.path().unwrap().as_str(), PATH);
    let (interface, name, value) = call.body::<(String, String, Value)>().unwrap();
    assert_eq!(interface, INTERFACE);
    assert_eq!(name, "Brightness");
    match value {
        Value::F64(value) => value,
        other => panic!("Brightness set as {other:?}"),
    }
}

fn properties_changed(changed: PropMap, invalidated: &[&str]) -> Message {
    Message::signal(PATH, "org.freedesktop.DBus.Properties", "PropertiesChanged")
        .with_sender(OWNER)
        .with_body(&(
            INTERFACE,
            changed,
            invalidated
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>(),
        ))
}

fn owner_changed(old: &str, new: &str) -> Message {
    Message::signal(
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
        "NameOwnerChanged",
    )
    .with_sender("org.freedesktop.DBus")
    .with_body(&(BRIGHTNESS, old, new))
}

/// Answer the startup `GetAll` with `Brightness` and `Minimum`.
fn ready(bus: &mut FakeBus, brightness: f64, minimum: f64) {
    let read = bus.expect_call("GetAll");
    bus.reply(
        &read,
        &(props(&[
            ("Brightness", brightness),
            ("Minimum", minimum),
            ("AmbientLux", 120.0),
        ]),),
    );
    bus.turn();
    bus.received();
}

#[test]
fn installation_watches_properties_on_the_session_bus_then_reads_them() {
    let mut bus = bus();
    let sent = bus.received();
    let watch = sent
        .iter()
        .position(|m| {
            m.member().unwrap().as_str() == "AddMatch"
                && m.body::<(String,)>()
                    .unwrap()
                    .0
                    .contains("PropertiesChanged")
        })
        .expect("a PropertiesChanged match");
    let read = sent
        .iter()
        .position(|m| m.member().unwrap().as_str() == "GetAll")
        .expect("a GetAll");
    assert!(watch < read);
    let (rule,) = sent[watch].body::<(String,)>().unwrap();
    for part in [
        "sender='org.mechanix.Brightness'",
        "path='/org/mechanix/Brightness'",
        "arg0='org.mechanix.Brightness'",
    ] {
        assert!(rule.contains(part), "missing {part} in {rule}");
    }
    assert_get_all(&sent[read]);
    assert!(updates(&mut bus).is_empty());
}

#[test]
fn snapshot_reports_brightness_and_minimum() {
    let mut bus = bus();
    ready(&mut bus, 0.4, 0.05);
    assert_eq!(updates(&mut bus), [(0.4, 0.05)]);
}

#[test]
fn failed_snapshot_reports_nothing() {
    let mut bus = bus();
    let read = bus.expect_call("GetAll");
    bus.error(
        &read,
        "org.freedesktop.DBus.Error.ServiceUnknown",
        "not running",
    );
    bus.turn();
    assert!(updates(&mut bus).is_empty());
}

#[test]
fn property_changes_report_the_latest_brightness_and_minimum() {
    let mut bus = bus();
    ready(&mut bus, 0.4, 0.05);
    updates(&mut bus);

    bus.send(properties_changed(props(&[("Brightness", 0.7)]), &[]));
    bus.turn();
    assert_eq!(updates(&mut bus), [(0.7, 0.05)]);

    bus.send(properties_changed(props(&[("Minimum", 0.1)]), &[]));
    bus.turn();
    assert_eq!(updates(&mut bus), [(0.7, 0.1)]);

    // Changes to properties the slider does not show are ignored.
    bus.send(properties_changed(props(&[("AmbientLux", 3.0)]), &[]));
    bus.turn();
    assert!(updates(&mut bus).is_empty());
}

#[test]
fn invalidated_brightness_is_read_again() {
    let mut bus = bus();
    ready(&mut bus, 0.4, 0.05);
    updates(&mut bus);

    bus.send(properties_changed(PropMap::new(), &["Brightness"]));
    bus.turn();
    assert_get_all(&bus.expect_call("GetAll"));
    assert!(updates(&mut bus).is_empty());
}

#[test]
fn set_brightness_sets_the_property() {
    let mut bus = bus();
    ready(&mut bus, 0.4, 0.05);

    bus.app.signal(SetBrightness(0.6));
    bus.turn();
    let set = bus.expect_call("Set");
    assert!((set_value(&set) - 0.6).abs() < 1e-6);
}

#[test]
fn set_brightness_stays_within_minimum_and_one() {
    let mut bus = bus();
    ready(&mut bus, 0.4, 0.05);

    for (asked, sent) in [(0.0, 0.05), (1.5, 1.0)] {
        bus.app.signal(SetBrightness(asked));
        bus.turn();
        let set = bus.expect_call("Set");
        assert!((set_value(&set) - sent).abs() < 1e-6, "asked {asked}");
        bus.reply(&set, &());
        bus.turn();
    }

    bus.app.signal(SetBrightness(f32::NAN));
    bus.turn();
    assert!(
        bus.received()
            .iter()
            .all(|m| m.member().is_none_or(|m| m.as_str() != "Set"))
    );
}

#[test]
fn writes_in_flight_coalesce_to_the_newest_value() {
    let mut bus = bus();
    ready(&mut bus, 0.4, 0.05);

    bus.app.signal(SetBrightness(0.5));
    bus.turn();
    let first = bus.expect_call("Set");
    for value in [0.6, 0.7, 0.8] {
        bus.app.signal(SetBrightness(value));
    }
    bus.turn();
    assert!(
        bus.received()
            .iter()
            .all(|m| m.member().is_none_or(|m| m.as_str() != "Set"))
    );

    bus.reply(&first, &());
    bus.turn();
    let second = bus.expect_call("Set");
    assert!((set_value(&second) - 0.8).abs() < 1e-6);
}

#[test]
fn a_set_made_before_the_snapshot_keeps_the_users_value() {
    let mut bus = bus();
    let read = bus.expect_call("GetAll");
    bus.received();

    bus.app.signal(SetBrightness(0.6));
    bus.turn();
    bus.reply(&read, &(props(&[("Brightness", 0.4), ("Minimum", 0.05)]),));
    bus.turn();
    assert_eq!(updates(&mut bus), [(0.6, 0.05)]);
}

#[test]
fn gaining_an_owner_reads_the_properties_again() {
    let mut bus = bus();
    ready(&mut bus, 0.4, 0.05);

    bus.send(owner_changed(OWNER, ""));
    bus.turn();
    assert!(
        bus.received()
            .iter()
            .all(|m| m.member().is_none_or(|m| m.as_str() != "GetAll"))
    );

    bus.send(owner_changed("", ":1.21"));
    bus.turn();
    assert_get_all(&bus.expect_call("GetAll"));
}
