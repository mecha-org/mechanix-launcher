use std::os::fd::OwnedFd;

use libpulse_sys::mainloop::api::{
    PA_IO_EVENT_ERROR, PA_IO_EVENT_HANGUP, PA_IO_EVENT_INPUT, PA_IO_EVENT_NULL, PA_IO_EVENT_OUTPUT,
    pa_io_event_flags_t,
};
use mecha_wayland::prelude::*;

mod mainloop;

struct PulseAudioModule;

impl Module for PulseAudioModule {
    fn install(self, app: &mut App) {}
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
