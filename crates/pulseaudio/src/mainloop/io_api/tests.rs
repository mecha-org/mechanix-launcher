use std::cell::Cell;
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::ptr;

use super::super::make_api;
use super::*;

// Only the I/O hooks are exercised here. This incomplete API must not yet be
// passed to a libpulse context: the quit hook is not implemented yet.
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

    fn add(
        &self,
        fd: RawFd,
        events: pa_io_event_flags_t,
        userdata: *mut c_void,
    ) -> *mut pa_io_event {
        self.api.io_new.unwrap()(&*self.api, fd, events, Some(on_ready), userdata)
    }

    fn watch_count(&self) -> usize {
        // SAFETY: no FFI callback is running, and this borrow does not escape.
        unsafe { (*self.state.get()).watches.len() }
    }
}

extern "C" fn on_ready(
    _: *const pa_mainloop_api,
    _: *mut pa_io_event,
    _: i32,
    _: pa_io_event_flags_t,
    _: *mut c_void,
) {
}

fn interests(event: *mut pa_io_event) -> (pa_io_event_flags_t, u64) {
    // SAFETY: tests only pass live handles, outside any FFI callback.
    unsafe {
        let watch = &*event.cast::<IoWatch>();
        (watch.events, watch.generation)
    }
}

#[test]
fn io_new_registers_the_watch_and_callback_data() {
    let mainloop = TestMainloop::new();
    let data = Cell::new(0_u32);
    let userdata = ptr::from_ref(&data).cast_mut().cast();
    let event = mainloop.add(42, PA_IO_EVENT_INPUT, userdata);

    assert!(!event.is_null());
    assert_eq!(mainloop.watch_count(), 1);
    // SAFETY: the registration is still live and no callback is running.
    let watch = unsafe { &*event.cast::<IoWatch>() };
    assert_eq!(watch.api, ptr::from_ref(&*mainloop.api));
    assert_eq!(watch.fd, 42);
    assert_eq!(watch.events, PA_IO_EVENT_INPUT);
    assert_eq!(watch.generation, 0);
    assert_eq!(watch.userdata, userdata);
    assert!(watch.callback.is_some());
    assert!(watch.destroy_callback.is_none());
}

#[test]
fn io_enable_invalidates_older_interests_only_when_the_mask_changes() {
    let mainloop = TestMainloop::new();
    let event = mainloop.add(42, PA_IO_EVENT_INPUT, ptr::null_mut());

    io_enable(event, PA_IO_EVENT_OUTPUT);
    assert_eq!(interests(event), (PA_IO_EVENT_OUTPUT, 1));
    io_enable(event, PA_IO_EVENT_OUTPUT);
    assert_eq!(interests(event), (PA_IO_EVENT_OUTPUT, 1));
    io_enable(event, PA_IO_EVENT_NULL);
    assert_eq!(interests(event), (PA_IO_EVENT_NULL, 2));
    assert_eq!(mainloop.watch_count(), 1, "disabled is not freed");
}

#[test]
fn handles_remain_valid_when_the_registry_grows() {
    let mainloop = TestMainloop::new();
    let first = mainloop.add(42, PA_IO_EVENT_INPUT, ptr::null_mut());
    for _ in 0..256 {
        mainloop.add(43, PA_IO_EVENT_OUTPUT, ptr::null_mut());
    }

    io_enable(first, PA_IO_EVENT_OUTPUT);
    assert_eq!(interests(first), (PA_IO_EVENT_OUTPUT, 1));
    io_free(first);
    assert_eq!(mainloop.watch_count(), 256);
}

extern "C" fn on_destroy(_: *const pa_mainloop_api, _: *mut pa_io_event, userdata: *mut c_void) {
    // SAFETY: the test supplies a live Cell for the duration of this callback.
    let count = unsafe { &*userdata.cast::<Cell<usize>>() };
    count.set(count.get() + 1);
}

#[test]
fn io_free_removes_only_its_watch_and_calls_the_destroy_callback_once() {
    let mainloop = TestMainloop::new();
    let count = Cell::new(0_usize);
    let first = mainloop.add(
        42,
        PA_IO_EVENT_INPUT,
        ptr::from_ref(&count).cast_mut().cast(),
    );
    let second = mainloop.add(43, PA_IO_EVENT_OUTPUT, ptr::null_mut());
    io_set_destroy(first, Some(on_destroy));

    io_free(first);
    assert_eq!(count.get(), 1);
    assert_eq!(mainloop.watch_count(), 1);
    assert_eq!(interests(second), (PA_IO_EVENT_OUTPUT, 0));
    io_free(second);
    assert_eq!(count.get(), 1);
    assert_eq!(mainloop.watch_count(), 0);
}

#[test]
fn io_free_does_not_close_libpulses_descriptor() {
    let mainloop = TestMainloop::new();
    let (mut reader, mut writer) = UnixStream::pair().unwrap();
    let event = mainloop.add(reader.as_raw_fd(), PA_IO_EVENT_INPUT, ptr::null_mut());

    io_free(event);

    writer.write_all(b"x").unwrap();
    let mut byte = [0];
    reader.read_exact(&mut byte).unwrap();
    assert_eq!(byte, *b"x");
}

extern "C" fn replace_on_destroy(
    api: *const pa_mainloop_api,
    _: *mut pa_io_event,
    userdata: *mut c_void,
) {
    let event = io_new(api, 43, PA_IO_EVENT_OUTPUT, Some(on_ready), ptr::null_mut());
    // SAFETY: the test keeps this Cell alive throughout the callback.
    let replacement = unsafe { &*userdata.cast::<Cell<*mut pa_io_event>>() };
    replacement.set(event);
}

#[test]
fn destroy_callbacks_can_register_another_watch() {
    let mainloop = TestMainloop::new();
    let replacement = Cell::new(ptr::null_mut::<pa_io_event>());
    let event = mainloop.add(
        42,
        PA_IO_EVENT_INPUT,
        ptr::from_ref(&replacement).cast_mut().cast(),
    );
    // SAFETY: the original registration is live.
    let old_id = unsafe { (*event.cast::<IoWatch>()).id };
    io_set_destroy(event, Some(replace_on_destroy));

    io_free(event);

    let new_event = replacement.get();
    assert!(!new_event.is_null());
    assert_eq!(mainloop.watch_count(), 1);
    assert_eq!(interests(new_event), (PA_IO_EVENT_OUTPUT, 0));
    // SAFETY: the destroy callback registered this replacement watch.
    assert_ne!(unsafe { (*new_event.cast::<IoWatch>()).id }, old_id);
    io_free(new_event);
}
