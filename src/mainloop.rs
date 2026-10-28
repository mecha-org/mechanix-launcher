//! Libpulse I/O watch registration and callback snapshots for the Ring adapter.

use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::ffi::c_void;
use std::os::fd::RawFd;

use libpulse_sys::mainloop::api::*;

/// Our registration, exposed to libpulse as an opaque `pa_io_event` pointer.
struct IoWatch {
    id: u64,
    api: *const pa_mainloop_api,
    fd: RawFd,
    events: pa_io_event_flags_t,
    callback: pa_io_event_cb_t,
    userdata: *mut c_void,
    destroy_callback: pa_io_event_destroy_cb_t,
    // Changes when the interest mask changes, making older polls obsolete.
    generation: u64,
}

/// Copied callback arguments, allowing the caller to release registry borrows.
///
/// This struct is just a named bundle, not a libpulse requirement or a heap
/// allocation. A tuple or local copies would also work. What matters is ending
/// registry/resource borrows before invoking libpulse, which may free the watch.
/// The caller must still ensure the copied pointers remain valid until invocation.
pub(super) struct IoCallback {
    api: *const pa_mainloop_api,
    event: *mut pa_io_event,
    fd: RawFd,
    callback: pa_io_event_cb_t,
    userdata: *mut c_void,
}

impl IoCallback {
    /// Notify libpulse after releasing all state and resource borrows.
    ///
    /// # Safety
    /// The API, watch, descriptor and callback data must still be live. No FFI
    /// callback may have changed or removed the watch since this snapshot.
    pub(super) unsafe fn invoke(self, events: pa_io_event_flags_t) {
        if let Some(callback) = self.callback {
            callback(self.api, self.event, self.fd, events, self.userdata);
        }
    }
}

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
        io_new: Some(io_new),
        io_enable: Some(io_enable),
        io_free: Some(io_free),
        io_set_destroy: Some(io_set_destroy),
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

impl MainloopState {
    /// Prepare a callback only for a still-enabled, matching registration.
    pub(super) fn io_callback(&self, id: u64, generation: u64) -> Option<IoCallback> {
        let registration = self.watches.get(&id)?;
        // SAFETY: lookup runs without an FFI callback in progress, and the
        // registry owns this watch. Only copied arguments escape this method.
        let watch = unsafe { &*registration.get() };
        if watch.generation != generation || watch.events == PA_IO_EVENT_NULL {
            return None;
        }
        Some(IoCallback {
            api: watch.api,
            event: registration.get().cast(),
            fd: watch.fd,
            callback: watch.callback,
            userdata: watch.userdata,
        })
    }

    /// Copy each watch's (ID, generation, descriptor, requested conditions).
    /// The caller must not invoke libpulse while iterating over the registry.
    pub(super) fn io_watches(
        &self,
    ) -> impl Iterator<Item = (u64, u64, RawFd, pa_io_event_flags_t)> + '_ {
        self.watches.iter().map(|(&id, watch)| {
            // SAFETY: the watch is owned by the registry, and no FFI callback
            // runs during iteration. No reference to the watch escapes.
            let watch = unsafe { &*watch.get() };
            (id, watch.generation, watch.fd, watch.events)
        })
    }

    fn insert_io_watch(
        &mut self,
        api: *const pa_mainloop_api,
        fd: RawFd,
        events: pa_io_event_flags_t,
        callback: pa_io_event_cb_t,
        userdata: *mut c_void,
    ) -> *mut pa_io_event {
        let id = self.next_watch_id;
        self.next_watch_id += 1;

        let watch = Box::new(UnsafeCell::new(IoWatch {
            id,
            api,
            fd,
            events,
            callback,
            userdata,
            destroy_callback: None,
            generation: 0,
        }));
        let event = watch.get().cast::<pa_io_event>();
        self.watches.insert(id, watch);
        event
    }
}

/*
FFI functions from here
*/

// The API and its userdata must outlive every registered watch. Its userdata
// points to MainloopState, with no outstanding Rust references during callbacks.
extern "C" fn io_new(
    api: *const pa_mainloop_api,
    fd: RawFd,
    events: pa_io_event_flags_t,
    callback: pa_io_event_cb_t,
    userdata: *mut c_void,
) -> *mut pa_io_event {
    // SAFETY: the adapter supplies a live API whose userdata points to its
    // state. All calls are single-threaded, without overlapping state borrows.
    let state = unsafe { &mut *(*api).userdata.cast::<MainloopState>() };
    state.insert_io_watch(api, fd, events, callback, userdata)
}

extern "C" fn io_enable(event: *mut pa_io_event, events: pa_io_event_flags_t) {
    // SAFETY: libpulse passes a live handle returned by io_new. The value is
    // inside UnsafeCell, and no other reference to this watch is held here.
    let watch = unsafe { &mut *event.cast::<IoWatch>() };
    if watch.events != events {
        watch.events = events;
        watch.generation += 1;
    }
}

extern "C" fn io_free(event: *mut pa_io_event) {
    // SAFETY: the handle is live and belongs to this adapter. Copy the owner
    // information without retaining a reference across the registry mutation.
    let (api, id) = unsafe {
        let watch = &*event.cast::<IoWatch>();
        (watch.api, watch.id)
    };

    let removed = {
        // SAFETY: the owning API and state remain alive; no other state borrow
        // is active. Removing an entry does not close libpulse's descriptor.
        let state = unsafe { &mut *(*api).userdata.cast::<MainloopState>() };
        state.watches.remove(&id)
    };

    if let Some(watch) = removed {
        // SAFETY: the removed box still owns the watch. Copy callback data so
        // no reference to it or the registry is held while invoking libpulse.
        let (destroy, userdata) = unsafe {
            let watch = &*watch.get();
            (watch.destroy_callback, watch.userdata)
        };
        if let Some(destroy) = destroy {
            destroy(api, watch.get().cast(), userdata);
        }
        // The box drops after the destroy callback returns. Future poll
        // dispatch must look up the watch by ID, never keep this raw pointer.
    }
}

extern "C" fn io_set_destroy(event: *mut pa_io_event, callback: pa_io_event_destroy_cb_t) {
    // SAFETY: as with io_enable, this is a live, exclusively accessed watch.
    let watch = unsafe { &mut *event.cast::<IoWatch>() };
    watch.destroy_callback = callback;
}

#[cfg(test)]
mod tests;
