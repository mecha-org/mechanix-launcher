//! Volume and mute state for the server's current default sink.
//!
//! Libpulse callbacks copy information into this service's queue;
//! App systems then publish `VolumeState`. No callback holds an App pointer.

use std::cell::RefCell;
use std::ffi::c_void;
use std::ptr;
use std::rc::Rc;

use libpulse_sys::context::{
    self as pulse, PA_SUBSCRIPTION_EVENT_FACILITY_MASK, PA_SUBSCRIPTION_EVENT_SERVER,
    PA_SUBSCRIPTION_EVENT_SINK, PA_SUBSCRIPTION_MASK_SERVER, PA_SUBSCRIPTION_MASK_SINK, pa_context,
    pa_sink_info,
};
use libpulse_sys::operation::{pa_operation, pa_operation_unref};
use libpulse_sys::volume::{
    PA_VOLUME_NORM, pa_cvolume, pa_cvolume_avg, pa_cvolume_set, pa_cvolume_valid,
};
use mecha_wayland::prelude::*;

use crate::{
    ConnectionState, PulseAudio, PulseAudioContextConnectionStateChanged, context::Context,
};

/// Observed state of the default output. `Unavailable` means the server or
/// default output cannot currently be queried; it is not a volume of zero.
/// Volume is the average channel level, as a percentage of PulseAudio's
/// normal (0 dB) volume. Values above 100% are possible.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum VolumeState {
    #[default]
    Loading,
    Unavailable,
    Ready {
        volume_percent: u32,
        muted: bool,
    },
}

impl Resource for VolumeState {}

/// Watch the default output after [`PulseAudioModule`](crate::PulseAudioModule).
/// On changes, read [`VolumeState`] using `OnChanged<VolumeState>`.
pub struct VolumeControlModule;

/// Request a specific mute state for the default output.
/// A UI button can send `SetMuted(true)` or `SetMuted(false)`; the UI decides
/// which one. Observed [`VolumeState`] is updated from a fresh server query.
pub struct SetMuted(pub bool);
impl Signal for SetMuted {}

/// Set the default output to a 0–100% level using a mono volume request.
/// The server scales its existing channels, preserving their balance. Values
/// above 100 are clamped. Since [`VolumeState`] displays the channel average,
/// an unbalanced sink may report a different percentage after the refresh.
pub struct SetVolume(pub u32);
impl Signal for SetVolume {}

impl Module for VolumeControlModule {
    fn install(self, app: &mut App) {
        app.init_resource::<VolumeState>();
        if !app.has_resource::<PulseAudio>() || app.has_resource::<VolumeBackend>() {
            return;
        }
        app.init_resource::<VolumeBackend>()
            .system(on_connection)
            .system(on_tick)
            .system(on_set_muted)
            .system(on_set_volume);
        // The service owns the typed queue; PulseAudio retains a generic
        // reference until its context has disconnected all callbacks.
        let updates = Rc::clone(&app.resource::<VolumeBackend>().updates);
        app.resource_mut::<PulseAudio>().keep_callback_data(updates);

        // Modules may be installed after the connection already resolved.
        match app.resource::<PulseAudio>().connection_state().clone() {
            ConnectionState::Ready => start(app),
            ConnectionState::Failed(_) | ConnectionState::Terminated => {
                app.resource_mut::<VolumeState>()
                    .set_if_neq(VolumeState::Unavailable);
            }
            ConnectionState::Connecting => {}
        }
    }
}

#[derive(Default)]
struct VolumeBackend {
    updates: Rc<RefCell<Vec<VolumeUpdate>>>,
    started: bool,
    loading: bool,
    dirty: bool,
    snapshot: Option<VolumeState>,
}
impl Resource for VolumeBackend {}

/// Only owned values cross the C callback boundary.
enum VolumeUpdate {
    Dirty,
    Sink(VolumeState),
    SinkDone,
}

fn on_connection(app: &mut App, event: &PulseAudioContextConnectionStateChanged) {
    if !app.has_resource::<VolumeBackend>() {
        return;
    }
    match event.0 {
        ConnectionState::Ready => start(app),
        ConnectionState::Failed(_) | ConnectionState::Terminated => {
            {
                let mut backend = app.resource_mut::<VolumeBackend>();
                // Keep the same Rc: libpulse may still hold its raw pointer
                // until the context disconnects.
                backend.updates.borrow_mut().clear();
                backend.started = false;
                backend.loading = false;
                backend.dirty = false;
                backend.snapshot = None;
            }
            app.resource_mut::<VolumeState>()
                .set_if_neq(VolumeState::Unavailable);
        }
        ConnectionState::Connecting => {}
    }
}

fn context_and_queue(app: &App) -> Option<(&Context, *mut c_void)> {
    let audio = app.resource::<PulseAudio>();
    let context = audio.context()?;
    let backend = app.resource::<VolumeBackend>();
    let userdata = Rc::as_ptr(&backend.updates).cast_mut().cast();
    Some((context, userdata))
}

fn start(app: &mut App) {
    if app.resource::<VolumeBackend>().started || app.resource::<PulseAudio>().context().is_none() {
        return;
    }
    app.resource_mut::<VolumeBackend>().started = true;
    let Some((context, userdata)) = context_and_queue(app) else {
        return;
    };

    // SAFETY: PulseAudio retains a generic Rc to this service's queue until
    // it disconnects the context and clears the subscription callback.
    unsafe {
        pulse::pa_context_set_subscribe_callback(context.raw(), Some(on_subscription), userdata);
        let operation = pulse::pa_context_subscribe(
            context.raw(),
            PA_SUBSCRIPTION_MASK_SINK | PA_SUBSCRIPTION_MASK_SERVER,
            None,
            ptr::null_mut(),
        );
        release(operation, "subscribe to output changes");
    }
    request_sink(app);
}

fn request_sink(app: &mut App) {
    if !matches!(
        app.resource::<PulseAudio>().connection_state(),
        ConnectionState::Ready
    ) || app.resource::<PulseAudio>().context().is_none()
    {
        return;
    }
    app.resource_mut::<VolumeBackend>().loading = true;
    let Some((context, userdata)) = context_and_queue(app) else {
        return;
    };
    // The server resolves this special name to its current default sink. No
    // server-info round trip or cached device name is needed.
    // SAFETY: the context and callback queue live through the async reply.
    let operation = unsafe {
        pulse::pa_context_get_sink_info_by_name(
            context.raw(),
            c"@DEFAULT_SINK@".as_ptr(),
            Some(on_sink),
            userdata,
        )
    };
    if !release(operation, "get default output volume") {
        app.resource_mut::<VolumeBackend>().loading = false;
        app.resource_mut::<VolumeState>()
            .set_if_neq(VolumeState::Unavailable);
    }
}

fn ready_to_set(app: &App) -> bool {
    let backend = app.resource::<VolumeBackend>();
    !backend.loading
        && !backend.dirty
        && matches!(app.resource::<VolumeState>(), VolumeState::Ready { .. })
        && matches!(
            app.resource::<PulseAudio>().connection_state(),
            ConnectionState::Ready
        )
}

fn on_set_muted(app: &mut App, request: &SetMuted) {
    if !ready_to_set(app) {
        return;
    }
    let Some((context, userdata)) = context_and_queue(app) else {
        return;
    };
    // SAFETY: context is borrowed for this call; both it and the callback
    // queue remain alive through the asynchronous acknowledgement. The server
    // resolves @DEFAULT_SINK@ when handling the request.
    let operation = unsafe {
        pulse::pa_context_set_sink_mute_by_name(
            context.raw(),
            c"@DEFAULT_SINK@".as_ptr(),
            i32::from(request.0),
            Some(on_mute_done),
            userdata,
        )
    };
    release(operation, "set default output mute");
}

fn on_set_volume(app: &mut App, request: &SetVolume) {
    if !ready_to_set(app) {
        return;
    }
    let volume = cvolume_for_percent(request.0);
    let Some((context, userdata)) = context_and_queue(app) else {
        return;
    };

    // SAFETY: libpulse copies the stack-local channel volumes before returning.
    // The context and callback queue remain live for the async acknowledgement.
    let operation = unsafe {
        pulse::pa_context_set_sink_volume_by_name(
            context.raw(),
            c"@DEFAULT_SINK@".as_ptr(),
            &volume,
            Some(on_volume_done),
            userdata,
        )
    };
    release(operation, "set default output volume");
}

fn cvolume_for_percent(percent: u32) -> pa_cvolume {
    // Use u64 before multiplying to avoid overflow from arbitrary UI input.
    let raw = (u64::from(percent.min(100)) * u64::from(PA_VOLUME_NORM) / 100) as u32;
    let mut volume = pa_cvolume::default();
    // PulseAudio accepts a one-channel volume for any sink: it scales the
    // sink's existing channels relative to their current maximum volume.
    // SAFETY: one channel is always valid; volume is a live C struct.
    unsafe { pa_cvolume_set(&mut volume, 1, raw) };
    volume
}

extern "C" fn on_mute_done(_: *mut pa_context, success: i32, userdata: *mut c_void) {
    if success == 0 {
        eprintln!("PulseAudio: could not set default output mute");
    }
    // SAFETY: the retained Rc keeps the volume queue alive until libpulse
    // can no longer invoke this callback. No App borrow crosses the FFI call.
    let queue = unsafe { &*userdata.cast::<RefCell<Vec<VolumeUpdate>>>() };
    // The UI may issue another request without waiting for this one; don't
    // guess the result. Refresh the server-observed state after each reply.
    queue.borrow_mut().push(VolumeUpdate::Dirty);
}

extern "C" fn on_volume_done(_: *mut pa_context, success: i32, userdata: *mut c_void) {
    if success == 0 {
        eprintln!("PulseAudio: could not set default output volume");
    }
    // SAFETY: the retained Rc keeps the callback queue alive; no App borrow
    // crosses this libpulse callback.
    let queue = unsafe { &*userdata.cast::<RefCell<Vec<VolumeUpdate>>>() };
    // Read back the actual server value even if the write failed.
    queue.borrow_mut().push(VolumeUpdate::Dirty);
}

/// Release the application's operation reference. Libpulse retains its own
/// reference until it calls the completion callback.
fn release(operation: *mut pa_operation, description: &str) -> bool {
    if operation.is_null() {
        eprintln!("PulseAudio: could not {description}");
        false
    } else {
        // SAFETY: libpulse returned an owned reference for a non-null operation.
        unsafe { pa_operation_unref(operation) };
        true
    }
}

fn on_tick(app: &mut App, _: &Tick) {
    let updates = {
        let backend = app.resource::<VolumeBackend>();
        std::mem::take(&mut *backend.updates.borrow_mut())
    };
    for update in updates {
        if !app.resource::<VolumeBackend>().started {
            continue;
        }
        match update {
            VolumeUpdate::Dirty => app.resource_mut::<VolumeBackend>().dirty = true,
            VolumeUpdate::Sink(state) if app.resource::<VolumeBackend>().loading => {
                app.resource_mut::<VolumeBackend>().snapshot = Some(state);
            }
            VolumeUpdate::SinkDone if app.resource::<VolumeBackend>().loading => {
                let (snapshot, dirty) = {
                    let mut backend = app.resource_mut::<VolumeBackend>();
                    backend.loading = false;
                    (
                        backend.snapshot.take().unwrap_or(VolumeState::Unavailable),
                        backend.dirty,
                    )
                };
                // A change during an in-flight query makes its reply stale.
                if !dirty {
                    app.resource_mut::<VolumeState>().set_if_neq(snapshot);
                }
            }
            _ => {}
        }
    }
    if app.resource::<VolumeBackend>().started
        && !app.resource::<VolumeBackend>().loading
        && app.resource::<VolumeBackend>().dirty
    {
        app.resource_mut::<VolumeBackend>().dirty = false;
        request_sink(app);
    }
}

extern "C" fn on_subscription(_: *mut pa_context, event: u32, _: u32, userdata: *mut c_void) {
    let facility = event & PA_SUBSCRIPTION_EVENT_FACILITY_MASK;
    if facility == PA_SUBSCRIPTION_EVENT_SINK || facility == PA_SUBSCRIPTION_EVENT_SERVER {
        // SAFETY: the queue is kept alive by PulseAudio's generic Rc until
        // disconnect, even if the volume resource is dropped first.
        let queue = unsafe { &*userdata.cast::<RefCell<Vec<VolumeUpdate>>>() };
        queue.borrow_mut().push(VolumeUpdate::Dirty);
    }
}

extern "C" fn on_sink(
    _: *mut pa_context,
    info: *const pa_sink_info,
    eol: i32,
    userdata: *mut c_void,
) {
    // SAFETY: the Rc kept by PulseAudio keeps this queue alive through the
    // libpulse operation. Copy the fields before the callback returns.
    let queue = unsafe { &*userdata.cast::<RefCell<Vec<VolumeUpdate>>>() };
    if eol != 0 {
        queue.borrow_mut().push(VolumeUpdate::SinkDone);
    } else if let Some(info) = unsafe { info.as_ref() } {
        // SAFETY: libpulse supplied a valid sink_info for this callback.
        if unsafe { pa_cvolume_valid(&info.volume) } != 0 {
            let average = unsafe { pa_cvolume_avg(&info.volume) };
            let percent = volume_percent(average);
            queue
                .borrow_mut()
                .push(VolumeUpdate::Sink(VolumeState::Ready {
                    volume_percent: percent,
                    muted: info.mute != 0,
                }));
        }
    }
}

fn volume_percent(average: u32) -> u32 {
    ((u64::from(average) * 100 + u64::from(PA_VOLUME_NORM) / 2) / u64::from(PA_VOLUME_NORM)) as u32
}

#[cfg(test)]
mod tests;
