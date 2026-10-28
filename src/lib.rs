use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::os::fd::OwnedFd;

use libpulse_sys::mainloop::api::*;
use mecha_wayland::prelude::*;

mod mainloop;

pub struct PulseAudioModule;

impl Module for PulseAudioModule {
    fn install(self, app: &mut App) {
        app.init_resource::<PulseAudio>();
    }
}

/// App-wide owner of the mainloop allocations and pending Ring polls.
/// This currently initializes I/O hooks only; it does not connect to a server.
pub struct PulseAudio {
    mainloop_api: Box<pa_mainloop_api>,
    mainloop_state: Box<UnsafeCell<mainloop::MainloopState>>,
    pending_polls: HashMap<Token, PendingPoll>,
}

impl Resource for PulseAudio {}

impl PulseAudio {
    pub fn new() -> Self {
        // Both allocations stay put even when the resource moves into App.
        // UnsafeCell allows the FFI hooks to mutate the state through userdata.
        let mainloop_state = Box::new(UnsafeCell::new(mainloop::MainloopState::default()));
        let mainloop_api = Box::new(mainloop::make_api(mainloop_state.as_ref()));

        Self {
            mainloop_api,
            mainloop_state,
            pending_polls: HashMap::new(),
        }
    }
}

impl Default for PulseAudio {
    fn default() -> Self {
        Self::new()
    }
}

/// One submitted poll. Look up its watch by ID and generation on completion.
struct PendingPoll {
    watch_id: u64,
    generation: u64,
    // An owned duplicate, kept open until the poll is finished.
    fd: OwnedFd,
}

/// Translate libpulse's requested conditions into Ring's Linux poll mask.
fn to_poll_flags(events: pa_io_event_flags_t) -> u32 {
    let mut flags = 0;
    if events & PA_IO_EVENT_INPUT != 0 {
        flags |= libc::POLLIN as u32;
    }
    if events & PA_IO_EVENT_OUTPUT != 0 {
        flags |= libc::POLLOUT as u32;
    }
    if events & PA_IO_EVENT_HANGUP != 0 {
        flags |= libc::POLLHUP as u32;
    }
    if events & PA_IO_EVENT_ERROR != 0 {
        flags |= libc::POLLERR as u32;
    }
    flags
}

/// Translate a successful poll completion's readiness mask for libpulse.
/// Handle negative `IoEvent::result` errors before converting the mask to u32.
fn to_pulse_flags(events: u32) -> pa_io_event_flags_t {
    let mut flags = PA_IO_EVENT_NULL;
    if events & libc::POLLIN as u32 != 0 {
        flags |= PA_IO_EVENT_INPUT;
    }
    if events & libc::POLLOUT as u32 != 0 {
        flags |= PA_IO_EVENT_OUTPUT;
    }
    if events & libc::POLLHUP as u32 != 0 {
        flags |= PA_IO_EVENT_HANGUP;
    }
    if events & libc::POLLERR as u32 != 0 {
        flags |= PA_IO_EVENT_ERROR;
    }
    flags
}

#[cfg(test)]
mod tests;
