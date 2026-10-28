//! Timer part of the libpulse asynchronous mainloop API.

use std::ffi::c_void;

use libc::timeval;
use libpulse_sys::mainloop::api::*;

/// Our timer registration, exposed to libpulse as an opaque `pa_time_event` pointer.
pub struct TimerEntry {
    id: u64,
    api: *const pa_mainloop_api,
    // Copy the absolute deadline rather than retaining libpulse's pointer.
    // None means the timer is disabled.
    deadline: Option<timeval>,
    callback: pa_time_event_cb_t,
    userdata: *mut c_void,
    destroy_callback: pa_time_event_destroy_cb_t,
    // Changes on restart or disabling, making older queued timeouts obsolete.
    generation: u64,
}
