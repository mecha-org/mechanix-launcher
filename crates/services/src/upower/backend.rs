//! Read-only UPower client on mecha-wayland's system D-Bus connection.
use mecha_wayland::prelude::*;

pub(super) mod upower_types {
    type EType = u32;
    pub const UNKNOWN: EType = 0;
    pub const BATTERY: EType = 2;
}

pub(super) struct UpowerBattery {
    pub(super) property_changed: MatchId,
    pub(super) property_read: CallId, // The latest GetAll request, including refreshes.

    pub(super) is_present: bool,
    pub(super) device_type: u32,
}
impl Resource for UpowerBattery {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BatteryPercentageChanged {
    /// event to send when the battery percentage changes
    /// None means the battery is invalid
    pub percentage: Option<u8>,
}
impl Event for BatteryPercentageChanged {}

pub(super) const UPOWER: &str = "org.freedesktop.UPower";
pub(super) const DISPLAY_DEVICE: &str = "/org/freedesktop/UPower/devices/DisplayDevice";
pub(super) const DEVICE_INTERFACE: &str = "org.freedesktop.UPower.Device";

/// Handle the latest GetAll snapshot, whether initial or a refresh.
pub(super) fn on_reply(app: &mut App, e: &MethodReply) {
    if e.call != app.resource::<UpowerBattery>().property_read {
        return;
    }

    let msg = e.result.as_ref().expect("UPower: GetAll failed");
    let (properties,) = msg
        .body::<(PropMap,)>()
        .expect("UPower: invalid GetAll reply");

    {
        let mut bat_resource = app.resource_mut::<UpowerBattery>();
        bat_resource.is_present = properties
            .get_as::<bool>("IsPresent")
            .expect("UPower: missing IsPresent")
            .expect("UPower: invalid IsPresent");

        bat_resource.device_type = properties
            .get_as::<u32>("Type")
            .expect("UPower: missing Type")
            .expect("UPower: invalid Type");
    }

    let percentage = if battery_valid(app) {
        Some(battery_percentage(&properties).expect("UPower: missing Percentage"))
    } else {
        None
    };
    app.emit(
        BatteryPercentageChanged { percentage },
        app.descendants(app.root()).collect::<Vec<_>>(),
    );
}

pub(super) fn on_property_changed(app: &mut App, e: &SignalReceived) {
    if !e
        .matches
        .contains(&app.resource::<UpowerBattery>().property_changed)
    {
        return;
    }

    let (_, changed, invalidated) =
        PropertiesChanged::parse(&e.msg).expect("UPower: invalid PropertiesChanged signal");

    if invalidated
        .iter()
        .any(|name| matches!(name.as_str(), "IsPresent" | "Type" | "Percentage"))
        || changed.contains_key("IsPresent")
        || changed.contains_key("Type")
    {
        let call = app
            .resource_mut::<DBus>()
            .system()
            .object(UPOWER, DISPLAY_DEVICE)
            .get_all(DEVICE_INTERFACE); // handled by on_reply
        app.resource_mut::<UpowerBattery>().property_read = call;
        return;
    }
    if let Some(percentage) = battery_percentage(&changed)
        && battery_valid(app)
    {
        app.emit(
            BatteryPercentageChanged {
                percentage: Some(percentage),
            },
            app.descendants(app.root()).collect::<Vec<_>>(),
        );
    }
}

pub(super) fn battery_valid(app: &App) -> bool {
    let bat_resource = app.resource::<UpowerBattery>();
    bat_resource.is_present && bat_resource.device_type == upower_types::BATTERY
}

/// Check battery percentage from a message. Assume battery is valid
/// None means the passed in message doesn't have a percentage
pub(super) fn battery_percentage(properties: &PropMap) -> Option<u8> {
    let percentage = properties
        .get_as::<f64>("Percentage")?
        .expect("UPower: invalid Percentage");
    assert!(percentage.is_finite(), "UPower: non-finite Percentage");
    Some(percentage.clamp(0.0, 100.0).round() as u8)
}
