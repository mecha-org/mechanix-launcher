//! Shared state and API-table assembly for the custom libpulse mainloop.

use std::cell::UnsafeCell;
use std::collections::HashMap;

use libpulse_sys::mainloop::api::pa_mainloop_api;

mod io_api;

use io_api::IoWatch;

#[derive(Default)]
pub(super) struct MainloopState {
    // Box keeps addresses stable; UnsafeCell permits access through FFI handles
    // even while other entries in the registry are inserted or removed.
    watches: HashMap<u64, Box<UnsafeCell<IoWatch>>>,
    next_watch_id: u64,
}

/// Assemble the implemented hooks; the caller keeps state alive at this address.
/// Timer, deferred-work and quit hooks must be added before using a context.
pub(super) fn make_api(state: &UnsafeCell<MainloopState>) -> pa_mainloop_api {
    pa_mainloop_api {
        userdata: state.get().cast(),
        io_new: Some(io_api::io_new),
        io_enable: Some(io_api::io_enable),
        io_free: Some(io_api::io_free),
        io_set_destroy: Some(io_api::io_set_destroy),
        time_new: None,
        time_restart: None,
        time_free: None,
        time_set_destroy: None,
        defer_new: None,
        defer_enable: None,
        defer_free: None,
        defer_set_destroy: None,
        quit: None,
    }
}
