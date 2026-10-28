//! Timer part of the libpulse asynchronous mainloop API.

use std::cell::UnsafeCell;
use std::ffi::c_void;

use libc::timeval;
use libpulse_sys::mainloop::api::*;

use super::MainloopState;

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

impl MainloopState {
    fn insert_timer(
        &mut self,
        api: *const pa_mainloop_api,
        deadline: Option<timeval>,
        callback: pa_time_event_cb_t,
        userdata: *mut c_void,
    ) -> *mut pa_time_event {
        let id = self.next_timer_id;
        self.next_timer_id += 1;

        let timer = Box::new(UnsafeCell::new(TimerEntry {
            id,
            api,
            deadline,
            callback,
            userdata,
            destroy_callback: None,
            generation: 0,
        }));
        let event = timer.get().cast::<pa_time_event>();
        self.timers.insert(id, timer);
        event
    }
}

/// Register a timer, or a disabled timer if the deadline is NULL.
pub(super) extern "C" fn time_new(
    api: *const pa_mainloop_api,
    deadline: *const timeval,
    callback: pa_time_event_cb_t,
    userdata: *mut c_void,
) -> *mut pa_time_event {
    // SAFETY: libpulse supplies NULL or a valid timestamp for this call.
    // Copy it now; the original may be on libpulse's stack.
    let deadline = unsafe { deadline.as_ref() }.copied();
    // SAFETY: the adapter supplies a live API and state, without overlapping
    // state borrows or concurrent callbacks.
    let state = unsafe { &mut *(*api).userdata.cast::<MainloopState>() };
    state.insert_timer(api, deadline, callback, userdata)
}

/// Restart the timer at a copied deadline, or disable it with NULL.
pub(super) extern "C" fn time_restart(event: *mut pa_time_event, deadline: *const timeval) {
    // SAFETY: the supplied timestamp is NULL or valid for this call. Copy
    // before borrowing the timer in case the timestamp refers to timer data.
    let deadline = unsafe { deadline.as_ref() }.copied();
    // SAFETY: libpulse passes a live handle returned by time_new. No other
    // reference to this timer is held while the FFI hook runs.
    let timer = unsafe { &mut *event.cast::<TimerEntry>() };
    timer.deadline = deadline;
    // Even restarting at the same deadline replaces the previous schedule.
    timer.generation += 1;
}

/// Remove the registration, keeping it alive through its destroy callback.
pub(super) extern "C" fn time_free(event: *mut pa_time_event) {
    // SAFETY: the handle belongs to this adapter and is still live. Do not
    // retain a reference while mutating the owning registry.
    let (api, id) = unsafe {
        let timer = &*event.cast::<TimerEntry>();
        (timer.api, timer.id)
    };

    let removed = {
        // SAFETY: the API's state is live and no other state borrow is active.
        let state = unsafe { &mut *(*api).userdata.cast::<MainloopState>() };
        state.timers.remove(&id)
    };

    if let Some(timer) = removed {
        // SAFETY: the local box owns the timer until the callback returns.
        // Copy callback arguments and release all registry/timer references.
        let (destroy, userdata) = unsafe {
            let timer = &*timer.get();
            (timer.destroy_callback, timer.userdata)
        };
        if let Some(destroy) = destroy {
            destroy(api, timer.get().cast(), userdata);
        }
        // Pending Ring timeouts will identify timers by ID and generation,
        // never by a retained pointer to this freed registration.
    }
}

pub(super) extern "C" fn time_set_destroy(
    event: *mut pa_time_event,
    callback: pa_time_event_destroy_cb_t,
) {
    // SAFETY: this is a live, exclusively accessed timer from time_new.
    let timer = unsafe { &mut *event.cast::<TimerEntry>() };
    timer.destroy_callback = callback;
}

#[cfg(test)]
mod tests;
