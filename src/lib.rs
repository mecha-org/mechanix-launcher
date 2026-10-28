use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};

use libpulse_sys::mainloop::api::*;
use mecha_wayland::prelude::*;

mod mainloop;

pub struct PulseAudioModule;

impl Module for PulseAudioModule {
    fn install(self, app: &mut App) {
        app.init_resource::<PulseAudio>().system(on_before_wait);
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

    /// Submit one poll for each enabled watch's current generation.
    fn arm_polls(&mut self, ring: &mut Ring) -> io::Result<()> {
        // SAFETY: no libpulse callbacks run while we inspect the registry and
        // queue Ring operations. The iterator yields copied metadata only.
        let state = unsafe { &*self.mainloop_state.get() };
        for (watch_id, generation, fd, events) in state.io_watches() {
            if events == PA_IO_EVENT_NULL {
                continue;
            }
            if self
                .pending_polls
                .values()
                .any(|poll| poll.watch_id == watch_id && poll.generation == generation)
            {
                continue;
            }

            // SAFETY: an active registration supplies a live libpulse-owned
            // descriptor. Duplicate it without taking ownership of the original.
            let fd = unsafe { BorrowedFd::borrow_raw(fd) }.try_clone_to_owned()?;
            let token = ring.poll(fd.as_fd(), to_poll_flags(events));
            self.pending_polls.insert(
                token,
                PendingPoll {
                    watch_id,
                    generation,
                    fd,
                },
            );
        }
        Ok(())
    }
}

impl Default for PulseAudio {
    fn default() -> Self {
        Self::new()
    }
}

fn on_before_wait(app: &mut App, _: &BeforeWait) {
    let (mut audio, mut ring) = app.query::<(ResMut<PulseAudio>, ResMut<Ring>)>();
    audio
        .arm_polls(&mut ring)
        .expect("PulseAudio: duplicate a watched descriptor");
		  // TODO: remove this
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
