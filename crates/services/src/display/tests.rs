use super::DisplayModule;
use super::backend::DisplayStatus;
use super::interface::DISPLAY;
use mecha_wayland::prelude::*;

#[test]
fn gaining_display_owner_refreshes_brightness() {
    let (mut app, _peer) = crate::test_bus::system_bus();
    app.add_module(DisplayModule);
    let initial = app.resource::<DisplayStatus>().read.unwrap();
    let owner_changed = app.resource::<DisplayStatus>().owner_changed;
    let signal = |new_owner: &str| SignalReceived {
        bus: BusId::System,
        matches: [owner_changed].into_iter().collect(),
        msg: Message::signal(
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "NameOwnerChanged",
        )
        .with_body(&(DISPLAY, ":1.2", new_owner)),
    };

    app.signal(signal(""));
    app.flush();
    assert_eq!(app.resource::<DisplayStatus>().read, None);

    app.signal(signal(":1.3"));
    app.flush();
    assert_ne!(app.resource::<DisplayStatus>().read, Some(initial));
    assert!(app.resource::<DisplayStatus>().read.is_some());
}
