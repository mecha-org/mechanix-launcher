use super::*;
use std::collections::HashSet;
use std::ops::{Deref, DerefMut};
use std::os::unix::net::UnixStream;

struct TestApp(App);

#[derive(Default)]
struct StateUpdates(Vec<WifiState>);
impl Resource for StateUpdates {}

fn record_state_update(app: &mut App, event: &Emitted<StateUpdate>) {
    app.resource_mut::<StateUpdates>().0.push(event.event.state);
}

#[derive(Default)]
struct APUpdates(Vec<APUpdate>);
impl Resource for APUpdates {}

fn record_ap_update(app: &mut App, event: &Emitted<APUpdate>) {
    app.resource_mut::<APUpdates>().0.push(event.event.clone());
}

impl Deref for TestApp {
    type Target = App;

    fn deref(&self) -> &App {
        &self.0
    }
}

impl DerefMut for TestApp {
    fn deref_mut(&mut self) -> &mut App {
        &mut self.0
    }
}

impl Drop for TestApp {
    fn drop(&mut self) {
        // Drain pending I/O while DBus still owns its socket. App resources have
        // no guaranteed drop order; closing the socket first permits fd reuse
        // by another test before the old ring submits/cancels its queued reads.
        drop(self.0.insert_resource(Ring::new(8)));
    }
}

fn pending_call(app: &App, matches: impl Fn(&CallType) -> bool) -> CallId {
    let state = app.resource::<NMState>();
    let calls: Vec<_> = state
        .pending
        .iter()
        .filter(|(_, kind)| matches(kind))
        .map(|(call, _)| *call)
        .collect();
    assert_eq!(calls.len(), 1, "expected exactly one matching pending call");
    assert_eq!(calls[0].bus, BusId::System);
    calls[0]
}

fn device_call(app: &App, path: &ObjectPath) -> CallId {
    pending_call(
        app,
        |kind| matches!(kind, CallType::DeviceProperties(device) if device == path),
    )
}

fn active_ap_call(app: &App, path: &ObjectPath) -> CallId {
    pending_call(
        app,
        |kind| matches!(kind, CallType::ActiveAccessPoint(device) if device == path),
    )
}

fn ap_properties_call(app: &App, device: &ObjectPath, ap: &ObjectPath) -> CallId {
    pending_call(app, |kind| {
        matches!(kind, CallType::AccessPointProperties { device_path, access_point_path }
            if device_path == device && access_point_path == ap)
    })
}

fn reply(app: &mut App, call: CallId, result: Result<Message, CallError>) {
    assert!(app.resource::<NMState>().pending.contains_key(&call));
    app.signal(MethodReply { call, result });
    app.flush();
    assert!(!app.resource::<NMState>().pending.contains_key(&call));
}

fn reply_properties(app: &mut App, call: CallId, properties: PropMap) {
    reply(
        app,
        call,
        Ok(Message::method_return(call.serial).with_body(&(properties,))),
    );
}

fn manager_properties(devices: &[ObjectPath]) -> PropMap {
    PropMap::from([
        ("NetworkingEnabled".into(), Value::Bool(true)),
        ("WirelessHardwareEnabled".into(), Value::Bool(true)),
        ("WirelessEnabled".into(), Value::Bool(true)),
        (
            "Devices".into(),
            Value::Array {
                elem: "o".into(),
                items: devices.iter().cloned().map(Value::ObjectPath).collect(),
            },
        ),
    ])
}

fn discover(app: &mut App, devices: &[ObjectPath]) {
    let call = pending_call(app, |kind| matches!(kind, CallType::GetAll));
    reply_properties(app, call, manager_properties(devices));
    let state = app.resource::<NMState>();
    assert!(state.networking_enabled);
    assert!(state.wireless_hardware_enabled);
    assert!(state.wireless_enabled);
    assert!(state.wifi_devices.is_empty());
    assert!(state.subscriptions.is_empty());
    assert_eq!(state.pending.len(), devices.len());
    for path in devices {
        device_call(app, path);
    }
}

fn device_properties(device_type: u32, state: u32) -> PropMap {
    PropMap::from([
        ("DeviceType".into(), Value::U32(device_type)),
        ("State".into(), Value::U32(state)),
    ])
}

fn ssid_value(bytes: &[u8]) -> Value {
    Value::Array {
        elem: "y".into(),
        items: bytes.iter().copied().map(Value::U8).collect(),
    }
}

fn read_active_ap(app: &mut App, device: &ObjectPath, ap: &ObjectPath) {
    let call = active_ap_call(app, device);
    reply(
        app,
        call,
        Ok(Message::method_return(call.serial).with_body(&(Value::ObjectPath(ap.clone()),))),
    );
}

fn remove_device(app: &mut App, path: &ObjectPath) {
    let id = app.resource::<NMState>().device_removed.unwrap();
    app.signal(SignalReceived {
        bus: BusId::System,
        matches: [id].into_iter().collect(),
        msg: Message::signal(NM_MAIN_OBJ, NM, "DeviceRemoved")
            .with_sender(NM)
            .with_body(&(path.clone(),)),
    });
    app.flush();
}

fn add_device(app: &mut App, path: &ObjectPath) {
    let id = app.resource::<NMState>().device_added.unwrap();
    app.signal(SignalReceived {
        bus: BusId::System,
        matches: [id].into_iter().collect(),
        msg: Message::signal(NM_MAIN_OBJ, NM, "DeviceAdded")
            .with_sender(NM)
            .with_body(&(path.clone(),)),
    });
    app.flush();
}

fn connected_device(app: &mut App, device: &ObjectPath) {
    discover(app, std::slice::from_ref(device));
    let call = device_call(app, device);
    reply_properties(app, call, device_properties(2, 100));
    assert_device(app, device, None);
    assert_eq!(app.resource::<NMState>().subscriptions.len(), 2);
    assert_eq!(app.resource::<NMState>().pending.len(), 1);
    active_ap_call(app, device);
}

fn state_changed_signal(
    path: &ObjectPath,
    id: MatchId,
    new_state: u32,
    old_state: u32,
) -> SignalReceived {
    SignalReceived {
        bus: BusId::System,
        matches: [id].into_iter().collect(),
        msg: Message::signal(path.clone(), NM_DEVICE, "StateChanged")
            .with_sender(NM)
            .with_body(&(new_state, old_state, 1u32)),
    }
}

fn wireless_change(app: &mut App, path: &ObjectPath, changed: PropMap, invalidated: &[&str]) {
    let id = app.resource::<NMState>().wifi_devices[path]
        .wireless_properties_changed
        .unwrap();
    app.signal(SignalReceived {
        bus: BusId::System,
        matches: [id].into_iter().collect(),
        msg: Message::signal(
            path.clone(),
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
        )
        .with_sender(NM)
        .with_body(&(
            NM_WIRELESS,
            changed,
            invalidated
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>(),
        )),
    });
    app.flush();
}

fn ap_properties(ssid: &[u8], strength: u8) -> PropMap {
    PropMap::from([
        ("Ssid".into(), ssid_value(ssid)),
        ("Strength".into(), Value::U8(strength)),
    ])
}

fn initialize_ap(app: &mut App, device: &ObjectPath, ap: &ObjectPath, ssid: &[u8], strength: u8) {
    read_active_ap(app, device, ap);
    let call = ap_properties_call(app, device, ap);
    reply_properties(app, call, ap_properties(ssid, strength));
    assert_device(app, device, Some((ap, ssid, strength)));
}

fn ap_properties_signal(
    path: &ObjectPath,
    id: MatchId,
    changed: PropMap,
    invalidated: &[&str],
) -> SignalReceived {
    SignalReceived {
        bus: BusId::System,
        matches: [id].into_iter().collect(),
        msg: Message::signal(
            path.clone(),
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
        )
        .with_sender(NM)
        .with_body(&(
            NM_ACCESS_POINT,
            changed,
            invalidated
                .iter()
                .map(|name| name.to_string())
                .collect::<Vec<_>>(),
        )),
    }
}

fn ap_change(app: &mut App, device: &ObjectPath, changed: PropMap, invalidated: &[&str]) {
    let ap = app.resource::<NMState>().wifi_devices[device]
        .active_ap
        .as_ref()
        .unwrap();
    let event = ap_properties_signal(
        &ap.path,
        ap.properties_changed.unwrap(),
        changed,
        invalidated,
    );
    app.signal(event);
    app.flush();
}

fn assert_device(app: &App, path: &ObjectPath, ap: Option<(&ObjectPath, &[u8], u8)>) {
    let state = app.resource::<NMState>();
    let device = state.wifi_devices.get(path).expect("missing Wi-Fi device");
    let state_match = device.state_changed.expect("missing state subscription");
    let wireless_match = device
        .wireless_properties_changed
        .expect("missing wireless subscription");
    assert_ne!(state_match, wireless_match);
    assert_eq!(state_match.bus, BusId::System);
    assert_eq!(wireless_match.bus, BusId::System);
    assert!(matches!(state.subscriptions.get(&state_match),
        Some(Subscription::DeviceState(owner)) if owner == path));
    assert!(matches!(state.subscriptions.get(&wireless_match),
        Some(Subscription::WirelessProperties(owner)) if owner == path));
    match ap {
        None => assert!(device.active_ap.is_none()),
        Some((ap_path, ssid, strength)) => {
            let ap = device.active_ap.as_ref().expect("missing access point");
            assert_eq!(&ap.path, ap_path);
            assert_eq!(ap.ssid, ssid);
            assert_eq!(ap.strength, strength);
            let ap_match = ap.properties_changed.expect("missing AP subscription");
            assert_eq!(ap_match.bus, BusId::System);
            assert_ne!(ap_match, state_match);
            assert_ne!(ap_match, wireless_match);
            assert!(matches!(state.subscriptions.get(&ap_match),
                Some(Subscription::AccessPointProperties(owner)) if owner == path));
        }
    }
}

fn setup() -> (TestApp, UnixStream) {
    let (mut app, peer) = crate::test_bus::system_bus();
    app.add_module(NetworkManagerModule);
    (TestApp(app), peer)
}

fn initialized() -> (TestApp, UnixStream, ObjectPath, ObjectPath) {
    let device = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/10");
    let (mut app, peer) = setup();
    connected_device(&mut app, &device);
    initialize_ap(&mut app, &device, &ap, b"Office", 70);
    (app, peer, device, ap)
}

fn subscription_ids(app: &App) -> HashSet<MatchId> {
    app.resource::<NMState>()
        .subscriptions
        .keys()
        .copied()
        .collect()
}

fn assert_initialized(app: &App, device: &ObjectPath, ap: &ObjectPath) {
    assert_device(app, device, Some((ap, b"Office", 70)));
    let state = app.resource::<NMState>();
    assert!(state.networking_enabled && state.wireless_hardware_enabled && state.wireless_enabled);
    assert_eq!(state.current_device.as_ref(), Some(device));
    assert_eq!(state.wifi_devices.len(), 1);
    assert_eq!(state.wifi_devices[device].state, nm_device_state::ACTIVE);
    assert_eq!(state.subscriptions.len(), 3);
    assert!(state.pending.is_empty());
}

fn properties_message(path: &ObjectPath, interface: &str, changed: PropMap) -> Message {
    Message::signal(
        path.clone(),
        "org.freedesktop.DBus.Properties",
        "PropertiesChanged",
    )
    .with_sender(NM)
    .with_body(&(interface, changed, Vec::<String>::new()))
}

#[test]
fn get_all_reads_networking_hardware_and_wireless_flags_independently() {
    let (mut app, _peer) = setup();
    app.init_resource::<StateUpdates>()
        .system(record_state_update);
    let call = *app.resource::<NMState>().pending.keys().next().unwrap();
    let cases = [
        (Some(Value::Bool(true)), true),
        (Some(Value::Bool(false)), false),
        (None, false),
        (Some(Value::Str("true".into())), false),
    ];
    for (networking, networking_expected) in cases.clone() {
        for (hardware, hardware_expected) in cases.clone() {
            for (wireless, wireless_expected) in cases.clone() {
                app.resource_mut::<NMState>()
                    .pending
                    .insert(call, CallType::GetAll);
                let mut properties = PropMap::new();
                if let Some(value) = networking.clone() {
                    properties.insert("NetworkingEnabled".into(), value);
                }
                if let Some(value) = hardware.clone() {
                    properties.insert("WirelessHardwareEnabled".into(), value);
                }
                if let Some(value) = wireless {
                    properties.insert("WirelessEnabled".into(), value);
                }
                app.signal(MethodReply {
                    call,
                    result: Ok(Message::method_return(call.serial).with_body(&(properties,))),
                });
                app.flush();

                let state = app.resource::<NMState>();
                assert_eq!(state.networking_enabled, networking_expected);
                assert_eq!(state.wireless_hardware_enabled, hardware_expected);
                assert_eq!(state.wireless_enabled, wireless_expected);
                assert!(state.pending.is_empty());
                let expected = if !networking_expected || !hardware_expected {
                    vec![WifiState::Unavailable]
                } else if !wireless_expected {
                    vec![WifiState::Off]
                } else {
                    Vec::new()
                };
                assert_eq!(
                    std::mem::take(&mut app.resource_mut::<StateUpdates>().0),
                    expected
                );
            }
        }
    }
}

#[test]
fn get_all_publishes_a_device_discovered_before_the_manager_reply() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    for (device_state, expected) in [
        (nm_device_state::UNKNOWN, WifiState::On),
        (nm_device_state::UNMANAGED, WifiState::Unavailable),
        (nm_device_state::UNAVAILABLE, WifiState::Unavailable),
        (nm_device_state::DISCONNECTED, WifiState::Disconnected),
        (nm_device_state::PREPARE, WifiState::Connecting),
        (nm_device_state::CONFIG, WifiState::Connecting),
        (nm_device_state::NEED_AUTH, WifiState::Connecting),
        (nm_device_state::IP_CONFIG, WifiState::Connecting),
        (nm_device_state::IP_CHECK, WifiState::Connecting),
        (nm_device_state::SECONDARIES, WifiState::Connecting),
        (nm_device_state::ACTIVE, WifiState::Connected),
        (nm_device_state::DEACTIVATING, WifiState::Disconnecting),
        (nm_device_state::FAILED, WifiState::Disconnected),
        (999, WifiState::On),
    ] {
        let (mut app, _peer) = setup();
        app.init_resource::<StateUpdates>()
            .system(record_state_update);
        add_device(&mut app, &path);
        let call = device_call(&app, &path);
        reply_properties(
            &mut app,
            call,
            device_properties(nm_device_type::WIFI, device_state),
        );
        assert_eq!(
            app.resource::<NMState>().current_device.as_ref(),
            Some(&path)
        );
        assert!(app.resource::<StateUpdates>().0.is_empty());

        let call = pending_call(&app, |kind| matches!(kind, CallType::GetAll));
        reply_properties(
            &mut app,
            call,
            manager_properties(std::slice::from_ref(&path)),
        );
        assert_eq!(app.resource::<StateUpdates>().0, vec![expected]);

        // The manager snapshot queues another discovery read, but the existing
        // device guard must preserve both its state and the single UI update.
        let call = device_call(&app, &path);
        reply_properties(
            &mut app,
            call,
            device_properties(nm_device_type::WIFI, device_state),
        );
        assert_eq!(app.resource::<StateUpdates>().0, vec![expected]);
        assert_eq!(app.resource::<NMState>().wifi_devices.len(), 1);
    }
}

#[test]
fn get_all_refresh_uses_latest_state_of_only_the_selected_device() {
    let selected = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let other = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/2");
    let (mut app, _peer) = setup();
    app.init_resource::<StateUpdates>()
        .system(record_state_update);
    for path in [&selected, &other] {
        add_device(&mut app, path);
        let call = device_call(&app, path);
        reply_properties(
            &mut app,
            call,
            device_properties(nm_device_type::WIFI, nm_device_state::ACTIVE),
        );
    }
    let id = app.resource::<NMState>().wifi_devices[&selected]
        .state_changed
        .unwrap();
    app.signal(state_changed_signal(
        &selected,
        id,
        nm_device_state::DISCONNECTED,
        nm_device_state::ACTIVE,
    ));
    app.flush();
    assert!(app.resource::<StateUpdates>().0.is_empty());

    let call = pending_call(&app, |kind| matches!(kind, CallType::GetAll));
    reply_properties(
        &mut app,
        call,
        manager_properties(&[selected.clone(), other.clone()]),
    );
    assert_eq!(
        app.resource::<StateUpdates>().0,
        vec![WifiState::Disconnected]
    );
    let state = app.resource::<NMState>();
    assert_eq!(state.current_device.as_ref(), Some(&selected));
    assert_eq!(state.wifi_devices[&other].state, nm_device_state::ACTIVE);
}

#[test]
fn get_all_manager_flags_override_an_already_connected_device() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    for (networking, hardware, wireless, expected) in [
        (false, false, false, WifiState::Unavailable),
        (false, false, true, WifiState::Unavailable),
        (false, true, false, WifiState::Unavailable),
        (false, true, true, WifiState::Unavailable),
        (true, false, false, WifiState::Unavailable),
        (true, false, true, WifiState::Unavailable),
        (true, true, false, WifiState::Off),
        (true, true, true, WifiState::Connected),
    ] {
        let (mut app, _peer) = setup();
        app.init_resource::<StateUpdates>()
            .system(record_state_update);
        add_device(&mut app, &path);
        let call = device_call(&app, &path);
        reply_properties(
            &mut app,
            call,
            device_properties(nm_device_type::WIFI, nm_device_state::ACTIVE),
        );
        let call = pending_call(&app, |kind| matches!(kind, CallType::GetAll));
        let mut properties = manager_properties(std::slice::from_ref(&path));
        properties.insert("NetworkingEnabled".into(), Value::Bool(networking));
        properties.insert("WirelessHardwareEnabled".into(), Value::Bool(hardware));
        properties.insert("WirelessEnabled".into(), Value::Bool(wireless));
        reply_properties(&mut app, call, properties);
        assert_eq!(app.resource::<StateUpdates>().0, vec![expected]);
    }
}

#[test]
fn install_starts_with_disabled_flags_manager_matches_and_one_request() {
    let (app, _peer) = setup();
    let state = app.resource::<NMState>();
    assert!(!state.networking_enabled);
    assert!(!state.wireless_hardware_enabled);
    assert!(!state.wireless_enabled);
    assert!(state.wifi_devices.is_empty());
    assert!(state.current_device.is_none());
    assert!(state.subscriptions.is_empty());
    assert_eq!(state.pending.len(), 1);
    pending_call(&app, |kind| matches!(kind, CallType::GetAll));
    let ids = [
        state.properties_changed.unwrap(),
        state.device_added.unwrap(),
        state.device_removed.unwrap(),
    ];
    assert!(ids.iter().all(|id| id.bus == BusId::System));
    assert_eq!(ids.into_iter().collect::<HashSet<_>>().len(), 3);
}

#[test]
fn empty_missing_or_invalid_devices_leave_discovery_empty() {
    for devices in [
        Some(Value::Array {
            elem: "o".into(),
            items: Vec::new(),
        }),
        None,
        Some(Value::Str("not an object-path array".into())),
    ] {
        let (mut app, _peer) = setup();
        let mut properties = manager_properties(&[]);
        properties.remove("Devices");
        if let Some(devices) = devices {
            properties.insert("Devices".into(), devices);
        }
        let call = pending_call(&app, |kind| matches!(kind, CallType::GetAll));
        reply_properties(&mut app, call, properties);
        let state = app.resource::<NMState>();
        assert!(
            state.networking_enabled && state.wireless_hardware_enabled && state.wireless_enabled
        );
        assert!(state.pending.is_empty());
        assert!(state.wifi_devices.is_empty());
        assert!(state.current_device.is_none());
        assert!(state.subscriptions.is_empty());
    }
}

#[test]
fn failed_or_malformed_manager_reply_leaves_initial_state() {
    let failures = [
        Some(CallError::Remote {
            name: "org.freedesktop.DBus.Error.ServiceUnknown".into(),
            message: "NetworkManager is not installed".into(),
        }),
        Some(CallError::Remote {
            name: "org.freedesktop.DBus.Error.NameHasNoOwner".into(),
            message: "NetworkManager is not running".into(),
        }),
        Some(CallError::Timeout),
        Some(CallError::Disconnected),
        None,
    ];
    for failure in failures {
        let (mut app, _peer) = setup();
        let call = pending_call(&app, |kind| matches!(kind, CallType::GetAll));
        let result = match failure {
            Some(error) => Err(error),
            None => Ok(Message::method_return(call.serial).with_body(&(false,))),
        };
        reply(&mut app, call, result);
        let state = app.resource::<NMState>();
        assert!(!state.networking_enabled);
        assert!(!state.wireless_hardware_enabled);
        assert!(!state.wireless_enabled);
        assert!(state.pending.is_empty());
        assert!(state.wifi_devices.is_empty());
        assert!(state.current_device.is_none());
        assert!(state.subscriptions.is_empty());
        assert!(state.properties_changed.is_some());
        assert!(state.device_added.is_some());
        assert!(state.device_removed.is_some());
    }
}

#[test]
fn non_wifi_missing_or_invalid_device_type_is_not_stored() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    for device_type in [
        Some(Value::U32(0)),
        Some(Value::U32(1)),
        Some(Value::U32(5)),
        None,
        Some(Value::Str("2".into())),
    ] {
        let (mut app, _peer) = setup();
        discover(&mut app, std::slice::from_ref(&path));
        let call = device_call(&app, &path);
        let mut properties = PropMap::from([("State".into(), Value::U32(100))]);
        if let Some(device_type) = device_type {
            properties.insert("DeviceType".into(), device_type);
        }
        reply_properties(&mut app, call, properties);
        let state = app.resource::<NMState>();
        assert!(state.pending.is_empty());
        assert!(state.wifi_devices.is_empty());
        assert!(state.current_device.is_none());
        assert!(state.subscriptions.is_empty());
    }
}

#[test]
fn current_device_stays_none_until_a_wifi_device_is_confirmed() {
    let ethernet = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let wifi = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/2");
    for wifi_state in [30, 100] {
        let (mut app, _peer) = setup();
        assert!(app.resource::<NMState>().current_device.is_none());

        discover(&mut app, &[ethernet.clone(), wifi.clone()]);
        assert!(app.resource::<NMState>().current_device.is_none());
        assert_eq!(app.resource::<NMState>().pending.len(), 2);

        let call = device_call(&app, &ethernet);
        reply_properties(&mut app, call, device_properties(1, 100));
        let state = app.resource::<NMState>();
        assert!(state.current_device.is_none());
        assert!(state.wifi_devices.is_empty());
        assert!(state.subscriptions.is_empty());
        assert_eq!(state.pending.len(), 1);

        let call = device_call(&app, &wifi);
        reply_properties(&mut app, call, device_properties(2, wifi_state));
        let state = app.resource::<NMState>();
        assert_eq!(state.current_device.as_ref(), Some(&wifi));
        assert!(state.wifi_devices.contains_key(&wifi));
        assert_eq!(state.wifi_devices.len(), 1);
        assert_eq!(state.pending.len(), usize::from(wifi_state == 100));
        assert_device(&app, &wifi, None);
    }
}

#[test]
fn current_device_stays_none_when_all_device_reads_fail() {
    let malformed = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let timed_out = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/2");
    let disappeared = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/3");
    let (mut app, _peer) = setup();
    discover(
        &mut app,
        &[malformed.clone(), timed_out.clone(), disappeared.clone()],
    );
    assert!(app.resource::<NMState>().current_device.is_none());

    let malformed_call = device_call(&app, &malformed);
    let timed_out_call = device_call(&app, &timed_out);
    let disappeared_call = device_call(&app, &disappeared);
    let replies = [
        (
            disappeared_call,
            Err(CallError::Remote {
                name: "org.freedesktop.DBus.Error.UnknownObject".into(),
                message: "device disappeared".into(),
            }),
        ),
        (
            malformed_call,
            Ok(Message::method_return(malformed_call.serial).with_body(&(false,))),
        ),
        (timed_out_call, Err(CallError::Timeout)),
    ];
    for (index, (call, result)) in replies.into_iter().enumerate() {
        reply(&mut app, call, result);
        let state = app.resource::<NMState>();
        assert!(state.current_device.is_none());
        assert!(state.wifi_devices.is_empty());
        assert!(state.subscriptions.is_empty());
        assert_eq!(state.pending.len(), 2 - index);
    }
}

#[test]
fn wifi_state_controls_initial_ap_read_but_not_device_subscriptions() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let mut cases: Vec<_> = [
        (nm_device_state::UNKNOWN, WifiState::On),
        (nm_device_state::UNMANAGED, WifiState::Unavailable),
        (nm_device_state::UNAVAILABLE, WifiState::Unavailable),
        (nm_device_state::DISCONNECTED, WifiState::Disconnected),
        (nm_device_state::PREPARE, WifiState::Connecting),
        (nm_device_state::CONFIG, WifiState::Connecting),
        (nm_device_state::NEED_AUTH, WifiState::Connecting),
        (nm_device_state::IP_CONFIG, WifiState::Connecting),
        (nm_device_state::IP_CHECK, WifiState::Connecting),
        (nm_device_state::SECONDARIES, WifiState::Connecting),
        (nm_device_state::ACTIVE, WifiState::Connected),
        (nm_device_state::DEACTIVATING, WifiState::Disconnecting),
        (nm_device_state::FAILED, WifiState::Disconnected),
        (999, WifiState::On),
    ]
    .into_iter()
    .map(|(state, event)| (Some(Value::U32(state)), state, event))
    .collect();
    cases.extend([
        (None, nm_device_state::UNKNOWN, WifiState::On),
        (
            Some(Value::Str("100".into())),
            nm_device_state::UNKNOWN,
            WifiState::On,
        ),
    ]);
    for (value, expected_state, expected_event) in cases {
        let activated = expected_state == nm_device_state::ACTIVE;
        let (mut app, _peer) = setup();
        discover(&mut app, std::slice::from_ref(&path));
        app.init_resource::<StateUpdates>()
            .system(record_state_update);
        let call = device_call(&app, &path);
        let mut properties = PropMap::from([("DeviceType".into(), Value::U32(2))]);
        if let Some(value) = value {
            properties.insert("State".into(), value);
        }
        reply_properties(&mut app, call, properties);
        assert_device(&app, &path, None);
        let state = app.resource::<NMState>();
        assert_eq!(state.wifi_devices.len(), 1);
        assert_eq!(state.wifi_devices[&path].state, expected_state);
        assert_eq!(state.subscriptions.len(), 2);
        assert_eq!(state.pending.len(), usize::from(activated));
        assert_eq!(app.resource::<StateUpdates>().0, vec![expected_event]);
        if activated {
            active_ap_call(&app, &path);
        }
    }
}

#[test]
fn device_properties_emits_only_for_selected_wifi_with_all_manager_flags_enabled() {
    let ethernet = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let selected = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/2");
    let other = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/3");
    for flags in 0..8 {
        let (mut app, _peer) = setup();
        let mut properties =
            manager_properties(&[ethernet.clone(), selected.clone(), other.clone()]);
        properties.insert("NetworkingEnabled".into(), Value::Bool(flags & 1 != 0));
        properties.insert(
            "WirelessHardwareEnabled".into(),
            Value::Bool(flags & 2 != 0),
        );
        properties.insert("WirelessEnabled".into(), Value::Bool(flags & 4 != 0));
        let call = pending_call(&app, |kind| matches!(kind, CallType::GetAll));
        reply_properties(&mut app, call, properties);
        app.init_resource::<StateUpdates>()
            .system(record_state_update);

        let call = device_call(&app, &ethernet);
        reply_properties(
            &mut app,
            call,
            device_properties(1, nm_device_state::ACTIVE),
        );
        assert!(app.resource::<StateUpdates>().0.is_empty());

        let call = device_call(&app, &selected);
        reply_properties(
            &mut app,
            call,
            device_properties(nm_device_type::WIFI, nm_device_state::DISCONNECTED),
        );
        let expected = if flags == 7 {
            vec![WifiState::Disconnected]
        } else {
            Vec::new()
        };
        assert_eq!(app.resource::<StateUpdates>().0, expected);

        let call = device_call(&app, &other);
        reply_properties(
            &mut app,
            call,
            device_properties(nm_device_type::WIFI, nm_device_state::ACTIVE),
        );
        assert_eq!(app.resource::<StateUpdates>().0, expected);
        let state = app.resource::<NMState>();
        assert_eq!(state.current_device.as_ref(), Some(&selected));
        assert_eq!(
            state.wifi_devices[&selected].state,
            nm_device_state::DISCONNECTED
        );
        assert_eq!(state.wifi_devices[&other].state, nm_device_state::ACTIVE);
        assert_eq!(state.subscriptions.len(), 4);
        // Suppressing UI events still permits discovery and AP initialization.
        active_ap_call(&app, &other);
    }
}

#[test]
fn ap_properties_emits_only_for_selected_device_with_all_manager_flags_enabled() {
    let selected = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let other = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/2");
    let selected_ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/1");
    let other_ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/2");
    let ssid = b"wifi\xff";
    for flags in 0..8 {
        let (mut app, _peer) = setup();
        discover(&mut app, &[selected.clone(), other.clone()]);
        for path in [&selected, &other] {
            let call = device_call(&app, path);
            reply_properties(
                &mut app,
                call,
                device_properties(nm_device_type::WIFI, nm_device_state::ACTIVE),
            );
        }
        {
            let mut state = app.resource_mut::<NMState>();
            state.networking_enabled = flags & 1 != 0;
            state.wireless_hardware_enabled = flags & 2 != 0;
            state.wireless_enabled = flags & 4 != 0;
        }
        app.init_resource::<APUpdates>().system(record_ap_update);

        initialize_ap(&mut app, &other, &other_ap, b"other", 80);
        assert!(app.resource::<APUpdates>().0.is_empty());

        initialize_ap(&mut app, &selected, &selected_ap, ssid, 61);
        let expected = if flags == 7 {
            vec![APUpdate {
                ssid: Some(ssid.to_vec()),
                strength: 61,
            }]
        } else {
            Vec::new()
        };
        assert_eq!(app.resource::<APUpdates>().0, expected);
    }
}

#[test]
fn state_changed_updates_only_its_device_and_keeps_ap_tracking() {
    let selected = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let other = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/2");
    let selected_ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/10");
    let other_ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/20");
    let (mut app, _peer) = setup();
    discover(&mut app, &[selected.clone(), other.clone()]);
    for (path, ap) in [(&selected, &selected_ap), (&other, &other_ap)] {
        let call = device_call(&app, path);
        reply_properties(
            &mut app,
            call,
            device_properties(nm_device_type::WIFI, nm_device_state::ACTIVE),
        );
        read_active_ap(&mut app, path, ap);
        let call = ap_properties_call(&app, path, ap);
        reply_properties(
            &mut app,
            call,
            PropMap::from([
                ("Ssid".into(), ssid_value(b"Office")),
                ("Strength".into(), Value::U8(75)),
            ]),
        );
    }

    app.init_resource::<StateUpdates>()
        .system(record_state_update);
    for (target, untouched) in [(&other, &selected), (&selected, &other)] {
        let id = app.resource::<NMState>().wifi_devices[target]
            .state_changed
            .unwrap();
        let mut old_state = nm_device_state::ACTIVE;
        for (new_state, expected) in [
            (nm_device_state::DEACTIVATING, WifiState::Disconnecting),
            (nm_device_state::DISCONNECTED, WifiState::Disconnected),
            (nm_device_state::UNKNOWN, WifiState::On),
            (nm_device_state::UNMANAGED, WifiState::Unavailable),
            (nm_device_state::UNAVAILABLE, WifiState::Unavailable),
            (nm_device_state::PREPARE, WifiState::Connecting),
            (nm_device_state::FAILED, WifiState::Disconnected),
            (nm_device_state::ACTIVE, WifiState::Connected),
            (nm_device_state::ACTIVE, WifiState::Connected), // Repeated notifications still emit.
        ] {
            app.signal(state_changed_signal(target, id, new_state, old_state));
            app.flush();
            let state = app.resource::<NMState>();
            assert_eq!(state.wifi_devices[target].state, new_state);
            assert_eq!(state.wifi_devices[untouched].state, nm_device_state::ACTIVE);
            assert_eq!(state.current_device.as_ref(), Some(&selected));
            assert_eq!(state.subscriptions.len(), 6);
            assert!(state.pending.is_empty());
            assert_device(&app, &selected, Some((&selected_ap, b"Office", 75)));
            assert_device(&app, &other, Some((&other_ap, b"Office", 75)));
            assert_eq!(
                std::mem::take(&mut app.resource_mut::<StateUpdates>().0),
                if target == &selected {
                    vec![expected]
                } else {
                    Vec::new()
                }
            );
            old_state = new_state;
        }
    }
}

#[test]
fn state_changed_updates_cache_but_emits_only_when_all_manager_flags_are_enabled() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let (mut app, _peer) = setup();
    connected_device(&mut app, &path);
    app.init_resource::<StateUpdates>()
        .system(record_state_update);
    let id = app.resource::<NMState>().wifi_devices[&path]
        .state_changed
        .unwrap();
    let mut old_state = nm_device_state::ACTIVE;
    for flags in 0..8 {
        {
            let mut state = app.resource_mut::<NMState>();
            state.networking_enabled = flags & 1 != 0;
            state.wireless_hardware_enabled = flags & 2 != 0;
            state.wireless_enabled = flags & 4 != 0;
        }
        let (new_state, expected) = if flags % 2 == 0 {
            (nm_device_state::DISCONNECTED, WifiState::Disconnected)
        } else {
            (nm_device_state::PREPARE, WifiState::Connecting)
        };
        app.signal(state_changed_signal(&path, id, new_state, old_state));
        app.flush();
        assert_eq!(
            app.resource::<NMState>().wifi_devices[&path].state,
            new_state
        );
        assert_eq!(
            std::mem::take(&mut app.resource_mut::<StateUpdates>().0),
            if flags == 7 {
                vec![expected]
            } else {
                Vec::new()
            }
        );
        old_state = new_state;
    }
}

#[test]
fn malformed_and_wrong_bus_state_changes_preserve_stored_state() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let (mut app, _peer) = setup();
    connected_device(&mut app, &path);
    let id = app.resource::<NMState>().wifi_devices[&path]
        .state_changed
        .unwrap();
    for msg in [
        Message::signal(path.clone(), NM_DEVICE, "StateChanged")
            .with_body(&(nm_device_state::DISCONNECTED, nm_device_state::ACTIVE)),
        Message::signal(path.clone(), NM_DEVICE, "StateChanged").with_body(&(
            "disconnected",
            nm_device_state::ACTIVE,
            1u32,
        )),
    ] {
        app.signal(SignalReceived {
            bus: BusId::System,
            matches: [id].into_iter().collect(),
            msg: msg.with_sender(NM),
        });
        app.flush();
        assert_eq!(
            app.resource::<NMState>().wifi_devices[&path].state,
            nm_device_state::ACTIVE
        );
    }

    let mut event = state_changed_signal(
        &path,
        id,
        nm_device_state::DISCONNECTED,
        nm_device_state::ACTIVE,
    );
    event.bus = BusId::Session;
    app.signal(event);
    app.flush();
    assert_eq!(
        app.resource::<NMState>().wifi_devices[&path].state,
        nm_device_state::ACTIVE
    );
    assert_device(&app, &path, None);
    assert_eq!(app.resource::<NMState>().subscriptions.len(), 2);
    active_ap_call(&app, &path);
}

#[test]
fn wireless_ap_replacement_cleans_only_its_device_and_ignores_late_reply() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let other = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/2");
    let old_ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/10");
    let other_ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/20");
    let new_ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/30");
    let (mut app, _peer) = setup();
    discover(&mut app, &[path.clone(), other.clone()]);
    for (device, ap) in [(&path, &old_ap), (&other, &other_ap)] {
        let call = device_call(&app, device);
        reply_properties(
            &mut app,
            call,
            device_properties(nm_device_type::WIFI, nm_device_state::ACTIVE),
        );
        read_active_ap(&mut app, device, ap);
    }
    let old_call = ap_properties_call(&app, &path, &old_ap);
    let other_call = ap_properties_call(&app, &other, &other_ap);
    app.init_resource::<APUpdates>().system(record_ap_update);
    app.init_resource::<StateUpdates>()
        .system(record_state_update);
    let old_match = app.resource::<NMState>().wifi_devices[&path]
        .active_ap
        .as_ref()
        .unwrap()
        .properties_changed
        .unwrap();

    for value in [
        None,
        Some(Value::Str("bad path type".into())),
        Some(Value::ObjectPath(old_ap.clone())),
    ] {
        let mut changed = PropMap::new();
        if let Some(value) = value {
            changed.insert("ActiveAccessPoint".into(), value);
        }
        wireless_change(&mut app, &path, changed, &[]);
        assert!(app.resource::<APUpdates>().0.is_empty());
        assert_device(&app, &path, Some((&old_ap, b"", 0)));
        assert!(app.resource::<NMState>().pending.contains_key(&old_call));
        assert!(
            app.resource::<NMState>()
                .subscriptions
                .contains_key(&old_match)
        );
    }

    wireless_change(
        &mut app,
        &path,
        PropMap::from([(
            "ActiveAccessPoint".into(),
            Value::ObjectPath(new_ap.clone()),
        )]),
        &[],
    );
    let state = app.resource::<NMState>();
    assert!(!state.pending.contains_key(&old_call));
    assert!(state.pending.contains_key(&other_call));
    assert!(!state.subscriptions.contains_key(&old_match));
    assert_eq!(state.subscriptions.len(), 6);
    assert_eq!(state.current_device.as_ref(), Some(&path));
    assert_eq!(state.wifi_devices[&path].state, nm_device_state::ACTIVE);
    assert_device(&app, &path, Some((&new_ap, b"", 0)));
    assert_device(&app, &other, Some((&other_ap, b"", 0)));
    let new_call = ap_properties_call(&app, &path, &new_ap);
    let new_match = state.wifi_devices[&path]
        .active_ap
        .as_ref()
        .unwrap()
        .properties_changed
        .unwrap();
    assert_ne!(new_match, old_match);
    assert_eq!(
        std::mem::take(&mut app.resource_mut::<APUpdates>().0),
        vec![APUpdate {
            ssid: Some(Vec::new()),
            strength: 0
        }]
    );

    // Repeated notifications preserve both the subscription and its pending read.
    wireless_change(
        &mut app,
        &path,
        PropMap::from([(
            "ActiveAccessPoint".into(),
            Value::ObjectPath(new_ap.clone()),
        )]),
        &[],
    );
    assert_eq!(ap_properties_call(&app, &path, &new_ap), new_call);
    assert_eq!(app.resource::<NMState>().subscriptions.len(), 6);
    assert_eq!(app.resource::<NMState>().pending.len(), 2);
    assert_eq!(
        app.resource::<NMState>().wifi_devices[&path]
            .active_ap
            .as_ref()
            .unwrap()
            .properties_changed,
        Some(new_match)
    );

    app.signal(MethodReply {
        call: old_call,
        result: Ok(
            Message::method_return(old_call.serial).with_body(&(PropMap::from([
                ("Ssid".into(), ssid_value(b"Stale AP")),
                ("Strength".into(), Value::U8(99)),
            ]),)),
        ),
    });
    app.flush();
    assert_device(&app, &path, Some((&new_ap, b"", 0)));
    assert!(app.resource::<APUpdates>().0.is_empty());
    reply_properties(
        &mut app,
        new_call,
        PropMap::from([
            ("Ssid".into(), ssid_value(b"New AP")),
            ("Strength".into(), Value::U8(65)),
        ]),
    );
    assert_device(&app, &path, Some((&new_ap, b"New AP", 65)));
    assert_device(&app, &other, Some((&other_ap, b"", 0)));
    assert_eq!(
        app.resource::<APUpdates>().0,
        vec![APUpdate {
            ssid: Some(b"New AP".to_vec()),
            strength: 65
        }]
    );
    assert!(app.resource::<StateUpdates>().0.is_empty());
}

#[test]
fn wireless_ap_clear_supersedes_pending_initial_and_ap_property_reads() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/10");
    for ap_known in [false, true] {
        for invalidated in [false, true] {
            let (mut app, _peer) = setup();
            connected_device(&mut app, &path);
            let call = if ap_known {
                wireless_change(
                    &mut app,
                    &path,
                    PropMap::from([("ActiveAccessPoint".into(), Value::ObjectPath(ap.clone()))]),
                    &[],
                );
                assert_device(&app, &path, Some((&ap, b"", 0)));
                ap_properties_call(&app, &path, &ap)
            } else {
                active_ap_call(&app, &path)
            };
            app.init_resource::<APUpdates>().system(record_ap_update);
            // Invalidation takes precedence even when a non-root path is supplied.
            wireless_change(
                &mut app,
                &path,
                PropMap::from([(
                    "ActiveAccessPoint".into(),
                    Value::ObjectPath(if invalidated {
                        ap.clone()
                    } else {
                        ObjectPath::from("/")
                    }),
                )]),
                if invalidated {
                    &["ActiveAccessPoint"]
                } else {
                    &[]
                },
            );
            assert_device(&app, &path, None);
            assert!(app.resource::<NMState>().pending.is_empty());
            assert_eq!(app.resource::<NMState>().subscriptions.len(), 2);
            let expected = if ap_known {
                vec![APUpdate {
                    ssid: None,
                    strength: 0,
                }]
            } else {
                Vec::new()
            };
            assert_eq!(app.resource::<APUpdates>().0, expected);

            let message = if ap_known {
                Message::method_return(call.serial)
                    .with_body(&(PropMap::from([("Ssid".into(), ssid_value(b"Stale AP"))]),))
            } else {
                Message::method_return(call.serial).with_body(&(Value::ObjectPath(ap.clone()),))
            };
            app.signal(MethodReply {
                call,
                result: Ok(message),
            });
            app.flush();
            assert_device(&app, &path, None);
            assert!(app.resource::<NMState>().pending.is_empty());
            assert_eq!(app.resource::<NMState>().subscriptions.len(), 2);
            assert_eq!(app.resource::<APUpdates>().0, expected);
        }
    }
}

#[test]
fn ap_signal_events_require_selected_device_and_all_manager_flags() {
    let selected = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let other = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/2");
    let ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/10");
    for flags in 0..8 {
        let (mut app, _peer) = setup();
        discover(&mut app, &[selected.clone(), other.clone()]);
        for path in [&selected, &other] {
            let call = device_call(&app, path);
            reply_properties(
                &mut app,
                call,
                device_properties(nm_device_type::WIFI, nm_device_state::DISCONNECTED),
            );
        }
        {
            let mut state = app.resource_mut::<NMState>();
            state.networking_enabled = flags & 1 != 0;
            state.wireless_hardware_enabled = flags & 2 != 0;
            state.wireless_enabled = flags & 4 != 0;
        }
        app.init_resource::<APUpdates>().system(record_ap_update);
        app.init_resource::<StateUpdates>()
            .system(record_state_update);
        for path in [&other, &selected] {
            wireless_change(
                &mut app,
                path,
                PropMap::from([("ActiveAccessPoint".into(), Value::ObjectPath(ap.clone()))]),
                &[],
            );
            assert_device(&app, path, Some((&ap, b"", 0)));
            ap_properties_call(&app, path, &ap);
            ap_change(&mut app, path, ap_properties(b"Updated", 61), &[]);
            assert_device(&app, path, Some((&ap, b"Updated", 61)));
            wireless_change(&mut app, path, PropMap::new(), &["ActiveAccessPoint"]);
            assert_device(&app, path, None);
            assert!(app.resource::<NMState>().pending.is_empty());
            assert_eq!(
                std::mem::take(&mut app.resource_mut::<APUpdates>().0),
                if flags == 7 && path == &selected {
                    vec![
                        APUpdate {
                            ssid: Some(Vec::new()),
                            strength: 0,
                        },
                        APUpdate {
                            ssid: Some(b"Updated".to_vec()),
                            strength: 61,
                        },
                        APUpdate {
                            ssid: None,
                            strength: 0,
                        },
                    ]
                } else {
                    Vec::new()
                }
            );
            assert!(app.resource::<StateUpdates>().0.is_empty());
        }
    }
}

#[test]
fn wireless_ap_signal_initializes_before_activation_and_supersedes_initial_read() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/10");
    let stale_ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/20");
    for device_state in [nm_device_state::DISCONNECTED, nm_device_state::ACTIVE] {
        let (mut app, _peer) = setup();
        discover(&mut app, std::slice::from_ref(&path));
        let call = device_call(&app, &path);
        reply_properties(
            &mut app,
            call,
            device_properties(nm_device_type::WIFI, device_state),
        );
        let initial_read =
            (device_state == nm_device_state::ACTIVE).then(|| active_ap_call(&app, &path));

        wireless_change(
            &mut app,
            &path,
            PropMap::from([("ActiveAccessPoint".into(), Value::ObjectPath(ap.clone()))]),
            &[],
        );
        assert_device(&app, &path, Some((&ap, b"", 0)));
        assert_eq!(
            app.resource::<NMState>().wifi_devices[&path].state,
            device_state
        );
        assert_eq!(app.resource::<NMState>().subscriptions.len(), 3);
        assert_eq!(app.resource::<NMState>().pending.len(), 1);

        if let Some(call) = initial_read {
            assert!(!app.resource::<NMState>().pending.contains_key(&call));
            app.signal(MethodReply {
                call,
                result: Ok(Message::method_return(call.serial)
                    .with_body(&(Value::ObjectPath(stale_ap.clone()),))),
            });
            app.flush();
            assert_device(&app, &path, Some((&ap, b"", 0)));
        }
        let call = ap_properties_call(&app, &path, &ap);
        reply_properties(
            &mut app,
            call,
            PropMap::from([
                ("Ssid".into(), ssid_value(b"Office")),
                ("Strength".into(), Value::U8(80)),
            ]),
        );
        assert_device(&app, &path, Some((&ap, b"Office", 80)));
        assert!(app.resource::<NMState>().pending.is_empty());
    }
}

#[test]
fn ap_property_changes_decode_fields_independently_and_preserve_raw_ssid() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/10");
    let ssid_cases = [
        (None, b"Office".as_slice()),
        (
            Some(ssid_value(b"Raw\xff\0SSID")),
            b"Raw\xff\0SSID".as_slice(),
        ),
        (Some(ssid_value(b"")), b"".as_slice()),
        (Some(Value::Str("not bytes".into())), b"Office".as_slice()),
        (
            Some(Value::Array {
                elem: "u".into(),
                items: vec![Value::U32(65)],
            }),
            b"Office".as_slice(),
        ),
    ];
    let strength_cases = [
        (None, 70),
        (Some(Value::U8(0)), 0),
        (Some(Value::U8(100)), 100),
        (Some(Value::U32(80)), 70),
        (Some(Value::Bool(true)), 70),
    ];
    for (ssid, expected_ssid) in ssid_cases {
        for (strength, expected_strength) in strength_cases.clone() {
            let (mut app, _peer) = setup();
            connected_device(&mut app, &path);
            initialize_ap(&mut app, &path, &ap, b"Office", 70);
            app.init_resource::<APUpdates>().system(record_ap_update);
            let mut changed = PropMap::new();
            if let Some(value) = ssid.clone() {
                changed.insert("Ssid".into(), value);
            }
            if let Some(value) = strength {
                changed.insert("Strength".into(), value);
            }
            ap_change(&mut app, &path, changed, &[]);
            assert_device(&app, &path, Some((&ap, expected_ssid, expected_strength)));
            assert_eq!(
                app.resource::<APUpdates>().0,
                vec![APUpdate {
                    ssid: Some(expected_ssid.to_vec()),
                    strength: expected_strength,
                }]
            );
            assert!(app.resource::<NMState>().pending.is_empty());
            assert_eq!(app.resource::<NMState>().subscriptions.len(), 3);
        }
    }
}

#[test]
fn ap_property_invalidations_reset_only_named_fields_and_override_supplied_values() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/10");
    for invalidate_ssid in [false, true] {
        for invalidate_strength in [false, true] {
            for (changed, supplied_ssid, supplied_strength) in [
                (PropMap::new(), b"Office".as_slice(), 70),
                (ap_properties(b"New", 90), b"New".as_slice(), 90),
                (
                    PropMap::from([
                        ("Ssid".into(), Value::Bool(false)),
                        ("Strength".into(), Value::Str("invalid".into())),
                    ]),
                    b"Office".as_slice(),
                    70,
                ),
            ] {
                let (mut app, _peer) = setup();
                connected_device(&mut app, &path);
                initialize_ap(&mut app, &path, &ap, b"Office", 70);
                app.init_resource::<APUpdates>().system(record_ap_update);
                let mut invalidated = vec!["Frequency"];
                if invalidate_ssid {
                    invalidated.push("Ssid");
                }
                if invalidate_strength {
                    invalidated.push("Strength");
                }
                ap_change(&mut app, &path, changed, &invalidated);
                assert_device(
                    &app,
                    &path,
                    Some((
                        &ap,
                        if invalidate_ssid { b"" } else { supplied_ssid },
                        if invalidate_strength {
                            0
                        } else {
                            supplied_strength
                        },
                    )),
                );
                assert!(app.resource::<NMState>().pending.is_empty());
                assert_eq!(app.resource::<NMState>().subscriptions.len(), 3);
                assert_eq!(
                    std::mem::take(&mut app.resource_mut::<APUpdates>().0),
                    vec![APUpdate {
                        ssid: Some(if invalidate_ssid {
                            Vec::new()
                        } else {
                            supplied_ssid.to_vec()
                        }),
                        strength: if invalidate_strength {
                            0
                        } else {
                            supplied_strength
                        },
                    }]
                );

                // Invalidations clear values, not the AP or its subscription.
                ap_change(&mut app, &path, ap_properties(b"Recovered", 42), &[]);
                assert_device(&app, &path, Some((&ap, b"Recovered", 42)));
                assert_eq!(
                    app.resource::<APUpdates>().0,
                    vec![APUpdate {
                        ssid: Some(b"Recovered".to_vec()),
                        strength: 42
                    }]
                );
            }
        }
    }
}

#[test]
fn repeated_and_unrelated_ap_changes_preserve_tracking_and_selection() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/10");
    let (mut app, _peer) = setup();
    connected_device(&mut app, &path);
    initialize_ap(&mut app, &path, &ap, b"Office", 70);
    let id = app.resource::<NMState>().wifi_devices[&path]
        .active_ap
        .as_ref()
        .unwrap()
        .properties_changed;
    for _ in 0..2 {
        for changed in [
            PropMap::new(),
            ap_properties(b"Office", 70),
            PropMap::from([
                ("Frequency".into(), Value::U32(5180)),
                ("SSID".into(), ssid_value(b"Wrong case")),
                ("strength".into(), Value::U8(1)),
            ]),
        ] {
            ap_change(&mut app, &path, changed, &["Frequency", "SSID", "strength"]);
            assert_device(&app, &path, Some((&ap, b"Office", 70)));
            let state = app.resource::<NMState>();
            assert_eq!(
                state.wifi_devices[&path]
                    .active_ap
                    .as_ref()
                    .unwrap()
                    .properties_changed,
                id
            );
            assert_eq!(state.wifi_devices[&path].state, nm_device_state::ACTIVE);
            assert_eq!(state.current_device.as_ref(), Some(&path));
            assert_eq!(state.subscriptions.len(), 3);
            assert!(state.pending.is_empty());
        }
    }
}

#[test]
fn malformed_wrong_bus_and_unmatched_ap_signals_preserve_cached_values() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/10");
    let (mut app, _peer) = setup();
    connected_device(&mut app, &path);
    initialize_ap(&mut app, &path, &ap, b"Office", 70);
    let id = app.resource::<NMState>().wifi_devices[&path]
        .active_ap
        .as_ref()
        .unwrap()
        .properties_changed
        .unwrap();
    let signal = || {
        Message::signal(
            ap.clone(),
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
        )
        .with_sender(NM)
    };
    for msg in [
        signal().with_body(&(false,)),
        signal().with_body(&(NM_ACCESS_POINT, ap_properties(b"Ignored", 1))),
        signal().with_body(&(NM_ACCESS_POINT, ap_properties(b"Ignored", 1), true)),
    ] {
        app.signal(SignalReceived {
            bus: BusId::System,
            matches: [id].into_iter().collect(),
            msg,
        });
        app.flush();
        assert_device(&app, &path, Some((&ap, b"Office", 70)));
    }
    let mut wrong_bus =
        ap_properties_signal(&ap, id, ap_properties(b"Ignored", 1), &["Ssid", "Strength"]);
    wrong_bus.bus = BusId::Session;
    app.signal(wrong_bus);
    app.flush();
    assert_device(&app, &path, Some((&ap, b"Office", 70)));

    let mut unmatched =
        ap_properties_signal(&ap, id, ap_properties(b"Ignored", 1), &["Ssid", "Strength"]);
    unmatched.matches.clear();
    app.signal(unmatched);
    app.flush();
    assert_device(&app, &path, Some((&ap, b"Office", 70)));
    assert!(app.resource::<NMState>().pending.is_empty());
    assert_eq!(app.resource::<NMState>().subscriptions.len(), 3);
}

#[test]
fn ap_changes_are_device_local_even_with_matching_ssids_and_different_states() {
    let selected = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let other = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/2");
    let selected_ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/10");
    let other_ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/20");
    let (mut app, _peer) = setup();
    discover(&mut app, &[selected.clone(), other.clone()]);
    for (device, ap, strength) in [(&selected, &selected_ap, 70), (&other, &other_ap, 20)] {
        let call = device_call(&app, device);
        reply_properties(
            &mut app,
            call,
            device_properties(nm_device_type::WIFI, nm_device_state::ACTIVE),
        );
        initialize_ap(&mut app, device, ap, b"Shared SSID", strength);
    }
    let state_match = app.resource::<NMState>().wifi_devices[&other]
        .state_changed
        .unwrap();
    app.signal(state_changed_signal(
        &other,
        state_match,
        nm_device_state::DEACTIVATING,
        nm_device_state::ACTIVE,
    ));
    app.flush();
    ap_change(
        &mut app,
        &other,
        PropMap::from([("Strength".into(), Value::U8(35))]),
        &[],
    );
    assert_device(&app, &selected, Some((&selected_ap, b"Shared SSID", 70)));
    assert_device(&app, &other, Some((&other_ap, b"Shared SSID", 35)));

    ap_change(
        &mut app,
        &selected,
        PropMap::from([("Ssid".into(), ssid_value(b"Renamed"))]),
        &["Strength"],
    );
    assert_device(&app, &selected, Some((&selected_ap, b"Renamed", 0)));
    assert_device(&app, &other, Some((&other_ap, b"Shared SSID", 35)));
    let state = app.resource::<NMState>();
    assert_eq!(state.current_device.as_ref(), Some(&selected));
    assert_eq!(state.wifi_devices[&selected].state, nm_device_state::ACTIVE);
    assert_eq!(
        state.wifi_devices[&other].state,
        nm_device_state::DEACTIVATING
    );
    assert_eq!(state.subscriptions.len(), 6);
    assert!(state.pending.is_empty());
}

#[test]
fn obsolete_ap_signals_are_ignored_after_roaming_clearing_reconnection_and_removal() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let old_ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/10");
    let new_ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/20");
    let (mut app, _peer) = setup();
    connected_device(&mut app, &path);
    initialize_ap(&mut app, &path, &old_ap, b"Old", 70);
    let old_id = app.resource::<NMState>().wifi_devices[&path]
        .active_ap
        .as_ref()
        .unwrap()
        .properties_changed
        .unwrap();
    wireless_change(
        &mut app,
        &path,
        PropMap::from([(
            "ActiveAccessPoint".into(),
            Value::ObjectPath(new_ap.clone()),
        )]),
        &[],
    );
    let call = ap_properties_call(&app, &path, &new_ap);
    reply_properties(&mut app, call, ap_properties(b"New", 60));
    let new_id = app.resource::<NMState>().wifi_devices[&path]
        .active_ap
        .as_ref()
        .unwrap()
        .properties_changed
        .unwrap();
    assert_ne!(old_id, new_id);
    app.signal(ap_properties_signal(
        &old_ap,
        old_id,
        ap_properties(b"Obsolete", 99),
        &["Ssid", "Strength"],
    ));
    app.flush();
    assert_device(&app, &path, Some((&new_ap, b"New", 60)));

    wireless_change(
        &mut app,
        &path,
        PropMap::from([(
            "ActiveAccessPoint".into(),
            Value::ObjectPath(ObjectPath::from("/")),
        )]),
        &[],
    );
    app.signal(ap_properties_signal(
        &new_ap,
        new_id,
        ap_properties(b"Obsolete", 99),
        &[],
    ));
    app.flush();
    assert_device(&app, &path, None);
    assert_eq!(app.resource::<NMState>().subscriptions.len(), 2);

    // Reusing an AP path must not revive the old subscription's queued signals.
    wireless_change(
        &mut app,
        &path,
        PropMap::from([(
            "ActiveAccessPoint".into(),
            Value::ObjectPath(new_ap.clone()),
        )]),
        &[],
    );
    let call = ap_properties_call(&app, &path, &new_ap);
    reply_properties(&mut app, call, ap_properties(b"Reconnected", 55));
    let current_id = app.resource::<NMState>().wifi_devices[&path]
        .active_ap
        .as_ref()
        .unwrap()
        .properties_changed
        .unwrap();
    assert_ne!(current_id, new_id);
    app.signal(ap_properties_signal(
        &new_ap,
        new_id,
        ap_properties(b"Obsolete", 99),
        &[],
    ));
    app.flush();
    assert_device(&app, &path, Some((&new_ap, b"Reconnected", 55)));
    ap_change(
        &mut app,
        &path,
        PropMap::from([("Strength".into(), Value::U8(56))]),
        &[],
    );
    assert_device(&app, &path, Some((&new_ap, b"Reconnected", 56)));

    remove_device(&mut app, &path);
    app.signal(ap_properties_signal(
        &new_ap,
        current_id,
        ap_properties(b"Obsolete", 99),
        &[],
    ));
    app.flush();
    let state = app.resource::<NMState>();
    assert!(state.wifi_devices.is_empty());
    assert!(state.current_device.is_none());
    assert!(state.subscriptions.is_empty());
    assert!(state.pending.is_empty());
}

#[test]
fn ap_signals_work_while_initial_properties_are_pending_and_after_read_failure() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/10");
    let (mut app, _peer) = setup();
    connected_device(&mut app, &path);
    read_active_ap(&mut app, &path, &ap);
    let call = ap_properties_call(&app, &path, &ap);
    ap_change(&mut app, &path, ap_properties(b"Live", 64), &[]);
    assert_device(&app, &path, Some((&ap, b"Live", 64)));
    assert!(app.resource::<NMState>().pending.contains_key(&call));
    reply(&mut app, call, Err(CallError::Timeout));
    assert_device(&app, &path, Some((&ap, b"Live", 64)));

    ap_change(&mut app, &path, PropMap::new(), &["Ssid", "Strength"]);
    assert_device(&app, &path, Some((&ap, b"", 0)));
    ap_change(&mut app, &path, ap_properties(b"Recovered", 81), &[]);
    assert_device(&app, &path, Some((&ap, b"Recovered", 81)));
    assert_eq!(app.resource::<NMState>().subscriptions.len(), 3);
    assert!(app.resource::<NMState>().pending.is_empty());
}

#[test]
fn mixed_devices_initialize_independently_with_out_of_order_replies() {
    let first = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let second = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/2");
    let disconnected = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/3");
    let ethernet = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/4");
    let first_ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/10");
    let second_ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/20");
    let (mut app, _peer) = setup();
    discover(
        &mut app,
        &[
            first.clone(),
            second.clone(),
            disconnected.clone(),
            ethernet.clone(),
        ],
    );
    for (path, device_type, state) in [
        (&disconnected, 2, 30),
        (&second, 2, 100),
        (&ethernet, 1, 100),
        (&first, 2, 100),
    ] {
        let call = device_call(&app, path);
        reply_properties(&mut app, call, device_properties(device_type, state));
        assert_eq!(
            app.resource::<NMState>().current_device.as_ref(),
            Some(&disconnected)
        );
    }
    assert_eq!(app.resource::<NMState>().wifi_devices.len(), 3);
    assert!(
        !app.resource::<NMState>()
            .wifi_devices
            .contains_key(&ethernet)
    );
    assert_eq!(app.resource::<NMState>().subscriptions.len(), 6);
    assert_eq!(app.resource::<NMState>().pending.len(), 2);
    read_active_ap(&mut app, &first, &first_ap);
    read_active_ap(&mut app, &second, &second_ap);
    assert_device(&app, &first, Some((&first_ap, b"", 0)));
    assert_device(&app, &second, Some((&second_ap, b"", 0)));
    assert_eq!(app.resource::<NMState>().subscriptions.len(), 8);
    assert_eq!(app.resource::<NMState>().pending.len(), 2);

    let call = ap_properties_call(&app, &second, &second_ap);
    reply_properties(
        &mut app,
        call,
        PropMap::from([
            ("Ssid".into(), ssid_value(b"Guest")),
            ("Strength".into(), Value::U8(28)),
        ]),
    );
    assert_device(&app, &first, Some((&first_ap, b"", 0)));
    assert_device(&app, &second, Some((&second_ap, b"Guest", 28)));
    assert_eq!(app.resource::<NMState>().pending.len(), 1);

    let call = ap_properties_call(&app, &first, &first_ap);
    reply_properties(
        &mut app,
        call,
        PropMap::from([
            ("Ssid".into(), ssid_value(b"Office\xff\0")),
            ("Strength".into(), Value::U8(93)),
        ]),
    );
    assert_device(&app, &first, Some((&first_ap, b"Office\xff\0", 93)));
    assert_device(&app, &second, Some((&second_ap, b"Guest", 28)));
    assert_device(&app, &disconnected, None);
    let state = app.resource::<NMState>();
    assert_eq!(state.current_device.as_ref(), Some(&disconnected));
    assert!(state.pending.is_empty());
    assert_eq!(state.wifi_devices.len(), 3);
    assert_eq!(state.subscriptions.len(), 8);
    assert!(state.networking_enabled && state.wireless_hardware_enabled && state.wireless_enabled);
}

#[test]
fn root_active_ap_path_leaves_device_without_an_ap() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let (mut app, _peer) = setup();
    connected_device(&mut app, &path);
    read_active_ap(&mut app, &path, &ObjectPath::from("/"));
    assert_device(&app, &path, None);
    let state = app.resource::<NMState>();
    assert!(state.pending.is_empty());
    assert_eq!(state.wifi_devices.len(), 1);
    assert_eq!(state.subscriptions.len(), 2);
}

#[test]
fn malformed_or_failed_active_ap_reply_preserves_device_subscriptions() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let ap_path = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/10");
    for case in 0..4 {
        let (mut app, _peer) = setup();
        connected_device(&mut app, &path);
        let call = active_ap_call(&app, &path);
        let result = match case {
            0 => Ok(Message::method_return(call.serial).with_body(&(ap_path.clone(),))),
            1 => Ok(Message::method_return(call.serial)
                .with_body(&(Value::Str(ap_path.as_str().into()),))),
            2 => Err(CallError::Timeout),
            _ => Err(CallError::Remote {
                name: "org.freedesktop.DBus.Error.UnknownObject".into(),
                message: "device disappeared".into(),
            }),
        };
        reply(&mut app, call, result);
        assert_device(&app, &path, None);
        let state = app.resource::<NMState>();
        assert!(state.pending.is_empty());
        assert_eq!(state.wifi_devices.len(), 1);
        assert_eq!(state.subscriptions.len(), 2);
    }
}

#[test]
fn failed_device_discovery_does_not_discard_other_device_requests() {
    let failed = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let other = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/2");
    for case in 0..3 {
        let (mut app, _peer) = setup();
        discover(&mut app, &[failed.clone(), other.clone()]);
        let call = device_call(&app, &failed);
        let result = match case {
            0 => Ok(Message::method_return(call.serial).with_body(&(false,))),
            1 => Err(CallError::Timeout),
            _ => Err(CallError::Remote {
                name: "org.freedesktop.DBus.Error.UnknownObject".into(),
                message: "device disappeared".into(),
            }),
        };
        reply(&mut app, call, result);
        assert!(app.resource::<NMState>().wifi_devices.is_empty());
        assert!(app.resource::<NMState>().subscriptions.is_empty());
        assert_eq!(app.resource::<NMState>().pending.len(), 1);
        let call = device_call(&app, &other);
        reply_properties(&mut app, call, device_properties(2, 30));
        assert_device(&app, &other, None);
        let state = app.resource::<NMState>();
        assert!(state.pending.is_empty());
        assert_eq!(state.wifi_devices.len(), 1);
        assert!(!state.wifi_devices.contains_key(&failed));
        assert_eq!(state.subscriptions.len(), 2);
    }
}

#[test]
fn ap_properties_initialize_ssid_and_strength_independently() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let ap_path = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/10");
    let ssid_cases = [
        (Some(ssid_value(b"WiFi\xff\0")), b"WiFi\xff\0".to_vec()),
        (Some(ssid_value(b"")), Vec::new()),
        (None, Vec::new()),
        (Some(Value::Str("wrong wire type".into())), Vec::new()),
    ];
    let strength_cases = [
        (Some(Value::U8(0)), 0),
        (Some(Value::U8(73)), 73),
        (Some(Value::U8(100)), 100),
        (None, 0),
        (Some(Value::U32(73)), 0),
    ];
    for (ssid, expected_ssid) in ssid_cases {
        for (strength, expected_strength) in strength_cases.clone() {
            let (mut app, _peer) = setup();
            connected_device(&mut app, &path);
            read_active_ap(&mut app, &path, &ap_path);
            assert_device(&app, &path, Some((&ap_path, b"", 0)));
            assert_eq!(app.resource::<NMState>().pending.len(), 1);
            let call = ap_properties_call(&app, &path, &ap_path);
            let mut properties = PropMap::new();
            if let Some(ssid) = ssid.clone() {
                properties.insert("Ssid".into(), ssid);
            }
            if let Some(strength) = strength {
                properties.insert("Strength".into(), strength);
            }
            reply_properties(&mut app, call, properties);
            assert_device(
                &app,
                &path,
                Some((&ap_path, &expected_ssid, expected_strength)),
            );
            let state = app.resource::<NMState>();
            assert!(state.pending.is_empty());
            assert_eq!(state.wifi_devices.len(), 1);
            assert_eq!(state.subscriptions.len(), 3);
        }
    }
}

#[test]
fn failed_or_malformed_ap_properties_keep_initial_ap_defaults() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let ap_path = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/10");
    for case in 0..3 {
        let (mut app, _peer) = setup();
        connected_device(&mut app, &path);
        read_active_ap(&mut app, &path, &ap_path);
        let call = ap_properties_call(&app, &path, &ap_path);
        let result = match case {
            0 => Ok(Message::method_return(call.serial).with_body(&(false,))),
            1 => Err(CallError::Timeout),
            _ => Err(CallError::Remote {
                name: "org.freedesktop.DBus.Error.UnknownObject".into(),
                message: "access point disappeared".into(),
            }),
        };
        reply(&mut app, call, result);
        assert_device(&app, &path, Some((&ap_path, b"", 0)));
        let state = app.resource::<NMState>();
        assert!(state.pending.is_empty());
        assert_eq!(state.wifi_devices.len(), 1);
        assert_eq!(state.subscriptions.len(), 3);
    }
}

#[test]
fn device_removed_cleans_subscriptions_and_only_replaces_the_removed_selection() {
    let selected = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let other = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/2");
    let remaining = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/3");
    let other_ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/20");
    let remaining_ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/30");
    let (mut app, _peer) = setup();
    discover(
        &mut app,
        &[selected.clone(), other.clone(), remaining.clone()],
    );
    for (path, state) in [(&selected, 30), (&other, 100), (&remaining, 100)] {
        let call = device_call(&app, path);
        reply_properties(&mut app, call, device_properties(2, state));
    }
    read_active_ap(&mut app, &other, &other_ap);
    read_active_ap(&mut app, &remaining, &remaining_ap);
    let other_call = ap_properties_call(&app, &other, &other_ap);
    let remaining_call = ap_properties_call(&app, &remaining, &remaining_ap);
    app.init_resource::<StateUpdates>()
        .system(record_state_update);
    app.init_resource::<APUpdates>().system(record_ap_update);

    remove_device(&mut app, &other);
    assert!(app.resource::<StateUpdates>().0.is_empty());
    assert!(app.resource::<APUpdates>().0.is_empty());
    let state = app.resource::<NMState>();
    assert_eq!(state.current_device.as_ref(), Some(&selected));
    assert_eq!(state.wifi_devices.len(), 2);
    assert!(!state.wifi_devices.contains_key(&other));
    assert_eq!(state.subscriptions.len(), 5);
    assert_eq!(state.pending.len(), 1);
    assert!(state.pending.contains_key(&remaining_call));
    assert_device(&app, &selected, None);
    assert_device(&app, &remaining, Some((&remaining_ap, b"", 0)));

    app.signal(MethodReply {
        call: other_call,
        result: Ok(
            Message::method_return(other_call.serial).with_body(&(PropMap::from([
                ("Ssid".into(), ssid_value(b"Removed AP")),
                ("Strength".into(), Value::U8(100)),
            ]),)),
        ),
    });
    app.flush();
    assert!(!app.resource::<NMState>().wifi_devices.contains_key(&other));
    assert_device(&app, &remaining, Some((&remaining_ap, b"", 0)));

    remove_device(&mut app, &selected);
    let state = app.resource::<NMState>();
    assert_eq!(state.current_device.as_ref(), Some(&remaining));
    assert_eq!(state.wifi_devices.len(), 1);
    assert_eq!(state.subscriptions.len(), 3);
    assert_eq!(state.pending.len(), 1);
    assert_device(&app, &remaining, Some((&remaining_ap, b"", 0)));
    assert_eq!(
        std::mem::take(&mut app.resource_mut::<StateUpdates>().0),
        vec![WifiState::Connected]
    );
    assert_eq!(
        std::mem::take(&mut app.resource_mut::<APUpdates>().0),
        vec![APUpdate {
            ssid: Some(Vec::new()),
            strength: 0
        }]
    );

    remove_device(&mut app, &remaining);
    let state = app.resource::<NMState>();
    assert!(state.current_device.is_none());
    assert!(state.wifi_devices.is_empty());
    assert!(state.subscriptions.is_empty());
    assert!(state.pending.is_empty());
    assert!(state.properties_changed.is_some());
    assert!(state.device_added.is_some());
    assert!(state.device_removed.is_some());
    assert!(app.resource::<StateUpdates>().0.is_empty());
    assert!(app.resource::<APUpdates>().0.is_empty());
}

#[test]
fn device_removed_refreshes_replacement_details_only_when_manager_flags_are_enabled() {
    let selected = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let remaining = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/2");
    let ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/20");
    for flags in 0..8 {
        for has_ap in [false, true] {
            let (mut app, _peer) = setup();
            discover(&mut app, &[selected.clone(), remaining.clone()]);
            for path in [&selected, &remaining] {
                let call = device_call(&app, path);
                reply_properties(
                    &mut app,
                    call,
                    device_properties(nm_device_type::WIFI, nm_device_state::ACTIVE),
                );
            }
            if has_ap {
                initialize_ap(&mut app, &remaining, &ap, b"replacement\xff", 73);
            }
            // Use the replacement's latest state, not its initial discovery state.
            let id = app.resource::<NMState>().wifi_devices[&remaining]
                .state_changed
                .unwrap();
            app.signal(state_changed_signal(
                &remaining,
                id,
                nm_device_state::PREPARE,
                nm_device_state::ACTIVE,
            ));
            app.flush();
            {
                let mut state = app.resource_mut::<NMState>();
                state.networking_enabled = flags & 1 != 0;
                state.wireless_hardware_enabled = flags & 2 != 0;
                state.wireless_enabled = flags & 4 != 0;
            }
            app.init_resource::<StateUpdates>()
                .system(record_state_update);
            app.init_resource::<APUpdates>().system(record_ap_update);

            remove_device(&mut app, &selected);
            assert_eq!(
                app.resource::<NMState>().current_device.as_ref(),
                Some(&remaining)
            );
            assert_device(
                &app,
                &remaining,
                has_ap.then_some((&ap, b"replacement\xff".as_slice(), 73)),
            );
            assert_eq!(
                app.resource::<StateUpdates>().0,
                if flags == 7 {
                    vec![WifiState::Connecting]
                } else {
                    Vec::new()
                }
            );
            assert_eq!(
                app.resource::<APUpdates>().0,
                if flags == 7 && has_ap {
                    vec![APUpdate {
                        ssid: Some(b"replacement\xff".to_vec()),
                        strength: 73,
                    }]
                } else {
                    Vec::new()
                }
            );
        }
    }
}

#[test]
fn device_removed_during_discovery_prevents_late_reply_from_recreating_it() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let (mut app, _peer) = setup();
    discover(&mut app, std::slice::from_ref(&path));
    let call = device_call(&app, &path);
    remove_device(&mut app, &path);
    assert!(app.resource::<NMState>().pending.is_empty());

    app.signal(MethodReply {
        call,
        result: Ok(Message::method_return(call.serial).with_body(&(device_properties(2, 100),))),
    });
    app.flush();
    let state = app.resource::<NMState>();
    assert!(state.current_device.is_none());
    assert!(state.wifi_devices.is_empty());
    assert!(state.subscriptions.is_empty());
    assert!(state.pending.is_empty());
}

#[test]
fn device_removed_clears_pending_active_ap_read() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let (mut app, _peer) = setup();
    connected_device(&mut app, &path);
    let call = active_ap_call(&app, &path);
    let state_match = app.resource::<NMState>().wifi_devices[&path]
        .state_changed
        .unwrap();
    remove_device(&mut app, &path);
    app.signal(state_changed_signal(
        &path,
        state_match,
        nm_device_state::ACTIVE,
        nm_device_state::DISCONNECTED,
    ));
    app.signal(MethodReply {
        call,
        result: Ok(
            Message::method_return(call.serial).with_body(&(Value::ObjectPath(ObjectPath::from(
                "/org/freedesktop/NetworkManager/AccessPoint/10",
            )),)),
        ),
    });
    app.flush();
    let state = app.resource::<NMState>();
    assert!(state.current_device.is_none());
    assert!(state.wifi_devices.is_empty());
    assert!(state.subscriptions.is_empty());
    assert!(state.pending.is_empty());
}

#[test]
fn device_added_uses_discovery_before_storing_or_selecting_wifi() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    for (device_type, device_state) in [(1, 100), (2, 30), (2, 100)] {
        let (mut app, _peer) = setup();
        discover(&mut app, &[]);
        add_device(&mut app, &path);
        let state = app.resource::<NMState>();
        assert!(state.wifi_devices.is_empty());
        assert!(state.current_device.is_none());
        assert!(state.subscriptions.is_empty());
        assert_eq!(state.pending.len(), 1);

        let call = device_call(&app, &path);
        reply_properties(&mut app, call, device_properties(device_type, device_state));
        let state = app.resource::<NMState>();
        if device_type == 2 {
            assert_device(&app, &path, None);
            assert_eq!(state.current_device.as_ref(), Some(&path));
            assert_eq!(state.wifi_devices.len(), 1);
            assert_eq!(state.subscriptions.len(), 2);
            assert_eq!(state.pending.len(), usize::from(device_state == 100));
            if device_state == 100 {
                active_ap_call(&app, &path);
            }
        } else {
            assert!(state.wifi_devices.is_empty());
            assert!(state.current_device.is_none());
            assert!(state.subscriptions.is_empty());
            assert!(state.pending.is_empty());
        }
    }
}

#[test]
fn device_added_overlapping_initial_discovery_keeps_one_device_and_subscription_set() {
    let path = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    for added_reply_first in [true, false] {
        let (mut app, _peer) = setup();
        add_device(&mut app, &path);
        let added_call = device_call(&app, &path);
        let manager_call = pending_call(&app, |kind| matches!(kind, CallType::GetAll));
        reply_properties(
            &mut app,
            manager_call,
            manager_properties(std::slice::from_ref(&path)),
        );
        let initial_call = *app
            .resource::<NMState>()
            .pending
            .keys()
            .find(|call| **call != added_call)
            .unwrap();
        assert_eq!(app.resource::<NMState>().pending.len(), 2);
        let (first, second) = if added_reply_first {
            (added_call, initial_call)
        } else {
            (initial_call, added_call)
        };
        reply_properties(&mut app, first, device_properties(2, 100));
        let ap_call = active_ap_call(&app, &path);
        let state = app.resource::<NMState>();
        let device = &state.wifi_devices[&path];
        let matches = (device.state_changed, device.wireless_properties_changed);

        app.signal(state_changed_signal(
            &path,
            matches.0.unwrap(),
            nm_device_state::DEACTIVATING,
            nm_device_state::ACTIVE,
        ));
        app.flush();
        reply_properties(&mut app, second, device_properties(2, 100));
        assert_device(&app, &path, None);
        let state = app.resource::<NMState>();
        let device = &state.wifi_devices[&path];
        assert_eq!(device.state, nm_device_state::DEACTIVATING);
        assert_eq!(
            matches,
            (device.state_changed, device.wireless_properties_changed)
        );
        assert_eq!(state.current_device.as_ref(), Some(&path));
        assert_eq!(state.wifi_devices.len(), 1);
        assert_eq!(state.subscriptions.len(), 2);
        assert_eq!(state.pending.len(), 1);
        assert!(state.pending.contains_key(&ap_call));
    }
}
#[test]
fn untracked_reply_ids_and_wrong_bus_serial_collisions_do_not_consume_pending_calls() {
    let (mut app, _peer) = setup();
    let pending = pending_call(&app, |kind| matches!(kind, CallType::GetAll));
    for call in [
        CallId {
            bus: BusId::Session,
            serial: pending.serial,
        },
        CallId {
            bus: BusId::System,
            serial: u32::MAX,
        },
    ] {
        for result in [
            Ok(Message::method_return(call.serial).with_body(&(false,))),
            Err(CallError::Remote {
                name: "org.freedesktop.DBus.Error.ServiceUnknown".into(),
                message: "unrelated call failed".into(),
            }),
        ] {
            app.signal(MethodReply { call, result });
            app.flush();
            let state = app.resource::<NMState>();
            assert_eq!(state.pending.len(), 1);
            assert!(state.pending.contains_key(&pending));
            assert!(state.wifi_devices.is_empty());
            assert!(state.current_device.is_none());
            assert!(!state.networking_enabled);
        }
    }
    reply_properties(&mut app, pending, manager_properties(&[]));
    assert!(app.resource::<NMState>().networking_enabled);
    assert!(app.resource::<NMState>().pending.is_empty());
}

#[test]
fn service_absence_errors_clear_all_pending_request_kinds() {
    for name in [
        "org.freedesktop.DBus.Error.ServiceUnknown",
        "org.freedesktop.DBus.Error.NameHasNoOwner",
    ] {
        let (mut app, _peer, path, ap) = initialized();
        let kinds = [
            CallType::GetAll,
            CallType::DeviceProperties(path.clone()),
            CallType::ActiveAccessPoint(path.clone()),
            CallType::AccessPointProperties {
                device_path: path.clone(),
                access_point_path: ap.clone(),
            },
        ];
        // Populate each bookkeeping variant to check that service-wide errors
        // discard all requests, rather than just the call that failed.
        for (index, kind) in kinds.into_iter().enumerate() {
            app.resource_mut::<NMState>().pending.insert(
                CallId {
                    bus: BusId::System,
                    serial: 10_000 + index as u32,
                },
                kind,
            );
        }
        let failed = CallId {
            bus: BusId::System,
            serial: 10_002,
        };
        reply(
            &mut app,
            failed,
            Err(CallError::Remote {
                name: name.into(),
                message: "service disappeared".into(),
            }),
        );
        assert_initialized(&app, &path, &ap);
        // A late snapshot must not overwrite preserved flags once untracked.
        app.signal(MethodReply {
            call: CallId {
                bus: BusId::System,
                serial: 10_000,
            },
            result: Ok(Message::method_return(10_000).with_body(&(PropMap::new(),))),
        });
        app.flush();
        assert_initialized(&app, &path, &ap);
    }
}

#[test]
fn malformed_tracked_replies_preserve_existing_runtime_state() {
    let (mut app, _peer, path, ap) = initialized();
    let ids = subscription_ids(&app);
    for kind in [
        CallType::GetAll,
        CallType::DeviceProperties(path.clone()),
        CallType::ActiveAccessPoint(path.clone()),
        CallType::AccessPointProperties {
            device_path: path.clone(),
            access_point_path: ap.clone(),
        },
    ] {
        let call = CallId {
            bus: BusId::System,
            serial: 10_000,
        };
        app.resource_mut::<NMState>().pending.insert(call, kind);
        reply(
            &mut app,
            call,
            Ok(Message::method_return(call.serial).with_body(&(false,))),
        );
        assert_initialized(&app, &path, &ap);
        assert_eq!(subscription_ids(&app), ids);
    }
}

#[test]
fn malformed_manager_and_wireless_signals_preserve_initialized_state() {
    let (mut app, _peer, path, ap) = initialized();
    let ids = subscription_ids(&app);
    let state = app.resource::<NMState>();
    let cases = [
        (
            state.device_added.unwrap(),
            ObjectPath::from(NM_MAIN_OBJ),
            NM,
            "DeviceAdded",
        ),
        (
            state.device_removed.unwrap(),
            ObjectPath::from(NM_MAIN_OBJ),
            NM,
            "DeviceRemoved",
        ),
        (
            state.properties_changed.unwrap(),
            ObjectPath::from(NM_MAIN_OBJ),
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
        ),
        (
            state.wifi_devices[&path]
                .wireless_properties_changed
                .unwrap(),
            path.clone(),
            "org.freedesktop.DBus.Properties",
            "PropertiesChanged",
        ),
    ];
    for (id, object, interface, member) in cases {
        app.signal(SignalReceived {
            bus: BusId::System,
            matches: [id].into_iter().collect(),
            msg: Message::signal(object, interface, member)
                .with_sender(NM)
                .with_body(&(false,)),
        });
        app.flush();
        assert_initialized(&app, &path, &ap);
        assert_eq!(subscription_ids(&app), ids);
    }
}

#[test]
fn wrong_bus_is_rejected_before_any_manager_or_device_signal_handler() {
    let (mut app, _peer, path, ap) = initialized();
    let ids = subscription_ids(&app);
    let state = app.resource::<NMState>();
    let device = &state.wifi_devices[&path];
    let cases = [
        (
            state.device_added.unwrap(),
            Message::signal(NM_MAIN_OBJ, NM, "DeviceAdded")
                .with_body(&(ObjectPath::from("/new/device"),)),
        ),
        (
            state.device_removed.unwrap(),
            Message::signal(NM_MAIN_OBJ, NM, "DeviceRemoved").with_body(&(path.clone(),)),
        ),
        (
            state.properties_changed.unwrap(),
            properties_message(
                &ObjectPath::from(NM_MAIN_OBJ),
                NM,
                PropMap::from([("NetworkingEnabled".into(), Value::Bool(false))]),
            ),
        ),
        (
            device.state_changed.unwrap(),
            Message::signal(path.clone(), NM_DEVICE, "StateChanged").with_body(&(
                nm_device_state::DISCONNECTED,
                nm_device_state::ACTIVE,
                1u32,
            )),
        ),
        (
            device.wireless_properties_changed.unwrap(),
            properties_message(
                &path,
                NM_WIRELESS,
                PropMap::from([(
                    "ActiveAccessPoint".into(),
                    Value::ObjectPath(ObjectPath::from("/")),
                )]),
            ),
        ),
        (
            device
                .active_ap
                .as_ref()
                .unwrap()
                .properties_changed
                .unwrap(),
            properties_message(&ap, NM_ACCESS_POINT, ap_properties(b"Wrong bus", 0)),
        ),
    ];
    for (id, msg) in cases {
        app.signal(SignalReceived {
            bus: BusId::Session,
            matches: [id].into_iter().collect(),
            msg: msg.with_sender(NM),
        });
        app.flush();
        assert_initialized(&app, &path, &ap);
        assert_eq!(subscription_ids(&app), ids);
    }
}

#[test]
fn unknown_match_is_skipped_and_does_not_hide_a_later_matching_subscription() {
    let (mut app, _peer, path, ap) = initialized();
    // A match owned by another module is valid D-Bus bookkeeping, but unknown to NMState.
    let foreign =
        app.resource_mut::<DBus>()
            .system()
            .watch_properties(NM, ap.clone(), NM_ACCESS_POINT);
    let id = app.resource::<NMState>().wifi_devices[&path]
        .active_ap
        .as_ref()
        .unwrap()
        .properties_changed
        .unwrap();
    let mut event = ap_properties_signal(&ap, foreign, ap_properties(b"Ignored", 0), &[]);
    app.signal(event.clone());
    app.flush();
    assert_initialized(&app, &path, &ap);
    event.matches.push(id);
    app.signal(event);
    app.flush();
    assert_device(&app, &path, Some((&ap, b"Ignored", 0)));
    assert_eq!(app.resource::<NMState>().subscriptions.len(), 3);
    assert!(app.resource::<NMState>().pending.is_empty());
    app.resource_mut::<DBus>().system().remove_match(foreign);
}

#[test]
fn manager_partial_values_and_invalidations_update_only_valid_named_flags() {
    let (mut app, _peer) = setup();
    discover(&mut app, &[]);
    app.init_resource::<StateUpdates>()
        .system(record_state_update);
    let id = app.resource::<NMState>().properties_changed.unwrap();
    for target in [
        "NetworkingEnabled",
        "WirelessHardwareEnabled",
        "WirelessEnabled",
    ] {
        for (value, invalidated, expected) in [
            (None, false, true),
            (Some(Value::Str("false".into())), false, true),
            (Some(Value::Bool(false)), false, false),
            (None, true, false),
            (Some(Value::Bool(true)), true, false),
            (Some(Value::U32(1)), true, false),
        ] {
            let mut state = app.resource_mut::<NMState>();
            state.networking_enabled = true;
            state.wireless_hardware_enabled = true;
            state.wireless_enabled = true;
            let mut changed = PropMap::from([("Unrelated".into(), Value::Bool(false))]);
            if let Some(value) = value {
                changed.insert(target.into(), value);
            }
            app.signal(SignalReceived {
                bus: BusId::System,
                matches: [id].into_iter().collect(),
                msg: Message::signal(
                    NM_MAIN_OBJ,
                    "org.freedesktop.DBus.Properties",
                    "PropertiesChanged",
                )
                .with_sender(NM)
                .with_body(&(
                    NM,
                    changed,
                    if invalidated {
                        vec![target.to_string(), "Unrelated".into()]
                    } else {
                        vec!["Unrelated".into()]
                    },
                )),
            });
            app.flush();
            let state = app.resource::<NMState>();
            for (name, actual) in [
                ("NetworkingEnabled", state.networking_enabled),
                ("WirelessHardwareEnabled", state.wireless_hardware_enabled),
                ("WirelessEnabled", state.wireless_enabled),
            ] {
                assert_eq!(
                    actual,
                    if name == target { expected } else { true },
                    "{name}"
                );
            }
            assert!(state.pending.is_empty());
            assert!(state.wifi_devices.is_empty());
            let expected_events = if expected {
                Vec::new()
            } else if target == "WirelessEnabled" {
                vec![WifiState::Off]
            } else {
                vec![WifiState::Unavailable]
            };
            assert_eq!(
                std::mem::take(&mut app.resource_mut::<StateUpdates>().0),
                expected_events
            );
        }
    }
}

#[test]
fn manager_enable_replays_ap_updates_and_clears_suppressed_during_startup_or_disable() {
    let selected = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let other = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/2");
    let selected_ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/10");
    let other_ap = ObjectPath::from("/org/freedesktop/NetworkManager/AccessPoint/20");
    for disabled_flag in [
        None,
        Some("NetworkingEnabled"),
        Some("WirelessHardwareEnabled"),
        Some("WirelessEnabled"),
    ] {
        for clear_ap in [false, true] {
            let (mut app, _peer) = setup();
            if disabled_flag.is_some() {
                discover(&mut app, &[selected.clone(), other.clone()]);
            } else {
                // Discovery can complete before the initial manager reply.
                add_device(&mut app, &selected);
                add_device(&mut app, &other);
            }
            for (path, ap) in [(&selected, &selected_ap), (&other, &other_ap)] {
                let call = device_call(&app, path);
                reply_properties(
                    &mut app,
                    call,
                    device_properties(nm_device_type::WIFI, nm_device_state::ACTIVE),
                );
                initialize_ap(&mut app, path, ap, b"Old", 20);
            }
            let id = app.resource::<NMState>().properties_changed.unwrap();
            if let Some(flag) = disabled_flag {
                app.signal(SignalReceived {
                    bus: BusId::System,
                    matches: [id].into_iter().collect(),
                    msg: properties_message(
                        &ObjectPath::from(NM_MAIN_OBJ),
                        NM,
                        PropMap::from([(flag.into(), Value::Bool(false))]),
                    ),
                });
                app.flush();
            }
            app.init_resource::<APUpdates>().system(record_ap_update);
            app.init_resource::<StateUpdates>()
                .system(record_state_update);
            ap_change(&mut app, &other, ap_properties(b"Other", 99), &[]);
            let expected_ap = if clear_ap {
                wireless_change(&mut app, &selected, PropMap::new(), &["ActiveAccessPoint"]);
                APUpdate {
                    ssid: None,
                    strength: 0,
                }
            } else {
                ap_change(&mut app, &selected, ap_properties(b"Latest\xff", 73), &[]);
                APUpdate {
                    ssid: Some(b"Latest\xff".to_vec()),
                    strength: 73,
                }
            };
            assert!(app.resource::<APUpdates>().0.is_empty());
            assert!(app.resource::<StateUpdates>().0.is_empty());

            if let Some(flag) = disabled_flag {
                app.signal(SignalReceived {
                    bus: BusId::System,
                    matches: [id].into_iter().collect(),
                    msg: properties_message(
                        &ObjectPath::from(NM_MAIN_OBJ),
                        NM,
                        PropMap::from([(flag.into(), Value::Bool(true))]),
                    ),
                });
                app.flush();
            } else {
                let call = pending_call(&app, |kind| matches!(kind, CallType::GetAll));
                reply_properties(
                    &mut app,
                    call,
                    manager_properties(&[selected.clone(), other.clone()]),
                );
            }
            assert_eq!(app.resource::<StateUpdates>().0, vec![WifiState::Connected]);
            assert_eq!(app.resource::<APUpdates>().0, vec![expected_ap]);
        }
    }
}

#[test]
fn manager_properties_changed_respects_precedence_and_refreshes_latest_selected_state() {
    let selected = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/1");
    let other = ObjectPath::from("/org/freedesktop/NetworkManager/Devices/2");
    let (mut app, _peer) = setup();
    discover(&mut app, &[selected.clone(), other.clone()]);
    for path in [&selected, &other] {
        let call = device_call(&app, path);
        reply_properties(
            &mut app,
            call,
            device_properties(nm_device_type::WIFI, nm_device_state::DISCONNECTED),
        );
    }
    app.init_resource::<StateUpdates>()
        .system(record_state_update);
    let id = app.resource::<NMState>().properties_changed.unwrap();
    let device_id = app.resource::<NMState>().wifi_devices[&selected]
        .state_changed
        .unwrap();
    for flags in 0..8 {
        if flags == 7 {
            // Networking is still disabled from the previous signal. Its
            // re-enabling must publish the state cached during that interval.
            app.signal(state_changed_signal(
                &selected,
                device_id,
                nm_device_state::PREPARE,
                nm_device_state::DISCONNECTED,
            ));
            app.flush();
            assert!(app.resource::<StateUpdates>().0.is_empty());
        }
        app.signal(SignalReceived {
            bus: BusId::System,
            matches: [id].into_iter().collect(),
            msg: properties_message(
                &ObjectPath::from(NM_MAIN_OBJ),
                NM,
                PropMap::from([
                    ("NetworkingEnabled".into(), Value::Bool(flags & 1 != 0)),
                    (
                        "WirelessHardwareEnabled".into(),
                        Value::Bool(flags & 2 != 0),
                    ),
                    ("WirelessEnabled".into(), Value::Bool(flags & 4 != 0)),
                ]),
            ),
        });
        app.flush();
        let expected = if flags & 1 == 0 || flags & 2 == 0 {
            WifiState::Unavailable
        } else if flags & 4 == 0 {
            WifiState::Off
        } else {
            WifiState::Connecting
        };
        assert_eq!(
            std::mem::take(&mut app.resource_mut::<StateUpdates>().0),
            vec![expected]
        );
        let state = app.resource::<NMState>();
        assert_eq!(state.current_device.as_ref(), Some(&selected));
        assert!(state.pending.is_empty());
    }
}

#[test]
fn routed_device_signals_with_missing_owner_do_not_recreate_the_device() {
    for kind in 0..3 {
        let (mut app, _peer, path, ap) = initialized();
        let state = app.resource::<NMState>();
        let device = &state.wifi_devices[&path];
        let (id, subscription, msg) = match kind {
            0 => (
                device.state_changed.unwrap(),
                Subscription::DeviceState(path.clone()),
                Message::signal(path.clone(), NM_DEVICE, "StateChanged").with_body(&(
                    nm_device_state::DISCONNECTED,
                    nm_device_state::ACTIVE,
                    1u32,
                )),
            ),
            1 => (
                device.wireless_properties_changed.unwrap(),
                Subscription::WirelessProperties(path.clone()),
                properties_message(
                    &path,
                    NM_WIRELESS,
                    PropMap::from([("ActiveAccessPoint".into(), Value::ObjectPath(ap.clone()))]),
                ),
            ),
            _ => (
                device
                    .active_ap
                    .as_ref()
                    .unwrap()
                    .properties_changed
                    .unwrap(),
                Subscription::AccessPointProperties(path.clone()),
                properties_message(&ap, NM_ACCESS_POINT, ap_properties(b"Stale", 99)),
            ),
        };
        remove_device(&mut app, &path);
        // Deliberately restore stale routing to reach the missing-owner guard.
        // Ordinary removal clears this entry and exits at the routing-map lookup.
        app.resource_mut::<NMState>()
            .subscriptions
            .insert(id, subscription);
        app.signal(SignalReceived {
            bus: BusId::System,
            matches: [id].into_iter().collect(),
            msg,
        });
        app.flush();
        let state = app.resource::<NMState>();
        assert!(state.wifi_devices.is_empty());
        assert!(state.current_device.is_none());
        assert!(state.pending.is_empty());
        assert_eq!(state.subscriptions.len(), 1);
        assert!(state.subscriptions.contains_key(&id));
    }
}

#[test]
fn routed_ap_signal_without_a_tracked_ap_is_ignored() {
    let (mut app, _peer, path, ap) = initialized();
    let old_ap = app
        .resource_mut::<NMState>()
        .wifi_devices
        .get_mut(&path)
        .unwrap()
        .active_ap
        .take()
        .unwrap();
    // Keep the route deliberately, otherwise only the unknown-match guard runs.
    let ids = subscription_ids(&app);
    app.signal(ap_properties_signal(
        &ap,
        old_ap.properties_changed.unwrap(),
        ap_properties(b"Stale", 99),
        &[],
    ));
    app.flush();
    assert_device(&app, &path, None);
    assert_eq!(subscription_ids(&app), ids);
    assert!(app.resource::<NMState>().pending.is_empty());
}

#[test]
fn active_ap_reply_preserves_an_already_tracked_ap_and_its_subscription() {
    let (mut app, _peer, path, ap) = initialized();
    let ids = subscription_ids(&app);
    let call = CallId {
        bus: BusId::System,
        serial: 10_000,
    };
    // Bypass pending cleanup to exercise the second line of defense directly.
    app.resource_mut::<NMState>()
        .pending
        .insert(call, CallType::ActiveAccessPoint(path.clone()));
    reply(
        &mut app,
        call,
        Ok(Message::method_return(call.serial)
            .with_body(&(Value::ObjectPath(ObjectPath::from("/obsolete/ap")),))),
    );
    assert_initialized(&app, &path, &ap);
    assert_eq!(subscription_ids(&app), ids);
}

#[test]
fn active_ap_reply_for_a_missing_device_does_not_leave_local_tracking() {
    let (mut app, _peer, path, ap) = initialized();
    remove_device(&mut app, &path);
    let call = CallId {
        bus: BusId::System,
        serial: 10_000,
    };
    app.resource_mut::<NMState>()
        .pending
        .insert(call, CallType::ActiveAccessPoint(path));
    reply(
        &mut app,
        call,
        Ok(Message::method_return(call.serial).with_body(&(Value::ObjectPath(ap),))),
    );
    let state = app.resource::<NMState>();
    assert!(state.wifi_devices.is_empty());
    assert!(state.current_device.is_none());
    assert!(state.pending.is_empty());
    assert!(state.subscriptions.is_empty());
}

#[test]
fn ap_property_reply_requires_both_a_device_and_a_matching_tracked_ap() {
    for case in 0..3 {
        let (mut app, _peer, path, ap) = initialized();
        let reply_path = if case == 2 {
            ObjectPath::from("/obsolete/ap")
        } else {
            ap.clone()
        };
        match case {
            0 => remove_device(&mut app, &path),
            1 => wireless_change(
                &mut app,
                &path,
                PropMap::from([(
                    "ActiveAccessPoint".into(),
                    Value::ObjectPath(ObjectPath::from("/")),
                )]),
                &[],
            ),
            _ => {}
        }
        let ids = subscription_ids(&app);
        let call = CallId {
            bus: BusId::System,
            serial: 10_000,
        };
        // Real cleanup removes the pending entry. Restore it to reach each of
        // the reply handler's device/AP/path guards independently.
        app.resource_mut::<NMState>().pending.insert(
            call,
            CallType::AccessPointProperties {
                device_path: path.clone(),
                access_point_path: reply_path,
            },
        );
        reply_properties(&mut app, call, ap_properties(b"Stale", 99));
        assert_eq!(subscription_ids(&app), ids);
        assert!(app.resource::<NMState>().pending.is_empty());
        match case {
            0 => assert!(app.resource::<NMState>().wifi_devices.is_empty()),
            1 => assert_device(&app, &path, None),
            _ => assert_initialized(&app, &path, &ap),
        }
    }
}
