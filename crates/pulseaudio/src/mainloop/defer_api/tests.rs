use std::cell::Cell;
use std::ptr;

use super::super::make_api;
use super::*;

struct TestMainloop {
    state: Box<UnsafeCell<MainloopState>>,
    api: Box<pa_mainloop_api>,
}

impl TestMainloop {
    fn new() -> Self {
        let state = Box::new(UnsafeCell::new(MainloopState::default()));
        let api = Box::new(make_api(state.as_ref()));
        Self { state, api }
    }

    fn add(&self, userdata: *mut c_void) -> *mut pa_defer_event {
        self.api.defer_new.unwrap()(self.api.as_ref(), Some(on_deferred), userdata)
    }

    fn count(&self) -> usize {
        // SAFETY: no FFI callback is active, and no reference escapes.
        unsafe { (*self.state.get()).deferred.len() }
    }
}

extern "C" fn on_deferred(_: *const pa_mainloop_api, _: *mut pa_defer_event, _: *mut c_void) {}

fn status(event: *mut pa_defer_event) -> (bool, u64) {
    // SAFETY: tests pass live registrations, outside FFI callbacks.
    unsafe {
        let entry = &*event.cast::<DeferEntry>();
        (entry.enabled, entry.generation)
    }
}

#[test]
fn defer_new_registers_enabled_work_and_callback_data() {
    let mainloop = TestMainloop::new();
    let data = Cell::new(0_usize);
    let userdata = ptr::from_ref(&data).cast_mut().cast();
    let event = mainloop.add(userdata);

    assert!(!event.is_null());
    assert_eq!(mainloop.count(), 1);
    assert_eq!(status(event), (true, 0));
    // SAFETY: the registration is live and no callback is running.
    let entry = unsafe { &*event.cast::<DeferEntry>() };
    assert_eq!(entry.api, ptr::from_ref(mainloop.api.as_ref()));
    assert_eq!(entry.userdata, userdata);
    assert!(entry.callback.is_some());
    assert!(entry.destroy_callback.is_none());
}

#[test]
fn defer_enable_uses_c_boolean_semantics_and_invalidates_changed_registrations() {
    let mainloop = TestMainloop::new();
    let event = mainloop.add(ptr::null_mut());

    defer_enable(event, 0);
    assert_eq!(status(event), (false, 1));
    defer_enable(event, -1);
    assert_eq!(status(event), (true, 2));
    defer_enable(event, 42);
    assert_eq!(status(event), (true, 2));
    assert_eq!(mainloop.count(), 1, "disabled is not freed");
}

#[test]
fn handles_remain_valid_when_the_deferred_registry_grows() {
    let mainloop = TestMainloop::new();
    let first = mainloop.add(ptr::null_mut());
    for _ in 0..256 {
        mainloop.add(ptr::null_mut());
    }

    defer_enable(first, 0);
    assert_eq!(status(first), (false, 1));
    defer_free(first);
    assert_eq!(mainloop.count(), 256);
}

extern "C" fn on_destroy(_: *const pa_mainloop_api, _: *mut pa_defer_event, userdata: *mut c_void) {
    // SAFETY: the test keeps this Cell alive through the destroy callback.
    let count = unsafe { &*userdata.cast::<Cell<usize>>() };
    count.set(count.get() + 1);
}

#[test]
fn defer_free_removes_only_its_entry_and_invokes_the_destroy_callback_once() {
    let mainloop = TestMainloop::new();
    let count = Cell::new(0_usize);
    let first = mainloop.add(ptr::from_ref(&count).cast_mut().cast());
    let second = mainloop.add(ptr::null_mut());
    defer_set_destroy(first, Some(on_destroy));

    defer_free(first);
    assert_eq!(count.get(), 1);
    assert_eq!(mainloop.count(), 1);
    assert_eq!(status(second), (true, 0));
    defer_free(second);
    assert_eq!(count.get(), 1);
    assert_eq!(mainloop.count(), 0);
}

#[test]
fn defer_set_destroy_can_clear_the_callback() {
    let mainloop = TestMainloop::new();
    let count = Cell::new(0_usize);
    let event = mainloop.add(ptr::from_ref(&count).cast_mut().cast());
    defer_set_destroy(event, Some(on_destroy));
    defer_set_destroy(event, None);

    defer_free(event);
    assert_eq!(count.get(), 0);
}

extern "C" fn replace_on_destroy(
    api: *const pa_mainloop_api,
    _: *mut pa_defer_event,
    userdata: *mut c_void,
) {
    let event = defer_new(api, Some(on_deferred), ptr::null_mut());
    // SAFETY: the test keeps this Cell alive through the callback.
    let replacement = unsafe { &*userdata.cast::<Cell<*mut pa_defer_event>>() };
    replacement.set(event);
}

#[test]
fn destroy_callbacks_can_register_more_deferred_work() {
    let mainloop = TestMainloop::new();
    let replacement = Cell::new(ptr::null_mut::<pa_defer_event>());
    let event = mainloop.add(ptr::from_ref(&replacement).cast_mut().cast());
    // SAFETY: the original registration is live.
    let old_id = unsafe { (*event.cast::<DeferEntry>()).id };
    defer_set_destroy(event, Some(replace_on_destroy));

    defer_free(event);

    let new_event = replacement.get();
    assert!(!new_event.is_null());
    assert_eq!(mainloop.count(), 1);
    assert_eq!(status(new_event), (true, 0));
    // SAFETY: the callback created this replacement registration.
    assert_ne!(unsafe { (*new_event.cast::<DeferEntry>()).id }, old_id);
    defer_free(new_event);
}

#[test]
fn deferred_snapshots_reject_removed_or_changed_entries() {
    let mainloop = TestMainloop::new();
    let event = mainloop.add(ptr::null_mut());
    // SAFETY: snapshotting occurs without FFI callbacks in progress.
    let entries = unsafe { &*mainloop.state.get() }.enabled_deferred();
    let (id, generation) = entries[0];

    defer_enable(event, 0);
    defer_enable(event, 1);
    // SAFETY: the enable hooks have returned, leaving no overlapping borrow.
    assert!(
        unsafe { &*mainloop.state.get() }
            .defer_callback(id, generation)
            .is_none()
    );
    defer_free(event);
    // SAFETY: the free hook has returned.
    assert!(!unsafe { &*mainloop.state.get() }.has_deferred_work());
}
