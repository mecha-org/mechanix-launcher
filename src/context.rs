//! The libpulse connection and its bridge to mecha-wayland signals.
//!
//! `pa_context` does the actual PulseAudio protocol work. We give it the
//! Ring-backed `pa_mainloop_api`, which tells us when to call libpulse again.
//! Its C state callback cannot access `App`: it copies important transitions
//! into a stable queue. `on_tick` below later drains that queue and
//! publishes `ConnectionStateChanged` on the ordinary App thread.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_void};
use std::ptr::{self, NonNull};

use libpulse_sys::context::{self as pulse, pa_context};
use libpulse_sys::error::pa_strerror;
use libpulse_sys::mainloop::api::pa_mainloop_api;
use mecha_wayland::prelude::{App, Signal, Tick};

use crate::PulseAudio;

/// Connection lifecycle reported by libpulse, delivered on the App thread.
/// `Connecting` is the initial state; intermediate handshake states remain
/// internal to libpulse and are not emitted separately.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionState {
    Connecting,
    Ready,
    Failed(String),
    Terminated,
}

/// A change in the shared connection's state, queued on `Tick` after the
/// libpulse callback has returned. This is a mecha-wayland `Signal`.
pub struct ConnectionStateChanged(pub ConnectionState);
impl Signal for ConnectionStateChanged {}

impl PulseAudio {
    /// Start connecting once. Installation returns before the handshake ends.
    pub(crate) fn start_connection(&mut self) {
        if self.context.is_none() {
            match Context::connect(&self.mainloop_api) {
                Ok(context) => self.context = Some(context),
                Err(error) => self.connection_updates.push(ConnectionState::Failed(error)),
            }
        }
    }

    /// Most recently reported connection state.
    pub fn connection_state(&self) -> &ConnectionState {
        &self.connection_state
    }

    /// A borrow for issuing an operation; async callbacks need separate
    /// owned userdata because this borrow ends before the reply arrives.
    pub(crate) fn context(&self) -> Option<&Context> {
        self.context.as_ref()
    }

    fn take_connection_updates(&mut self) -> Vec<ConnectionState> {
        // Merge early setup failures with changes reported by the live C
        // context. Neither queue calls back into App while libpulse runs.
        let mut updates = std::mem::take(&mut self.connection_updates);
        if let Some(context) = &self.context {
            updates.extend(context.take_updates());
        }
        updates
    }

    pub(crate) fn has_connection_updates(&self) -> bool {
        !self.connection_updates.is_empty()
            || self.context.as_ref().is_some_and(Context::has_updates)
    }
}

/// Transfer C state notifications into ordinary app-wide signals on Tick.
pub(crate) fn on_tick(app: &mut App, _: &Tick) {
    // Never pass an App pointer to libpulse. It only writes to its queue;
    // here, after its callback returned, we drain and publish the updates.
    let updates = app.resource_mut::<PulseAudio>().take_connection_updates();
    for state in updates {
        // Synchronous setup failure and the callback might report the same
        // state. Publish only actual changes.
        if app.resource::<PulseAudio>().connection_state == state {
            continue;
        }
        app.resource_mut::<PulseAudio>().connection_state = state.clone();
        app.signal(ConnectionStateChanged(state));
    }
}

impl Drop for PulseAudio {
    fn drop(&mut self) {
        // Disconnect before the API, mainloop state, or retained callback
        // data drop. C callbacks can still access these during disconnect.
        self.context.take();
    }
}

/// Owns one libpulse reference; Rust must release it with `pa_context_unref`.
pub(crate) struct Context {
    ptr: NonNull<pa_context>,
    // Libpulse's callback userdata points here. Boxing keeps the address
    // stable even if Context or the enclosing PulseAudio resource moves.
    // It never receives an App pointer.
    updates: Box<RefCell<Vec<ConnectionState>>>,
}

impl Context {
    /// Borrow the native context for another service's asynchronous request.
    pub(crate) fn raw(&self) -> *mut pa_context {
        self.ptr.as_ptr()
    }

    /// Start an asynchronous connection; success here does NOT mean Ready.
    /// The later state callback reports readiness or connection failure.
    pub(crate) fn connect(api: &pa_mainloop_api) -> Result<Self, String> {
        // Deliberately pick the local server. `NULL` would ask libpulse to
        // select a server from its configuration/environment instead.
        let runtime = std::env::var("XDG_RUNTIME_DIR")
            .map_err(|_| "XDG_RUNTIME_DIR is not set".to_owned())?;
        let address = CString::new(format!("unix:{runtime}/pulse/native"))
            .map_err(|_| "PulseAudio socket path contains a NUL byte".to_owned())?;
        let updates = Box::new(RefCell::new(Vec::new()));

        // SAFETY: `api` is a stable allocation owned by PulseAudio. The name
        // is NUL-terminated; we take ownership of the returned C reference.
        let ptr = NonNull::new(unsafe { pulse::pa_context_new(api, c"mecha-pulseaudio".as_ptr()) })
            .ok_or_else(|| "pa_context_new returned null".to_owned())?;
        // Register before connecting: even an immediate error could trigger
        // a state change. Libpulse will call on_state, not an App system.
        // SAFETY: the context and boxed callback data live until Drop.
        unsafe {
            pulse::pa_context_set_state_callback(
                ptr.as_ptr(),
                Some(on_state),
                ptr::from_ref(updates.as_ref()).cast_mut().cast(),
            );
        }

        let context = Self { ptr, updates };
        // This starts connecting; libpulse does not wait for authentication or
        // readiness here. Ring's future I/O and timer completions drive it.
        // SAFETY: the address is NUL-terminated; NOAUTOSPAWN prevents libpulse
        // from starting a new server if none is listening.
        let result = unsafe {
            pulse::pa_context_connect(
                context.ptr.as_ptr(),
                address.as_ptr(),
                pulse::PA_CONTEXT_NOAUTOSPAWN,
                ptr::null(),
            )
        };
        if result < 0 {
            // A synchronous setup error might not generate a callback. Put
            // it through the same queue; on_tick suppresses duplicate states.
            let error = context.last_error();
            context
                .updates
                .borrow_mut()
                .push(ConnectionState::Failed(error));
        }
        Ok(context)
    }

    pub(crate) fn take_updates(&self) -> Vec<ConnectionState> {
        // Draining here, after the C callback returned, avoids borrowing
        // the queue while libpulse is writing to it.
        std::mem::take(&mut *self.updates.borrow_mut())
    }

    pub(crate) fn has_updates(&self) -> bool {
        !self.updates.borrow().is_empty()
    }

    fn last_error(&self) -> String {
        // SAFETY: libpulse owns the live context and returns a static string.
        unsafe { error_string(self.ptr.as_ptr()) }
    }
}

extern "C" fn on_state(context: *mut pa_context, userdata: *mut c_void) {
    // SAFETY: libpulse invokes this while the Context and its boxed RefCell
    // still live. No App reference or other borrow of the update queue exists.
    let updates = unsafe { &*userdata.cast::<RefCell<Vec<ConnectionState>>>() };
    // SAFETY: libpulse passes the live context currently changing state.
    let state = unsafe { pulse::pa_context_get_state(context) };
    // Ignore internal handshake steps; consumers only need the transition
    // to usable, failed, or terminated.
    let update = match state {
        pulse::pa_context_state_t::Ready => Some(ConnectionState::Ready),
        pulse::pa_context_state_t::Failed => {
            // SAFETY: the context remains live throughout its callback.
            Some(ConnectionState::Failed(unsafe { error_string(context) }))
        }
        pulse::pa_context_state_t::Terminated => Some(ConnectionState::Terminated),
        _ => None,
    };
    if let Some(update) = update {
        updates.borrow_mut().push(update);
    }
}

unsafe fn error_string(context: *mut pa_context) -> String {
    // SAFETY: the context is live, and pa_strerror returns static storage.
    let message = unsafe { pa_strerror(pulse::pa_context_errno(context)) };
    if message.is_null() {
        "unknown PulseAudio error".to_owned()
    } else {
        // SAFETY: pa_strerror returns a NUL-terminated static string.
        unsafe { CStr::from_ptr(message) }
            .to_string_lossy()
            .into_owned()
    }
}

impl Drop for Context {
    fn drop(&mut self) {
        // PulseAudio::drop explicitly calls this BEFORE dropping the API and
        // its userdata state: disconnect can still call our mainloop hooks.
        // SAFETY: the context is still live, as are the API and callback data.
        // Clear the callback before disconnect so it cannot enqueue into data
        // that is about to be dropped. Disconnect before releasing the context.
        unsafe {
            pulse::pa_context_set_state_callback(self.ptr.as_ptr(), None, ptr::null_mut());
            pulse::pa_context_set_subscribe_callback(self.ptr.as_ptr(), None, ptr::null_mut());
            pulse::pa_context_disconnect(self.ptr.as_ptr());
            pulse::pa_context_unref(self.ptr.as_ptr());
        }
    }
}
