//! Deferred-work part of the libpulse asynchronous mainloop API.
//!
//! Enabled entries run once per loop turn, without a descriptor or deadline.

use std::cell::UnsafeCell;
use std::ffi::c_void;

use libpulse_sys::mainloop::api::*;

use super::MainloopState;

pub(super) struct DeferEntry {
    id: u64,
    api: *const pa_mainloop_api,
    enabled: bool,
    callback: pa_defer_event_cb_t,
    userdata: *mut c_void,
    destroy_callback: pa_defer_event_destroy_cb_t,
    generation: u64,
}

/// Copied arguments, so deferred callbacks run without registry borrows.
pub(crate) struct DeferCallback {
    api: *const pa_mainloop_api,
    event: *mut pa_defer_event,
    callback: pa_defer_event_cb_t,
    userdata: *mut c_void,
}

impl DeferCallback {
    /// # Safety
    /// The API, registration and callback data must still be live and unchanged
    /// since lookup. No state or resource borrows may be held during invocation.
    pub(crate) unsafe fn invoke(self) {
        if let Some(callback) = self.callback {
            callback(self.api, self.event, self.userdata);
        }
    }
}

impl MainloopState {
    /// Snapshot enabled IDs/generations, in creation order, for one loop turn.
    pub(crate) fn enabled_deferred(&self) -> Vec<(u64, u64)> {
        let mut entries: Vec<_> = self
            .deferred
            .iter()
            .filter_map(|(&id, entry)| {
                // SAFETY: no FFI callback runs during iteration. Only copied
                // IDs and generations escape this lookup.
                let entry = unsafe { &*entry.get() };
                entry.enabled.then_some((id, entry.generation))
            })
            .collect();
        entries.sort_unstable_by_key(|&(id, _)| id);
        entries
    }

    pub(crate) fn has_deferred_work(&self) -> bool {
        self.deferred.values().any(|entry| {
            // SAFETY: this check runs outside FFI callbacks.
            unsafe { (*entry.get()).enabled }
        })
    }

    pub(crate) fn defer_callback(&self, id: u64, generation: u64) -> Option<DeferCallback> {
        let registration = self.deferred.get(&id)?;
        // SAFETY: the registry owns the entry and lookup runs without an FFI
        // callback in progress. No references escape this method.
        let entry = unsafe { &*registration.get() };
        if !entry.enabled || entry.generation != generation {
            return None;
        }
        Some(DeferCallback {
            api: entry.api,
            event: registration.get().cast(),
            callback: entry.callback,
            userdata: entry.userdata,
        })
    }

    fn insert_deferred(
        &mut self,
        api: *const pa_mainloop_api,
        callback: pa_defer_event_cb_t,
        userdata: *mut c_void,
    ) -> *mut pa_defer_event {
        let id = self.next_defer_id;
        self.next_defer_id += 1;

        let entry = Box::new(UnsafeCell::new(DeferEntry {
            id,
            api,
            enabled: true,
            callback,
            userdata,
            destroy_callback: None,
            generation: 0,
        }));
        let event = entry.get().cast::<pa_defer_event>();
        self.deferred.insert(id, entry);
        event
    }
}

/// New deferred work starts enabled, but registration never invokes it inline.
pub(super) extern "C" fn defer_new(
    api: *const pa_mainloop_api,
    callback: pa_defer_event_cb_t,
    userdata: *mut c_void,
) -> *mut pa_defer_event {
    // SAFETY: the adapter supplies a live API and state, with no overlapping
    // state borrows or concurrent callbacks.
    let state = unsafe { &mut *(*api).userdata.cast::<MainloopState>() };
    state.insert_deferred(api, callback, userdata)
}

pub(super) extern "C" fn defer_enable(event: *mut pa_defer_event, enabled: i32) {
    // SAFETY: libpulse passes a live, exclusively accessed registration.
    let entry = unsafe { &mut *event.cast::<DeferEntry>() };
    let enabled = enabled != 0;
    if entry.enabled != enabled {
        entry.enabled = enabled;
        entry.generation += 1;
    }
}

pub(super) extern "C" fn defer_free(event: *mut pa_defer_event) {
    // SAFETY: the handle is live and belongs to this adapter. Copy its owner
    // information before mutating the registry.
    let (api, id) = unsafe {
        let entry = &*event.cast::<DeferEntry>();
        (entry.api, entry.id)
    };
    let removed = {
        // SAFETY: the owning state is live, with no other state borrow active.
        let state = unsafe { &mut *(*api).userdata.cast::<MainloopState>() };
        state.deferred.remove(&id)
    };

    if let Some(entry) = removed {
        // SAFETY: the local box keeps the entry alive through its destroy
        // callback. No registry/entry references are held during invocation.
        let (destroy, userdata) = unsafe {
            let entry = &*entry.get();
            (entry.destroy_callback, entry.userdata)
        };
        if let Some(destroy) = destroy {
            destroy(api, entry.get().cast(), userdata);
        }
    }
}

pub(super) extern "C" fn defer_set_destroy(
    event: *mut pa_defer_event,
    callback: pa_defer_event_destroy_cb_t,
) {
    // SAFETY: this is a live, exclusively accessed registration from defer_new.
    let entry = unsafe { &mut *event.cast::<DeferEntry>() };
    entry.destroy_callback = callback;
}

#[cfg(test)]
mod tests;
