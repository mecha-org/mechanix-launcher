//! Shared state and API-table assembly for the custom libpulse mainloop.

use std::cell::UnsafeCell;
use std::collections::HashMap;

use libpulse_sys::mainloop::api::pa_mainloop_api;

mod defer_api;
mod io_api;
mod timer_api;

use defer_api::DeferEntry;
use io_api::IoWatch;
use timer_api::TimerEntry;

#[derive(Default)]
pub(super) struct MainloopState {
    // Box keeps addresses stable; UnsafeCell permits access through FFI handles
    // even while other entries in the registry are inserted or removed.
    watches: HashMap<u64, Box<UnsafeCell<IoWatch>>>,
    timers: HashMap<u64, Box<UnsafeCell<TimerEntry>>>,
    deferred: HashMap<u64, Box<UnsafeCell<DeferEntry>>>,
    next_watch_id: u64,
    next_timer_id: u64,
    next_defer_id: u64,
}

/// Assemble the implemented hooks; the caller keeps state alive at this address.
pub(super) fn make_api(state: &UnsafeCell<MainloopState>) -> pa_mainloop_api {
    pa_mainloop_api {
        userdata: state.get().cast(),
        io_new: Some(io_api::io_new),
        io_enable: Some(io_api::io_enable),
        io_free: Some(io_api::io_free),
        io_set_destroy: Some(io_api::io_set_destroy),
        time_new: Some(timer_api::time_new),
        time_restart: Some(timer_api::time_restart),
        time_free: Some(timer_api::time_free),
        time_set_destroy: Some(timer_api::time_set_destroy),
        defer_new: Some(defer_api::defer_new),
        defer_enable: Some(defer_api::defer_enable),
        defer_free: Some(defer_api::defer_free),
        defer_set_destroy: Some(defer_api::defer_set_destroy),
        quit: Some(quit),
    }
}

/// This adapter does not own the App's Ring runner, so it cannot stop it.
extern "C" fn quit(_: *const pa_mainloop_api, _: i32) {
    eprintln!("quit() ignored");
}
