//! Display brightness client and reply handling.
// TODO: Subscribe to brightness changes made elsewhere when the Display service exposes
// a dedicated signal or a Brightness property with PropertiesChanged notifications.
use super::interface::{DISPLAY, INTERFACE, PATH};
use mecha_wayland::prelude::*;

pub(super) struct DisplayStatus {
    pub(super) owner_changed: MatchId,
    pub(super) read: Option<CallId>,
    pub(super) write: Option<CallId>,
    /// Latest value requested while `write` was in flight. Only the newest
    /// is kept, so a fast drag never queues a backlog of stale writes.
    pub(super) pending: Option<u8>,
}
impl Resource for DisplayStatus {}

/// Lowest brightness the launcher will set, so the screen never goes fully dark.
pub const MIN_BRIGHTNESS: f32 = 4.0;
pub const MAX_BRIGHTNESS: f32 = 254.0;

/// Display brightness as a slider value in `MIN_BRIGHTNESS..=MAX_BRIGHTNESS`.
pub struct BrightnessChanged {
    pub value: f32,
}
impl Event for BrightnessChanged {}

/// Request a slider brightness value in `MIN_BRIGHTNESS..=MAX_BRIGHTNESS`.
pub struct SetBrightness(pub f32);
impl Signal for SetBrightness {}

pub(super) fn read_brightness(app: &mut App) {
    let call = app
        .resource_mut::<DBus>()
        .system()
        .object(DISPLAY, PATH)
        .call(INTERFACE, "GetBrightness", &());
    app.resource_mut::<DisplayStatus>().read = Some(call);
}

pub(super) fn on_set_brightness(app: &mut App, signal: &SetBrightness) {
    if !signal.0.is_finite() {
        return;
    }
    let value = signal.0.clamp(MIN_BRIGHTNESS, MAX_BRIGHTNESS).round() as u8;
    let mut status = app.resource_mut::<DisplayStatus>();
    // A read started before the user's change must not reset the slider.
    status.read = None;
    if status.write.is_some() {
        status.pending = Some(value);
        return;
    }
    drop(status);
    write_brightness(app, value);
}

fn write_brightness(app: &mut App, value: u8) {
    let call = app
        .resource_mut::<DBus>()
        .system()
        .object(DISPLAY, PATH)
        .call(INTERFACE, "SetBrightness", &(value,));
    app.resource_mut::<DisplayStatus>().write = Some(call);
}

pub(super) fn on_reply(app: &mut App, event: &MethodReply) {
    let status = app.resource::<DisplayStatus>();
    if status.read == Some(event.call) {
        app.resource_mut::<DisplayStatus>().read = None;
        let brightness = event
            .result
            .as_ref()
            .map_err(|error| format!("{error:?}"))
            .and_then(|msg| msg.body::<(u8,)>().map_err(|error| format!("{error:?}")));
        match brightness {
            Ok((value,)) => {
                if (value as f32) <= MAX_BRIGHTNESS {
                    app.emit(
                        BrightnessChanged {
                            value: value as f32,
                        },
                        app.descendants(app.root()).collect::<Vec<_>>(),
                    );
                } else {
                    eprintln!("[display] invalid brightness value: {value}");
                }
            }
            Err(error) => eprintln!("[display] GetBrightness failed: {error}"),
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
            eprintln!("[display] SetBrightness failed: {error:?}");
            read_brightness(app);
        }
    }
}

pub(super) fn on_owner_changed(app: &mut App, event: &SignalReceived) {
    if event.bus != BusId::System
        || !event
            .matches
            .contains(&app.resource::<DisplayStatus>().owner_changed)
    {
        return;
    }
    let Ok((name, _, new_owner)) = event.msg.body::<(String, String, String)>() else {
        return;
    };
    if name != DISPLAY {
        return;
    }
    let mut status = app.resource_mut::<DisplayStatus>();
    status.read = None;
    status.write = None;
    status.pending = None;
    if !new_owner.is_empty() {
        read_brightness(app);
    }
}

pub(super) fn on_disconnect(app: &mut App, event: &BusDisconnected) {
    if event.bus == BusId::System {
        let mut status = app.resource_mut::<DisplayStatus>();
        status.read = None;
        status.write = None;
        status.pending = None;
    }
}
