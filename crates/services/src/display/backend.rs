//! mxbrightnessd client: property reads, writes and change notifications.
use super::interface::{BRIGHTNESS, INTERFACE, PATH};
use mecha_wayland::prelude::*;

pub(super) struct DisplayStatus {
    pub(super) owner_changed: MatchId,
    pub(super) properties: MatchId,
    pub(super) read: Option<CallId>,
    /// The user set a level after `read` was sent: its `Brightness` is old.
    pub(super) read_stale: bool,
    pub(super) write: Option<CallId>,
    /// Latest value requested while `write` was in flight. Only the newest
    /// is kept, so a fast drag never queues a backlog of stale writes.
    pub(super) pending: Option<f32>,
    /// Last known `Brightness`, or the user's latest request.
    pub(super) value: f32,
    /// Last known `Minimum`.
    pub(super) minimum: f32,
}
impl Resource for DisplayStatus {}

/// mxbrightnessd's perceived brightness scale.
pub const MIN_BRIGHTNESS: f32 = 0.0;
pub const MAX_BRIGHTNESS: f32 = 1.0;

/// Display brightness in `MIN_BRIGHTNESS..=MAX_BRIGHTNESS`, and the lowest
/// level the daemon allows.
pub struct BrightnessChanged {
    pub value: f32,
    pub minimum: f32,
}
impl Event for BrightnessChanged {}

/// Request a brightness in `MIN_BRIGHTNESS..=MAX_BRIGHTNESS`; values below
/// the daemon's minimum are raised to it.
pub struct SetBrightness(pub f32);
impl Signal for SetBrightness {}

pub(super) fn read_properties(app: &mut App) {
    let call = app
        .resource_mut::<DBus>()
        .session()
        .object(BRIGHTNESS, PATH)
        .get_all(INTERFACE);
    let mut status = app.resource_mut::<DisplayStatus>();
    status.read = Some(call);
    status.read_stale = false;
}

pub(super) fn on_set_brightness(app: &mut App, signal: &SetBrightness) {
    if !signal.0.is_finite() {
        return;
    }
    let mut status = app.resource_mut::<DisplayStatus>();
    let value = signal.0.clamp(status.minimum, MAX_BRIGHTNESS);
    status.value = value;
    // A read started before the user's change must not reset the slider.
    status.read_stale = status.read.is_some();
    if status.write.is_some() {
        status.pending = Some(value);
        return;
    }
    write_brightness(app, value);
}

fn write_brightness(app: &mut App, value: f32) {
    let call = app
        .resource_mut::<DBus>()
        .session()
        .object(BRIGHTNESS, PATH)
        .set_property(INTERFACE, "Brightness", Value::F64(value.into()));
    app.resource_mut::<DisplayStatus>().write = Some(call);
}

/// A level property as `0.0..=1.0`, if present and valid.
fn level(properties: &PropMap, name: &str) -> Option<f32> {
    match properties.get_as::<f64>(name)? {
        Ok(value) if value.is_finite() => Some(value.clamp(0.0, 1.0) as f32),
        Ok(value) => {
            eprintln!("[display] invalid {name}: {value}");
            None
        }
        Err(error) => {
            eprintln!("[display] invalid {name}: {error:?}");
            None
        }
    }
}

fn emit_brightness(app: &mut App) {
    let status = app.resource::<DisplayStatus>();
    let event = BrightnessChanged {
        value: status.value,
        minimum: status.minimum,
    };
    app.emit(event, app.descendants(app.root()).collect::<Vec<_>>());
}

pub(super) fn on_reply(app: &mut App, event: &MethodReply) {
    let status = app.resource::<DisplayStatus>();
    if status.read == Some(event.call) {
        let stale = {
            let mut status = app.resource_mut::<DisplayStatus>();
            status.read = None;
            std::mem::take(&mut status.read_stale)
        };
        let properties = event
            .result
            .as_ref()
            .map_err(|error| format!("{error:?}"))
            .and_then(|msg| {
                msg.body::<(PropMap,)>()
                    .map_err(|error| format!("{error:?}"))
            });
        match properties {
            Ok((properties,)) => {
                {
                    let mut status = app.resource_mut::<DisplayStatus>();
                    if let Some(minimum) = level(&properties, "Minimum") {
                        status.minimum = minimum;
                    }
                    if !stale && let Some(value) = level(&properties, "Brightness") {
                        status.value = value;
                    }
                }
                emit_brightness(app);
            }
            Err(error) => eprintln!("[display] reading brightness failed: {error}"),
        }
    } else if status.write == Some(event.call) {
        let pending = {
            let mut status = app.resource_mut::<DisplayStatus>();
            status.write = None;
            status.pending.take()
        };
        if let Some(value) = pending {
            write_brightness(app, value);
        } else if let Err(error) = &event.result {
            eprintln!("[display] setting brightness failed: {error:?}");
            read_properties(app);
        }
    }
}

pub(super) fn on_properties_changed(app: &mut App, event: &SignalReceived) {
    if !event
        .matches
        .contains(&app.resource::<DisplayStatus>().properties)
    {
        return;
    }
    let (_, changed, invalidated) = match PropertiesChanged::parse(&event.msg) {
        Ok(parsed) => parsed,
        Err(error) => {
            eprintln!("[display] invalid PropertiesChanged: {error:?}");
            return;
        }
    };
    if invalidated
        .iter()
        .any(|name| matches!(name.as_str(), "Brightness" | "Minimum"))
    {
        read_properties(app);
        return;
    }
    let value = level(&changed, "Brightness");
    let minimum = level(&changed, "Minimum");
    if value.is_none() && minimum.is_none() {
        return;
    }
    {
        let mut status = app.resource_mut::<DisplayStatus>();
        if let Some(value) = value {
            status.value = value;
        }
        if let Some(minimum) = minimum {
            status.minimum = minimum;
        }
    }
    emit_brightness(app);
}

pub(super) fn on_owner_changed(app: &mut App, event: &SignalReceived) {
    if event.bus != BusId::Session
        || !event
            .matches
            .contains(&app.resource::<DisplayStatus>().owner_changed)
    {
        return;
    }
    let Ok((name, _, new_owner)) = event.msg.body::<(String, String, String)>() else {
        return;
    };
    if name != BRIGHTNESS {
        return;
    }
    let mut status = app.resource_mut::<DisplayStatus>();
    status.read = None;
    status.read_stale = false;
    status.write = None;
    status.pending = None;
    if !new_owner.is_empty() {
        read_properties(app);
    }
}

pub(super) fn on_disconnect(app: &mut App, event: &BusDisconnected) {
    if event.bus == BusId::Session {
        let mut status = app.resource_mut::<DisplayStatus>();
        status.read = None;
        status.read_stale = false;
        status.write = None;
        status.pending = None;
    }
}
