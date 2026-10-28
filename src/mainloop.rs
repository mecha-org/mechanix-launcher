use std::collections::HashMap;
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

// store boxes
struct MainloopState {
    // use Box for address stability, libpulse wants a pointer to the IoWatch
    watches: HashMap<u64, Box<IoWatch>>,
    next_watch_id: u64,
}

impl MainloopState {
    fn insert_io_watch(&mut self, watch: IoWatch) -> u64 {
        let id = self.next_watch_id;
        self.next_watch_id += 1;

        self.watches.insert(id, Box::new(watch));
        id
    }
}
