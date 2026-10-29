//! Shared PulseAudio connection, driven by mecha-wayland's Ring.
//!
//! `PulseAudioModule` constructs a libpulse context with our mainloop API.
//! Libpulse registers watches/timers/deferred work; `BeforeWait` submits their
//! Ring operations, and `IoEvent` gives the completions back to libpulse.
//! Connection callbacks only queue owned state changes. On the next `Tick`,
//! those become app-wide `ConnectionStateChanged` signals. Nothing here starts
//! a second event loop or a thread.

use std::any::Any;
use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::io;
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::rc::Rc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use libc::timeval;
use libpulse_sys::mainloop::api::*;
use mecha_wayland::prelude::*;

mod context;
mod mainloop;
mod volume;

// re-export
pub use context::{ConnectionState, PulseAudioContextConnectionStateChanged};
pub use volume::{SetMuted, SetVolume, VolumeControlModule, VolumeState};

/// Connect to the local PulseAudio-compatible server through Ring.
///
/// Uses `$XDG_RUNTIME_DIR/pulse/native` without autospawning a daemon. Install
/// after `RingModule`; [`ConnectionStateChanged`] reports readiness or failure.
/// Queue actions in ordinary systems/handlers; BeforeWait drains libpulse work.
pub struct PulseAudioModule;

impl Module for PulseAudioModule {
    fn install(self, app: &mut App) {
        // Install the shared state and systems before asking libpulse to
        // connect. It may register I/O watches and timers immediately.
        app.init_resource::<PulseAudio>()
            .system(context::on_tick)
            .system(on_before_wait)
            .system(on_io);
        // This starts connecting without blocking; the context module owns
        // the setup/error path and later publishes state changes on Tick.
        app.resource_mut::<PulseAudio>().start_connection();
    }
}

/// App-wide owner of the connection, its mainloop API, and Ring bookkeeping.
///
/// The context and API have stable addresses because libpulse holds pointers
/// to them. `Drop` disconnects the context while the API is still alive.
pub struct PulseAudio {
    // The vtable passed to libpulse. Its userdata points to mainloop_state.
    mainloop_api: Box<pa_mainloop_api>,
    mainloop_state: Box<UnsafeCell<mainloop::MainloopState>>,
    // None until install starts connecting (or if setup failed early).
    context: Option<context::Context>,
    // Cached public state, updated on Tick rather than in a C callback.
    connection_state: ConnectionState,
    // Setup errors that occurred before a context could exist.
    connection_updates: Vec<ConnectionState>,
    // Keep callback data alive if its service resource is dropped before the
    // C context. This shared owner knows nothing about the data's type.
    callback_data: Vec<Rc<dyn Any>>,
    pending_polls: HashMap<Token, PendingPoll>,
    pending_timers: HashMap<Token, PendingTimer>,
    defer_wake: Option<Token>,
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
            context: None,
            connection_state: ConnectionState::Connecting,
            connection_updates: Vec::new(),
            callback_data: Vec::new(),
            pending_polls: HashMap::new(),
            pending_timers: HashMap::new(),
            defer_wake: None,
        }
    }

    /// Retain a service's callback data until the context disconnects.
    pub(crate) fn keep_callback_data<T: 'static>(&mut self, data: Rc<T>) {
        self.callback_data.push(data);
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

    /// Submit one timeout for each enabled timer's current generation.
    fn arm_timers(&mut self, ring: &mut Ring) -> io::Result<()> {
        // SAFETY: no libpulse callbacks run while inspecting registrations or
        // submitting Ring operations. The iterator yields copied metadata.
        let state = unsafe { &*self.mainloop_state.get() };
        for (timer_id, generation, deadline) in state.enabled_timers() {
            if self
                .pending_timers
                .values()
                .any(|timer| timer.timer_id == timer_id && timer.generation == generation)
            {
                continue;
            }
            let delay = timer_delay(deadline, SystemTime::now())?;
            let token = ring.timeout(delay);
            self.pending_timers.insert(
                token,
                PendingTimer {
                    timer_id,
                    generation,
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
    // Deferred callbacks can register new watches and timers. Run them first,
    // then arm their requests before Ring blocks for a completion.
    run_deferred(app);

    let (mut audio, mut ring) = app.query::<(ResMut<PulseAudio>, ResMut<Ring>)>();
    if let Err(error) = audio
        .arm_polls(&mut ring)
        .and_then(|()| audio.arm_timers(&mut ring))
    {
        eprintln!("PulseAudio scheduling failed: {error}");
        audio
            .connection_updates
            .push(ConnectionState::Failed(error.to_string()));
    }

    // If libpulse queued a state transition here and has no more I/O, Ring
    // still needs a wake so the next Tick can publish it (or exit on Stop).
    // SAFETY: all deferred callbacks have returned, and no FFI runs while
    // checking the registry or queueing Ring operations.
    let state = unsafe { &*audio.mainloop_state.get() };
    if (state.has_deferred_work() || audio.has_connection_updates()) && audio.defer_wake.is_none() {
        // Runnable work must not sleep waiting for unrelated socket activity.
        // This is a one-shot wake, not a periodic polling timer.
        audio.defer_wake = Some(ring.timeout(Duration::ZERO));
    }
}

fn run_deferred(app: &mut App) {
    let entries = {
        let audio = app.resource::<PulseAudio>();
        // SAFETY: no FFI runs during lookup. This returns copied IDs only.
        let state = unsafe { &*audio.mainloop_state.get() };
        state.enabled_deferred()
    };

    // One pass per turn: work created or re-enabled by a callback can run on
    // the next turn instead of growing an unbounded dispatch loop here.
    for (id, generation) in entries {
        let callback = {
            let audio = app.resource::<PulseAudio>();
            // SAFETY: the previous callback has returned. Revalidate each entry
            // because another callback may have disabled or freed it.
            let state = unsafe { &*audio.mainloop_state.get() };
            state.defer_callback(id, generation)
        };
        if let Some(callback) = callback {
            // SAFETY: the entry was just validated. No resource/state borrows
            // remain, and no intervening FFI callback can invalidate it.
            unsafe { callback.invoke() };
        }
    }
}

fn on_io(app: &mut App, event: &IoEvent) {
    if app.resource::<PulseAudio>().defer_wake == Some(event.token) {
        if app.resource_mut::<Ring>().finish(event.token).is_some() {
            app.resource_mut::<PulseAudio>().defer_wake = None;
        }
        return; // A zero-duration timeout normally completes with -ETIME.
    }

    let timer = app
        .resource_mut::<PulseAudio>()
        .pending_timers
        .remove(&event.token);
    if let Some(timer) = timer {
        on_timer(app, event, timer);
        return;
    }

    // get token we own
    let Some(PendingPoll {
        watch_id,
        generation,
        fd,
    }) = app
        .resource_mut::<PulseAudio>()
        .pending_polls
        .remove(&event.token)
    else {
        return; // Another module owns this token.
    };

    if app.resource_mut::<Ring>().finish(event.token).is_none() {
        // A token that is not actually completed must retain its duplicated
        // descriptor; the kernel may still be using it.
        app.resource_mut::<PulseAudio>().pending_polls.insert(
            event.token,
            PendingPoll {
                watch_id,
                generation,
                fd,
            },
        );
        return;
    }
    // The kernel is done with the duplicated descriptor now.
    drop(fd);

    // Run the callback
    let callback = {
        let audio = app.resource::<PulseAudio>();
        // SAFETY: no FFI callback runs during lookup. The result contains
        // copied arguments, not references into the registry.
        let state = unsafe { &*audio.mainloop_state.get() };
        state.io_callback(watch_id, generation)
    };
    let Some(callback) = callback else {
        return; // The watch was removed, disabled, or changed.
    };

    // A failed poll belongs to the audio connection, not the whole App.
    // Let libpulse handle it as an I/O error rather than panicking here.
    let flags = if event.result < 0 {
        PA_IO_EVENT_ERROR
    } else {
        to_pulse_flags(event.result as u32)
    };

    // SAFETY: no resource/state borrows remain, and no foreign callbacks ran
    // between validating this watch and invoking it. It may free itself here.
    unsafe { callback.invoke(flags) };
}

fn on_timer(app: &mut App, event: &IoEvent, timer: PendingTimer) {
    // Finish even obsolete timeouts, so Ring can release their operation storage.
    if app.resource_mut::<Ring>().finish(event.token).is_none() {
        app.resource_mut::<PulseAudio>()
            .pending_timers
            .insert(event.token, timer);
        return;
    }

    let callback = {
        let audio = app.resource::<PulseAudio>();
        // SAFETY: no FFI callback is active. Lookup and preparation only copy
        // arguments and mutate the timer's interior state before invocation.
        let state = unsafe { &*audio.mainloop_state.get() };
        let Some(deadline) = state.timer_deadline(timer.timer_id, timer.generation) else {
            return; // The timer was removed, disabled, or restarted.
        };
        if event.result != -libc::ETIME {
            eprintln!("PulseAudio timer failed: {}", event.result);
            return;
        }

        // Ring waits relatively, while libpulse's custom-loop deadlines use
        // wall time. If the clock moved backwards, leave the timer enabled so
        // BeforeWait can resubmit it with the newly remaining delay.
        if !timer_delay(deadline, SystemTime::now()).is_ok_and(|delay| delay.is_zero()) {
            return;
        }
        state.take_timer_callback(timer.timer_id, timer.generation)
    };

    if let Some(callback) = callback {
        // SAFETY: the timer was just validated and disabled. All resource/state
        // borrows have ended, so the callback may restart or free its timer.
        unsafe { callback.invoke() };
    }
}

/// One submitted poll. Look up its watch by ID and generation on completion.
struct PendingPoll {
    watch_id: u64,
    generation: u64,
    // An owned duplicate, kept open until the poll is finished.
    fd: OwnedFd,
}

struct PendingTimer {
    timer_id: u64,
    generation: u64,
}

/// Libpulse uses absolute wall-clock timevals for custom mainloop APIs.
/// Convert them to Ring's relative durations, expiring past deadlines immediately.
fn timer_delay(deadline: timeval, now: SystemTime) -> io::Result<Duration> {
    if !(0..1_000_000).contains(&deadline.tv_usec) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid timer microseconds",
        ));
    }
    #[allow(clippy::unnecessary_cast)] // libc::time_t can also be 32-bit.
    let seconds = Duration::from_secs(deadline.tv_sec.unsigned_abs() as u64);
    let timestamp = if deadline.tv_sec >= 0 {
        UNIX_EPOCH.checked_add(seconds)
    } else {
        UNIX_EPOCH.checked_sub(seconds)
    };
    let timestamp = timestamp
        .and_then(|time| time.checked_add(Duration::from_micros(deadline.tv_usec as u64)))
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "timer deadline out of range")
        })?;
    Ok(timestamp.duration_since(now).unwrap_or(Duration::ZERO))
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
    if events & (libc::POLLERR | libc::POLLNVAL) as u32 != 0 {
        flags |= PA_IO_EVENT_ERROR;
    }
    flags
}

#[cfg(test)]
mod tests;
