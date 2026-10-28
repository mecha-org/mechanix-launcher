use std::ffi::c_void;
use std::os::fd::RawFd;

use libpulse_sys::mainloop::api::*;

// describe a pulseaudio watch
struct IoWatch {
    fd: RawFd,
    events: pa_io_event_flags_t,
    callback: pa_io_event_cb_t,
    userdata: *mut c_void,
    destroy_callback: pa_io_event_destroy_cb_t,
    generation: u64, // use to ignore pending polls from watches already destroyed
}
