//! Read-only volume and mute state for the server's current default sink.
//!
//! Libpulse callbacks copy information into this service's queue;
//! App systems then publish `VolumeState`. No callback holds an App pointer.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_void};
use std::ptr;
use std::rc::Rc;

use libpulse_sys::context::{
    self as pulse, PA_SUBSCRIPTION_EVENT_FACILITY_MASK, PA_SUBSCRIPTION_EVENT_SERVER,
    PA_SUBSCRIPTION_EVENT_SINK, PA_SUBSCRIPTION_MASK_SERVER, PA_SUBSCRIPTION_MASK_SINK, pa_context,
    pa_server_info, pa_sink_info,
};
use libpulse_sys::operation::{pa_operation, pa_operation_unref};
use libpulse_sys::volume::{PA_VOLUME_NORM, pa_cvolume_avg, pa_cvolume_valid};
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

impl Module for VolumeControlModule {
    fn install(self, app: &mut App) {
        app.init_resource::<VolumeState>();
        if !app.has_resource::<PulseAudio>() || app.has_resource::<VolumeBackend>() {
            return;
        }
        app.init_resource::<VolumeBackend>()
            .system(on_connection)
            .system(on_tick);
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

#[derive(Default, PartialEq, Eq)]
enum Request {
    #[default]
    Idle,
    Server,
    Sink,
}

#[derive(Default)]
struct VolumeBackend {
    updates: Rc<RefCell<Vec<VolumeUpdate>>>,
    started: bool,
    request: Request,
    dirty: bool,
    snapshot: Option<VolumeState>,
}
impl Resource for VolumeBackend {}

/// Only owned values cross the C callback boundary.
pub(crate) enum VolumeUpdate {
    Dirty,
    DefaultSink(Option<CString>),
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
                backend.request = Request::Idle;
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
    request_server(app);
}

fn request_server(app: &mut App) {
    if !matches!(
        app.resource::<PulseAudio>().connection_state(),
        ConnectionState::Ready
    ) || app.resource::<PulseAudio>().context().is_none()
    {
        return;
    }
    app.resource_mut::<VolumeBackend>().request = Request::Server;
    let Some((context, userdata)) = context_and_queue(app) else {
        return;
    };
    // SAFETY: the context and callback queue stay alive through libpulse's
    // asynchronous reply. Libpulse owns another operation reference.
    let operation =
        unsafe { pulse::pa_context_get_server_info(context.raw(), Some(on_server), userdata) };
    if !release(operation, "get default output") {
        app.resource_mut::<VolumeBackend>().request = Request::Idle;
        app.resource_mut::<VolumeState>()
            .set_if_neq(VolumeState::Unavailable);
    }
}

fn request_sink(app: &mut App, name: &CStr) {
    if app.resource::<PulseAudio>().context().is_none() {
        return;
    }
    app.resource_mut::<VolumeBackend>().request = Request::Sink;
    let Some((context, userdata)) = context_and_queue(app) else {
        return;
    };
    // SAFETY: libpulse encodes the name before returning. The context and
    // callback queue live through the asynchronous sink reply.
    let operation = unsafe {
        pulse::pa_context_get_sink_info_by_name(
            context.raw(),
            name.as_ptr(),
            Some(on_sink),
            userdata,
        )
    };
    if !release(operation, "get default output volume") {
        app.resource_mut::<VolumeBackend>().request = Request::Idle;
        app.resource_mut::<VolumeState>()
            .set_if_neq(VolumeState::Unavailable);
    }
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
            VolumeUpdate::DefaultSink(name)
                if app.resource::<VolumeBackend>().request == Request::Server =>
            {
                if let Some(name) = name {
                    request_sink(app, &name);
                } else {
                    app.resource_mut::<VolumeBackend>().request = Request::Idle;
                    app.resource_mut::<VolumeState>()
                        .set_if_neq(VolumeState::Unavailable);
                }
            }
            VolumeUpdate::Sink(state)
                if app.resource::<VolumeBackend>().request == Request::Sink =>
            {
                app.resource_mut::<VolumeBackend>().snapshot = Some(state);
            }
            VolumeUpdate::SinkDone if app.resource::<VolumeBackend>().request == Request::Sink => {
                let snapshot = {
                    let mut backend = app.resource_mut::<VolumeBackend>();
                    backend.request = Request::Idle;
                    backend.snapshot.take().unwrap_or(VolumeState::Unavailable)
                };
                // A change during an in-flight query makes its reply stale.
                if !app.resource::<VolumeBackend>().dirty {
                    app.resource_mut::<VolumeState>().set_if_neq(snapshot);
                }
            }
            _ => {}
        }
    }
    if app.resource::<VolumeBackend>().started
        && app.resource::<VolumeBackend>().request == Request::Idle
        && app.resource::<VolumeBackend>().dirty
    {
        app.resource_mut::<VolumeBackend>().dirty = false;
        request_server(app);
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

extern "C" fn on_server(_: *mut pa_context, info: *const pa_server_info, userdata: *mut c_void) {
    // SAFETY: libpulse's info and its name pointer are valid for this callback.
    let name = unsafe { info.as_ref() }.and_then(|info| {
        (!info.default_sink_name.is_null()).then(|| {
            // SAFETY: this is a non-null, NUL-terminated libpulse string.
            unsafe { CStr::from_ptr(info.default_sink_name) }.to_owned()
        })
    });
    // SAFETY: the queue outlives this callback and holds only copied values.
    let queue = unsafe { &*userdata.cast::<RefCell<Vec<VolumeUpdate>>>() };
    queue.borrow_mut().push(VolumeUpdate::DefaultSink(name));
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
mod tests {
    use super::*;

    #[derive(Default)]
    struct Seen(Vec<VolumeState>);
    impl Resource for Seen {}

    fn record_change(app: &mut App, _: &OnChanged<VolumeState>) {
        let state = *app.resource::<VolumeState>();
        app.resource_mut::<Seen>().0.push(state);
    }

    #[test]
    fn volumes_are_percentages_of_normal_level_not_clamped_at_100() {
        assert_eq!(volume_percent(0), 0);
        assert_eq!(volume_percent(PA_VOLUME_NORM / 2), 50);
        assert_eq!(volume_percent(PA_VOLUME_NORM), 100);
        assert_eq!(volume_percent(PA_VOLUME_NORM * 3 / 2), 150);
    }

    #[test]
    fn sink_snapshot_publishes_only_after_the_reply_finishes() {
        let mut app = App::new();
        app.init_resource::<PulseAudio>()
            .add_module(VolumeControlModule);
        {
            let mut backend = app.resource_mut::<VolumeBackend>();
            backend.started = true;
            backend.request = Request::Sink;
        }
        app.resource::<VolumeBackend>()
            .updates
            .borrow_mut()
            .push(VolumeUpdate::Sink(VolumeState::Ready {
                volume_percent: 37,
                muted: true,
            }));

        on_tick(&mut app, &Tick);
        assert_eq!(*app.resource::<VolumeState>(), VolumeState::Loading);

        app.resource::<VolumeBackend>()
            .updates
            .borrow_mut()
            .push(VolumeUpdate::SinkDone);
        on_tick(&mut app, &Tick);
        assert_eq!(
            *app.resource::<VolumeState>(),
            VolumeState::Ready {
                volume_percent: 37,
                muted: true,
            }
        );
    }

    #[test]
    fn a_new_snapshot_emits_on_changed_for_volume_state() {
        let mut app = App::new();
        app.init_resource::<PulseAudio>()
            .add_module(VolumeControlModule)
            .init_resource::<Seen>()
            .system(record_change);
        {
            let mut backend = app.resource_mut::<VolumeBackend>();
            backend.started = true;
            backend.request = Request::Sink;
        }
        let state = VolumeState::Ready {
            volume_percent: 62,
            muted: true,
        };
        app.resource::<VolumeBackend>()
            .updates
            .borrow_mut()
            .extend([VolumeUpdate::Sink(state), VolumeUpdate::SinkDone]);

        app.tick();
        assert_eq!(app.resource::<Seen>().0, [state]);
        app.tick();
        assert_eq!(app.resource::<Seen>().0, [state]);
    }

    #[test]
    fn only_sink_and_server_subscriptions_request_a_refresh() {
        let backend = VolumeBackend::default();
        let userdata: *mut c_void = Rc::as_ptr(&backend.updates).cast_mut().cast();
        on_subscription(ptr::null_mut(), PA_SUBSCRIPTION_EVENT_SINK, 1, userdata);
        on_subscription(ptr::null_mut(), PA_SUBSCRIPTION_EVENT_SERVER, 0, userdata);
        on_subscription(ptr::null_mut(), 1, 2, userdata); // A source change.
        assert_eq!(backend.updates.borrow().len(), 2);
    }

    #[test]
    fn connection_failure_resets_volume_without_stopping_the_app() {
        let mut app = App::new();
        app.init_resource::<PulseAudio>()
            .add_module(VolumeControlModule);
        app.resource_mut::<VolumeState>()
            .set_if_neq(VolumeState::Ready {
                volume_percent: 37,
                muted: true,
            });

        on_connection(
            &mut app,
            &PulseAudioContextConnectionStateChanged(ConnectionState::Failed(
                "audio went away".into(),
            )),
        );

        assert_eq!(*app.resource::<VolumeState>(), VolumeState::Unavailable);
    }

    #[test]
    fn installing_after_a_connection_failure_reports_unavailable() {
        let mut app = App::new();
        app.init_resource::<PulseAudio>();
        app.resource_mut::<PulseAudio>().connection_state =
            ConnectionState::Failed("audio went away".into());

        app.add_module(VolumeControlModule);

        assert_eq!(*app.resource::<VolumeState>(), VolumeState::Unavailable);
    }

    #[test]
    fn duplicate_snapshots_do_not_flag_the_resource_as_changed() {
        let mut app = App::new();
        app.init_resource::<PulseAudio>()
            .add_module(VolumeControlModule);
        let value = VolumeState::Ready {
            volume_percent: 64,
            muted: false,
        };
        app.resource_mut::<VolumeState>().set_if_neq(value);
        app.take_resource_changed::<VolumeState>();
        {
            let mut backend = app.resource_mut::<VolumeBackend>();
            backend.started = true;
            backend.request = Request::Sink;
        }
        app.resource::<VolumeBackend>()
            .updates
            .borrow_mut()
            .extend([VolumeUpdate::Sink(value), VolumeUpdate::SinkDone]);

        on_tick(&mut app, &Tick);

        assert!(!app.take_resource_changed::<VolumeState>());
        assert_eq!(*app.resource::<VolumeState>(), value);
    }

    #[test]
    fn null_sink_reply_sets_unavailable_instead_of_panicking() {
        let mut app = App::new();
        app.init_resource::<PulseAudio>()
            .add_module(VolumeControlModule);
        {
            let mut backend = app.resource_mut::<VolumeBackend>();
            backend.started = true;
            backend.request = Request::Sink;
        }
        let backend = app.resource::<VolumeBackend>();
        let userdata: *mut c_void = Rc::as_ptr(&backend.updates).cast_mut().cast();
        on_sink(ptr::null_mut(), ptr::null(), -1, userdata);

        on_tick(&mut app, &Tick);

        assert_eq!(*app.resource::<VolumeState>(), VolumeState::Unavailable);
    }

    #[test]
    fn shared_connection_keeps_callback_data_alive_after_volume_backend_drops() {
        let mut app = App::new();
        app.init_resource::<PulseAudio>()
            .add_module(VolumeControlModule);
        let weak = Rc::downgrade(&app.resource::<VolumeBackend>().updates);

        // The volume backend can be dropped first during App teardown.
        let backend = app.insert_resource(VolumeBackend::default());
        drop(backend);
        assert!(weak.upgrade().is_some());

        drop(app);
        assert!(weak.upgrade().is_none());
    }
}
