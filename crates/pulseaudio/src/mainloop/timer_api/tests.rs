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

    fn add(&self, deadline: Option<&timeval>, userdata: *mut c_void) -> *mut pa_time_event {
        self.api.time_new.unwrap()(
            self.api.as_ref(),
            deadline.map_or(ptr::null(), ptr::from_ref),
            Some(on_timeout),
            userdata,
        )
    }

    fn timer_count(&self) -> usize {
        // SAFETY: no FFI callback is running, and this borrow does not escape.
        unsafe { (*self.state.get()).timers.len() }
    }
}

extern "C" fn on_timeout(
    _: *const pa_mainloop_api,
    _: *mut pa_time_event,
    _: *const timeval,
    userdata: *mut c_void,
) {
    // SAFETY: tests supply NULL or a live Cell for the callback duration.
    if let Some(count) = unsafe { userdata.cast::<Cell<usize>>().as_ref() } {
        count.set(count.get() + 1);
    }
}

fn schedule(event: *mut pa_time_event) -> (Option<(libc::time_t, libc::suseconds_t)>, u64) {
    // SAFETY: tests only inspect live handles, outside any FFI callback.
    unsafe {
        let timer = &*event.cast::<TimerEntry>();
        (
            timer.deadline.as_ref().map(|tv| (tv.tv_sec, tv.tv_usec)),
            timer.generation,
        )
    }
}

#[test]
fn time_new_registers_the_timer_and_copies_its_deadline() {
    let mainloop = TestMainloop::new();
    let count = Cell::new(0_usize);
    let userdata = ptr::from_ref(&count).cast_mut().cast();
    let mut deadline = timeval {
        tv_sec: 123,
        tv_usec: 456,
    };
    let event = mainloop.add(Some(&deadline), userdata);

    deadline.tv_sec = 789;
    assert_ne!(schedule(event).0, Some((deadline.tv_sec, deadline.tv_usec)));
    assert_eq!(schedule(event), (Some((123, 456)), 0));
    assert_eq!(mainloop.timer_count(), 1);
    // SAFETY: the registration remains live and no callback is running.
    let timer = unsafe { &*event.cast::<TimerEntry>() };
    assert_eq!(timer.api, ptr::from_ref(mainloop.api.as_ref()));
    assert_eq!(timer.userdata, userdata);
    assert!(timer.callback.is_some());
    assert!(timer.destroy_callback.is_none());
    assert_eq!(count.get(), 0, "registration does not invoke the timeout");
}

#[test]
fn time_new_with_null_deadline_registers_a_disabled_timer() {
    let mainloop = TestMainloop::new();
    let event = mainloop.add(None, ptr::null_mut());

    assert!(!event.is_null());
    assert_eq!(schedule(event), (None, 0));
    assert_eq!(mainloop.timer_count(), 1);
}

#[test]
fn time_restart_copies_the_deadline_and_invalidates_the_previous_generation() {
    let mainloop = TestMainloop::new();
    let count = Cell::new(0_usize);
    let event = mainloop.add(None, ptr::from_ref(&count).cast_mut().cast());
    let mut deadline = timeval {
        tv_sec: 123,
        tv_usec: 456,
    };

    time_restart(event, &deadline);
    assert_eq!(schedule(event), (Some((123, 456)), 1));
    time_restart(event, &deadline);
    assert_eq!(schedule(event), (Some((123, 456)), 2));
    deadline.tv_sec = 789;
    assert_eq!(schedule(event), (Some((123, 456)), 2));

    time_restart(event, ptr::null());
    assert_eq!(schedule(event), (None, 3));
    assert_eq!(mainloop.timer_count(), 1, "disabled is not freed");
    time_restart(event, &deadline);
    assert_eq!(schedule(event), (Some((789, 456)), 4));
    assert_eq!(count.get(), 0, "restart does not invoke the timeout");
}

#[test]
fn timer_handles_remain_valid_when_the_registry_grows() {
    let mainloop = TestMainloop::new();
    let first = mainloop.add(None, ptr::null_mut());
    for _ in 0..256 {
        mainloop.add(None, ptr::null_mut());
    }

    let deadline = timeval {
        tv_sec: 123,
        tv_usec: 456,
    };
    time_restart(first, &deadline);
    assert_eq!(schedule(first), (Some((123, 456)), 1));
    time_free(first);
    assert_eq!(mainloop.timer_count(), 256);
}

extern "C" fn on_destroy(_: *const pa_mainloop_api, _: *mut pa_time_event, userdata: *mut c_void) {
    // SAFETY: the test keeps this Cell alive throughout the callback.
    let count = unsafe { &*userdata.cast::<Cell<usize>>() };
    count.set(count.get() + 1);
}

#[test]
fn time_free_removes_only_its_timer_and_calls_the_destroy_callback_once() {
    let mainloop = TestMainloop::new();
    let count = Cell::new(0_usize);
    let first = mainloop.add(None, ptr::from_ref(&count).cast_mut().cast());
    let second = mainloop.add(None, ptr::null_mut());
    time_set_destroy(first, Some(on_destroy));

    time_free(first);
    assert_eq!(count.get(), 1);
    assert_eq!(mainloop.timer_count(), 1);
    assert_eq!(schedule(second), (None, 0));
    time_free(second);
    assert_eq!(count.get(), 1);
    assert_eq!(mainloop.timer_count(), 0);
}

#[test]
fn time_set_destroy_can_clear_the_callback() {
    let mainloop = TestMainloop::new();
    let count = Cell::new(0_usize);
    let event = mainloop.add(None, ptr::from_ref(&count).cast_mut().cast());
    time_set_destroy(event, Some(on_destroy));
    time_set_destroy(event, None);

    time_free(event);
    assert_eq!(count.get(), 0);
    assert_eq!(mainloop.timer_count(), 0);
}

extern "C" fn replace_on_destroy(
    api: *const pa_mainloop_api,
    _: *mut pa_time_event,
    userdata: *mut c_void,
) {
    let event = time_new(api, ptr::null(), Some(on_timeout), ptr::null_mut());
    // SAFETY: the test keeps this Cell alive throughout the callback.
    let replacement = unsafe { &*userdata.cast::<Cell<*mut pa_time_event>>() };
    replacement.set(event);
}

#[test]
fn destroy_callbacks_can_register_another_timer() {
    let mainloop = TestMainloop::new();
    let replacement = Cell::new(ptr::null_mut::<pa_time_event>());
    let event = mainloop.add(None, ptr::from_ref(&replacement).cast_mut().cast());
    // SAFETY: the original registration is live.
    let old_id = unsafe { (*event.cast::<TimerEntry>()).id };
    time_set_destroy(event, Some(replace_on_destroy));

    time_free(event);

    let new_event = replacement.get();
    assert!(!new_event.is_null());
    assert_eq!(mainloop.timer_count(), 1);
    assert_eq!(schedule(new_event), (None, 0));
    // SAFETY: the destroy callback registered this replacement timer.
    assert_ne!(unsafe { (*new_event.cast::<TimerEntry>()).id }, old_id);
    time_free(new_event);
}

#[test]
fn taking_a_timer_callback_disables_it_before_invocation() {
    let mainloop = TestMainloop::new();
    let count = Cell::new(0_usize);
    let deadline = timeval {
        tv_sec: 123,
        tv_usec: 456,
    };
    let event = mainloop.add(Some(&deadline), ptr::from_ref(&count).cast_mut().cast());
    // SAFETY: the registration is live and no callback is active.
    let id = unsafe { (*event.cast::<TimerEntry>()).id };
    let callback = {
        // SAFETY: no FFI callback is active and no state borrow escapes.
        let state = unsafe { &*mainloop.state.get() };
        assert_eq!(state.enabled_timers().count(), 1);
        state.take_timer_callback(id, 0).unwrap()
    };

    assert_eq!(schedule(event), (None, 0));
    assert_eq!(callback.deadline.tv_sec, 123);
    assert_eq!(callback.deadline.tv_usec, 456);
    // SAFETY: the API, timer and test-owned callback data are still live.
    unsafe { callback.invoke() };
    assert_eq!(count.get(), 1);
    // SAFETY: the callback has returned.
    assert!(
        unsafe { &*mainloop.state.get() }
            .take_timer_callback(id, 0)
            .is_none()
    );
}

#[test]
fn stale_timer_lookup_does_not_disable_the_restarted_timer() {
    let mainloop = TestMainloop::new();
    let event = mainloop.add(None, ptr::null_mut());
    // SAFETY: the registration is live.
    let id = unsafe { (*event.cast::<TimerEntry>()).id };
    let deadline = timeval {
        tv_sec: 123,
        tv_usec: 456,
    };
    time_restart(event, &deadline);

    {
        // SAFETY: the restart hook has returned and no callback is active.
        let state = unsafe { &*mainloop.state.get() };
        assert!(state.timer_deadline(id, 0).is_none());
        assert!(state.take_timer_callback(id, 0).is_none());
        assert!(state.timer_deadline(id, 1).is_some());
    }
    assert_eq!(schedule(event), (Some((123, 456)), 1));

    time_free(event);
    // SAFETY: lookup by ID does not dereference the freed event handle.
    assert!(
        unsafe { &*mainloop.state.get() }
            .take_timer_callback(id, 1)
            .is_none()
    );
}
