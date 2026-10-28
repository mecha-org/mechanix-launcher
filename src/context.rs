//! One libpulse context, with state callbacks buffered for App systems.

use std::cell::RefCell;
use std::ffi::{CStr, CString, c_void};
use std::ptr::{self, NonNull};

use libpulse_sys::context::{self as pulse, pa_context};
use libpulse_sys::error::pa_strerror;
use libpulse_sys::mainloop::api::pa_mainloop_api;

use crate::ConnectionState;

pub(crate) struct Context {
    ptr: NonNull<pa_context>,
    // Libpulse's userdata points to this allocation, not to App or Context.
    updates: Box<RefCell<Vec<ConnectionState>>>,
}

impl Context {
    pub(crate) fn connect(api: &pa_mainloop_api) -> Result<Self, String> {
        let runtime = std::env::var("XDG_RUNTIME_DIR")
            .map_err(|_| "XDG_RUNTIME_DIR is not set".to_owned())?;
        let address = CString::new(format!("unix:{runtime}/pulse/native"))
            .map_err(|_| "PulseAudio socket path contains a NUL byte".to_owned())?;
        let updates = Box::new(RefCell::new(Vec::new()));

        // SAFETY: `api` is a stable allocation owned by PulseAudio. The name
        // is NUL-terminated, and libpulse owns the returned context.
        let ptr = NonNull::new(unsafe { pulse::pa_context_new(api, c"mecha-pulseaudio".as_ptr()) })
            .ok_or_else(|| "pa_context_new returned null".to_owned())?;
        // SAFETY: the context and the boxed callback data live until Drop.
        unsafe {
            pulse::pa_context_set_state_callback(
                ptr.as_ptr(),
                Some(on_state),
                ptr::from_ref(updates.as_ref()).cast_mut().cast(),
            );
        }

        let context = Self { ptr, updates };
        // SAFETY: the local Unix socket address is NUL-terminated; disabling
        // autospawn keeps the connection attempt within the existing server.
        let result = unsafe {
            pulse::pa_context_connect(
                context.ptr.as_ptr(),
                address.as_ptr(),
                pulse::PA_CONTEXT_NOAUTOSPAWN,
                ptr::null(),
            )
        };
        if result < 0 {
            let error = context.last_error();
            context
                .updates
                .borrow_mut()
                .push(ConnectionState::Failed(error));
        }
        Ok(context)
    }

    pub(crate) fn take_updates(&self) -> Vec<ConnectionState> {
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
        // SAFETY: the context is still live, as are the API and callback data.
        // Clear the callback before disconnect so it cannot enqueue into data
        // that is about to be dropped. Disconnect before releasing the context.
        unsafe {
            pulse::pa_context_set_state_callback(self.ptr.as_ptr(), None, ptr::null_mut());
            pulse::pa_context_disconnect(self.ptr.as_ptr());
            pulse::pa_context_unref(self.ptr.as_ptr());
        }
    }
}
