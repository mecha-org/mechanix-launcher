use std::cell::Cell;
use std::ffi::c_void;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::ptr;
use std::time::Duration;

use super::*;

mod timers;

const FLAGS: [(pa_io_event_flags_t, u32); 4] = [
    (PA_IO_EVENT_INPUT, libc::POLLIN as u32),
    (PA_IO_EVENT_OUTPUT, libc::POLLOUT as u32),
    (PA_IO_EVENT_HANGUP, libc::POLLHUP as u32),
    (PA_IO_EVENT_ERROR, libc::POLLERR as u32),
];

#[test]
fn unconnected_fixture_initializes_the_pulse_audio_resource() {
    let app = polling_app();

    let audio = app.resource::<PulseAudio>();
    assert_eq!(
        audio.mainloop_api.userdata,
        audio.mainloop_state.get().cast()
    );
    assert!(audio.pending_polls.is_empty());
    assert!(audio.pending_cancels.is_empty());
    assert!(audio.pending_timers.is_empty());
    assert!(audio.context.is_none());
    assert_eq!(audio.connection_state(), &ConnectionState::Connecting);
    assert!(audio.mainloop_api.io_new.is_some());
    assert!(audio.mainloop_api.io_enable.is_some());
    assert!(audio.mainloop_api.io_free.is_some());
    assert!(audio.mainloop_api.io_set_destroy.is_some());
    assert!(audio.mainloop_api.time_new.is_some());
    assert!(audio.mainloop_api.time_restart.is_some());
    assert!(audio.mainloop_api.time_free.is_some());
    assert!(audio.mainloop_api.time_set_destroy.is_some());
    assert!(audio.mainloop_api.defer_new.is_some());
    assert!(audio.mainloop_api.defer_enable.is_some());
    assert!(audio.mainloop_api.defer_free.is_some());
    assert!(audio.mainloop_api.defer_set_destroy.is_some());
    assert!(audio.mainloop_api.quit.is_some());
    assert!(audio.defer_wake.is_none());
}

#[derive(Default)]
struct ConnectionLog(Vec<ConnectionState>);
impl Resource for ConnectionLog {}

fn record_connection(app: &mut App, event: &PulseAudioContextConnectionStateChanged) {
    app.resource_mut::<ConnectionLog>().0.push(event.0.clone());
}

#[test]
fn state_updates_are_published_as_app_signals_on_tick() {
    let mut app = polling_app();
    app.init_resource::<ConnectionLog>()
        .system(record_connection);

    app.resource_mut::<PulseAudio>().connection_updates.extend([
        ConnectionState::Ready,
        ConnectionState::Ready,
        ConnectionState::Failed("server disappeared".to_owned()),
    ]);

    app.tick();

    let expected = vec![
        ConnectionState::Ready,
        ConnectionState::Failed("server disappeared".to_owned()),
    ];
    assert_eq!(app.resource::<ConnectionLog>().0, expected);
    assert_eq!(
        app.resource::<PulseAudio>().connection_state(),
        expected.last().unwrap()
    );
    app.tick();
    assert_eq!(app.resource::<ConnectionLog>().0, expected);
}

#[test]
fn libpulse_quit_does_not_stop_the_app() {
    let mut app = polling_app();
    let api = app.resource::<PulseAudio>().mainloop_api.as_ref();
    api.quit.unwrap()(api, 42);

    app.tick();
    assert!(!app.resource::<Ring>().is_stopped());
}

#[test]
fn mainloop_allocations_remain_stable_when_the_resource_moves_into_app() {
    let audio = PulseAudio::new();
    let api = std::ptr::from_ref(audio.mainloop_api.as_ref());
    let state = audio.mainloop_state.get();

    let mut app = App::new();
    app.insert_resource(audio);

    let audio = app.resource::<PulseAudio>();
    assert_eq!(std::ptr::from_ref(audio.mainloop_api.as_ref()), api);
    assert_eq!(audio.mainloop_state.get(), state);
    assert_eq!(audio.mainloop_api.userdata, state.cast());
}

#[test]
fn initializing_the_resource_again_preserves_its_api() {
    let mut app = App::new();
    app.init_resource::<PulseAudio>();
    let api = std::ptr::from_ref(app.resource::<PulseAudio>().mainloop_api.as_ref());

    app.init_resource::<PulseAudio>();

    assert_eq!(
        std::ptr::from_ref(app.resource::<PulseAudio>().mainloop_api.as_ref()),
        api
    );
}

#[test]
fn pulse_flags_map_to_poll_flags() {
    assert_eq!(to_poll_flags(PA_IO_EVENT_NULL), 0);
    for (pulse, poll) in FLAGS {
        assert_eq!(to_poll_flags(pulse), poll);
    }
}

#[test]
fn poll_flags_map_to_pulse_flags() {
    assert_eq!(to_pulse_flags(0), PA_IO_EVENT_NULL);
    for (pulse, poll) in FLAGS {
        assert_eq!(to_pulse_flags(poll), pulse);
    }
}

#[test]
fn all_combinations_of_supported_flags_round_trip() {
    // The four PulseAudio flags occupy bits 0 through 3.
    for flags in 0..16 {
        assert_eq!(to_pulse_flags(to_poll_flags(flags)), flags);
    }
}

#[test]
fn unsupported_flags_are_ignored() {
    assert_eq!(to_poll_flags(1 << 31), 0);
    assert_eq!(to_pulse_flags(libc::POLLPRI as u32), PA_IO_EVENT_NULL);
}

extern "C" fn on_ready(
    _: *const pa_mainloop_api,
    _: *mut pa_io_event,
    _: i32,
    _: pa_io_event_flags_t,
    _: *mut c_void,
) {
}

fn add_watch(audio: &PulseAudio, fd: RawFd, events: pa_io_event_flags_t) -> *mut pa_io_event {
    audio.mainloop_api.io_new.unwrap()(
        audio.mainloop_api.as_ref(),
        fd,
        events,
        Some(on_ready),
        ptr::null_mut(),
    )
}

// Bound the wait so a broken readiness mask fails instead of hanging the test.
fn wait_for_poll(ring: &mut Ring, token: Token) -> IoEvent {
    let deadline = ring.timeout(Duration::from_secs(1));
    loop {
        for event in ring.wait() {
            if event.token == token {
                return event;
            }
            ring.finish(event.token).expect("operation completed");
            assert_ne!(event.token, deadline, "poll did not complete");
        }
    }
}

// Submission-only tests finish directly; dispatch tests use the on_io system.
fn finish_poll(ring: &mut Ring, token: Token) -> IoEvent {
    let event = wait_for_poll(ring, token);
    let completed = ring.finish(token).expect("poll completed");
    assert!(event.result >= 0, "poll failed: {event:?}");
    assert!(completed.buf.is_empty() && completed.fds.is_empty());
    event
}

#[test]
fn before_wait_arms_readable_watches_without_duplicate_polls() {
    let (mut reader, mut writer) = UnixStream::pair().unwrap();
    reader.set_nonblocking(true).unwrap();
    let mut app = polling_app();
    add_watch(
        app.resource::<PulseAudio>(),
        reader.as_raw_fd(),
        PA_IO_EVENT_INPUT,
    );

    // Exercise the registered system, including a second call with no changes.
    for _ in 0..2 {
        app.signal(BeforeWait);
        app.flush();
    }
    let token = {
        let audio = app.resource::<PulseAudio>();
        assert_eq!(audio.pending_polls.len(), 1);
        let (&token, pending) = audio.pending_polls.iter().next().unwrap();
        assert_eq!(pending.generation, 0);
        assert_ne!(pending.fd.as_raw_fd(), reader.as_raw_fd());
        token
    };

    writer.write_all(b"x").unwrap();
    let event = finish_poll(&mut app.resource_mut::<Ring>(), token);
    assert_ne!(event.result & libc::POLLIN as i32, 0);
    app.resource_mut::<PulseAudio>()
        .pending_polls
        .remove(&token);

    let mut byte = [0];
    reader.read_exact(&mut byte).unwrap();
    assert_eq!(byte, *b"x", "polling must not consume the data");
}

#[test]
fn disabled_watches_do_not_arm_polls() {
    let (reader, _writer) = UnixStream::pair().unwrap();
    let mut audio = PulseAudio::new();
    // Drop Ring before audio, keeping pending descriptors alive during shutdown.
    let mut ring = Ring::new(8);
    let event = add_watch(&audio, reader.as_raw_fd(), PA_IO_EVENT_NULL);

    audio.arm_polls(&mut ring).unwrap();
    assert!(audio.pending_polls.is_empty());

    audio.mainloop_api.io_enable.unwrap()(event, PA_IO_EVENT_INPUT);
    audio.arm_polls(&mut ring).unwrap();
    assert_eq!(audio.pending_polls.len(), 1);

    audio.mainloop_api.io_enable.unwrap()(event, PA_IO_EVENT_NULL);
    audio.cancel_obsolete_polls(&mut ring);
    audio.arm_polls(&mut ring).unwrap();
    assert_eq!(audio.pending_polls.len(), 1, "wait for the poll completion");
    assert_eq!(audio.pending_cancels.len(), 1);
}

#[test]
fn changed_interests_arm_a_new_poll_while_retiring_the_old_one() {
    let (reader, _writer) = UnixStream::pair().unwrap();
    let mut audio = PulseAudio::new();
    let mut ring = Ring::new(8);
    let event = add_watch(&audio, reader.as_raw_fd(), PA_IO_EVENT_INPUT);

    audio.arm_polls(&mut ring).unwrap();
    let (&old_token, old_poll) = audio.pending_polls.iter().next().unwrap();
    let watch_id = old_poll.watch_id;

    audio.mainloop_api.io_enable.unwrap()(event, PA_IO_EVENT_OUTPUT);
    audio.cancel_obsolete_polls(&mut ring);
    audio.arm_polls(&mut ring).unwrap();
    audio.arm_polls(&mut ring).unwrap();
    assert_eq!(audio.pending_polls.len(), 2);
    assert_eq!(audio.pending_cancels.len(), 1);
    assert_eq!(audio.pending_polls[&old_token].generation, 0);
    let (&new_token, new_poll) = audio
        .pending_polls
        .iter()
        .find(|(_, poll)| poll.generation == 1)
        .unwrap();
    assert_eq!(new_poll.watch_id, watch_id);

    let event = finish_poll(&mut ring, new_token);
    assert_ne!(event.result & libc::POLLOUT as i32, 0);
    audio.pending_polls.remove(&new_token);
}

#[test]
fn pending_poll_keeps_its_descriptor_after_the_watch_is_freed() {
    let (reader, mut writer) = UnixStream::pair().unwrap();
    let mut audio = PulseAudio::new();
    let mut ring = Ring::new(8);
    let event = add_watch(&audio, reader.as_raw_fd(), PA_IO_EVENT_INPUT);

    audio.arm_polls(&mut ring).unwrap();
    let token = *audio.pending_polls.keys().next().unwrap();
    audio.mainloop_api.io_free.unwrap()(event);
    drop(reader);
    audio.arm_polls(&mut ring).unwrap();
    assert_eq!(audio.pending_polls.len(), 1);

    // The original descriptor is closed before Ring submits the queued poll.
    writer.write_all(b"x").unwrap();
    let event = finish_poll(&mut ring, token);
    assert_ne!(event.result & libc::POLLIN as i32, 0);
    audio.pending_polls.remove(&token);
}

#[derive(Default)]
struct Readiness {
    calls: Cell<usize>,
    api: Cell<*const pa_mainloop_api>,
    event: Cell<*mut pa_io_event>,
    fd: Cell<RawFd>,
    flags: Cell<pa_io_event_flags_t>,
    replacement: Cell<*mut pa_io_event>,
}

extern "C" fn record_readiness(
    api: *const pa_mainloop_api,
    event: *mut pa_io_event,
    fd: i32,
    flags: pa_io_event_flags_t,
    userdata: *mut c_void,
) {
    // SAFETY: these tests keep their Readiness value alive while dispatching.
    let data = unsafe { &*userdata.cast::<Readiness>() };
    data.calls.set(data.calls.get() + 1);
    data.api.set(api);
    data.event.set(event);
    data.fd.set(fd);
    data.flags.set(flags);
}

fn polling_app() -> App {
    let mut app = App::new();
    app.add_module(RingModule::default())
        .init_resource::<PulseAudio>()
        .system(context::on_tick)
        .system(on_before_wait)
        .system(on_io);
    app
}

fn arm_callback(
    app: &mut App,
    fd: RawFd,
    flags: pa_io_event_flags_t,
    callback: pa_io_event_cb_t,
    data: &Readiness,
) -> (*mut pa_io_event, Token) {
    let event = {
        let audio = app.resource::<PulseAudio>();
        audio.mainloop_api.io_new.unwrap()(
            audio.mainloop_api.as_ref(),
            fd,
            flags,
            callback,
            ptr::from_ref(data).cast_mut().cast(),
        )
    };
    app.signal(BeforeWait);
    app.flush();
    let audio = app.resource::<PulseAudio>();
    assert_eq!(audio.pending_polls.len(), 1);
    (event, *audio.pending_polls.keys().next().unwrap())
}

fn dispatch_poll(app: &mut App, token: Token) {
    let event = wait_for_poll(&mut app.resource_mut::<Ring>(), token);
    app.signal(event);
    app.flush();
}

// Bound cancellation tests without relying on socket activity to wake Ring.
// Dispatch every CQE; a cancellation and its poll may complete in either order.
fn dispatch_cancelled_poll(app: &mut App, poll: Token) -> Vec<IoEvent> {
    let deadline = app.resource_mut::<Ring>().timeout(Duration::from_secs(1));
    let mut seen = Vec::new();
    for _ in 0..10 {
        for event in app.resource_mut::<Ring>().wait() {
            assert_ne!(event.token, deadline, "poll cancellation did not complete");
            app.signal(event);
            seen.push(event);
        }
        app.flush();
        let audio = app.resource::<PulseAudio>();
        if !audio.pending_polls.contains_key(&poll) && audio.pending_cancels.is_empty() {
            return seen;
        }
    }
    panic!("poll cancellation did not complete: {seen:?}");
}

#[test]
fn freeing_a_watch_cancels_its_poll_and_releases_its_fd() {
    let (reader, _writer) = UnixStream::pair().unwrap();
    let data = Readiness::default();
    let mut app = polling_app();
    let (watch, poll) = arm_callback(
        &mut app,
        reader.as_raw_fd(),
        PA_IO_EVENT_INPUT,
        Some(record_readiness),
        &data,
    );
    let fd = app.resource::<PulseAudio>().pending_polls[&poll]
        .fd
        .as_raw_fd();

    app.resource::<PulseAudio>().mainloop_api.io_free.unwrap()(watch);
    drop(reader);
    before_wait(&mut app);
    let cancel = *app
        .resource::<PulseAudio>()
        .pending_cancels
        .iter()
        .next()
        .unwrap();
    assert!(
        app.resource::<PulseAudio>()
            .pending_polls
            .contains_key(&poll)
    );

    let seen = dispatch_cancelled_poll(&mut app, poll);
    assert_eq!(seen.iter().find(|e| e.token == cancel).unwrap().result, 0);
    assert_eq!(
        seen.iter().find(|e| e.token == poll).unwrap().result,
        -libc::ECANCELED
    );
    assert_eq!(data.calls.get(), 0);
    assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETFD) }, -1);
    assert!(app.resource_mut::<Ring>().finish(cancel).is_none());
    assert!(app.resource_mut::<Ring>().finish(poll).is_none());
}

#[test]
fn disabling_a_watch_cancels_its_poll_only_once() {
    let (reader, _writer) = UnixStream::pair().unwrap();
    let data = Readiness::default();
    let mut app = polling_app();
    let (watch, poll) = arm_callback(
        &mut app,
        reader.as_raw_fd(),
        PA_IO_EVENT_INPUT,
        Some(record_readiness),
        &data,
    );

    app.resource::<PulseAudio>().mainloop_api.io_enable.unwrap()(watch, PA_IO_EVENT_NULL);
    before_wait(&mut app);
    let cancel = *app
        .resource::<PulseAudio>()
        .pending_cancels
        .iter()
        .next()
        .unwrap();
    before_wait(&mut app);
    assert_eq!(app.resource::<PulseAudio>().pending_cancels.len(), 1);
    assert!(
        app.resource::<PulseAudio>()
            .pending_cancels
            .contains(&cancel)
    );

    // Deliver the cancel CQE first, then run BeforeWait again while the
    // original poll is still tracked: it must not submit a second cancel.
    let deadline = app.resource_mut::<Ring>().timeout(Duration::from_secs(1));
    let mut completions = Vec::new();
    while completions.len() < 2 {
        for event in app.resource_mut::<Ring>().wait() {
            assert_ne!(event.token, deadline, "poll cancellation did not complete");
            completions.push(event);
        }
    }
    let cancel_event = *completions.iter().find(|e| e.token == cancel).unwrap();
    let poll_event = *completions.iter().find(|e| e.token == poll).unwrap();
    app.signal(cancel_event);
    app.flush();
    before_wait(&mut app);
    assert!(app.resource::<PulseAudio>().pending_cancels.is_empty());
    assert!(app.resource::<PulseAudio>().pending_polls[&poll].cancel_requested);

    app.signal(poll_event);
    app.flush();
    assert_eq!(data.calls.get(), 0);
    before_wait(&mut app);
    assert!(app.resource::<PulseAudio>().pending_polls.is_empty());
    assert!(app.resource::<PulseAudio>().pending_cancels.is_empty());
}

#[test]
fn changing_a_watch_cancels_the_old_poll_and_arms_the_new_generation() {
    let (reader, _writer) = UnixStream::pair().unwrap();
    let data = Readiness::default();
    let mut app = polling_app();
    let (watch, old) = arm_callback(
        &mut app,
        reader.as_raw_fd(),
        PA_IO_EVENT_INPUT,
        Some(record_readiness),
        &data,
    );

    app.resource::<PulseAudio>().mainloop_api.io_enable.unwrap()(
        watch,
        PA_IO_EVENT_INPUT | PA_IO_EVENT_HANGUP,
    );
    before_wait(&mut app);
    let new = *app
        .resource::<PulseAudio>()
        .pending_polls
        .keys()
        .find(|&&token| token != old)
        .unwrap();
    assert_eq!(
        app.resource::<PulseAudio>().pending_polls[&new].generation,
        1
    );

    dispatch_cancelled_poll(&mut app, old);
    assert_eq!(data.calls.get(), 0);
    assert!(
        app.resource::<PulseAudio>()
            .pending_polls
            .contains_key(&new)
    );
    assert!(!app.resource::<PulseAudio>().pending_polls[&new].cancel_requested);
}

#[test]
fn readiness_racing_with_removal_never_calls_the_freed_watch() {
    let (reader, mut writer) = UnixStream::pair().unwrap();
    let data = Readiness::default();
    let mut app = polling_app();
    let (watch, poll) = arm_callback(
        &mut app,
        reader.as_raw_fd(),
        PA_IO_EVENT_INPUT,
        Some(record_readiness),
        &data,
    );
    writer.write_all(b"x").unwrap();
    app.resource::<PulseAudio>().mainloop_api.io_free.unwrap()(watch);
    before_wait(&mut app);

    let seen = dispatch_cancelled_poll(&mut app, poll);
    assert!(seen.iter().any(|event| event.token == poll));
    assert_eq!(data.calls.get(), 0);
}

#[test]
fn already_completed_poll_and_enoent_cancel_are_finished_independently() {
    let (reader, mut writer) = UnixStream::pair().unwrap();
    let data = Readiness::default();
    let mut app = polling_app();
    let (watch, poll) = arm_callback(
        &mut app,
        reader.as_raw_fd(),
        PA_IO_EVENT_INPUT,
        Some(record_readiness),
        &data,
    );
    writer.write_all(b"x").unwrap();
    // Observe the poll CQE in Ring, but delay dispatch until after io_free.
    let ready = wait_for_poll(&mut app.resource_mut::<Ring>(), poll);
    assert_ne!(ready.result & libc::POLLIN as i32, 0);
    app.resource::<PulseAudio>().mainloop_api.io_free.unwrap()(watch);
    before_wait(&mut app);
    let cancel = *app
        .resource::<PulseAudio>()
        .pending_cancels
        .iter()
        .next()
        .unwrap();

    app.signal(ready);
    app.flush();
    assert!(app.resource::<PulseAudio>().pending_polls.is_empty());
    assert!(
        app.resource::<PulseAudio>()
            .pending_cancels
            .contains(&cancel)
    );

    let seen = dispatch_cancelled_poll(&mut app, poll);
    assert_eq!(
        seen.iter()
            .find(|event| event.token == cancel)
            .unwrap()
            .result,
        -libc::ENOENT
    );
    assert!(app.resource_mut::<Ring>().finish(cancel).is_none());
    assert_eq!(data.calls.get(), 0);
}

#[test]
fn on_io_delivers_readiness_and_allows_the_watch_to_be_rearmed() {
    let (mut reader, mut writer) = UnixStream::pair().unwrap();
    reader.set_nonblocking(true).unwrap();
    let data = Readiness::default();
    let mut app = polling_app();
    let (watch, token) = arm_callback(
        &mut app,
        reader.as_raw_fd(),
        PA_IO_EVENT_INPUT,
        Some(record_readiness),
        &data,
    );
    let duplicate = app.resource::<PulseAudio>().pending_polls[&token]
        .fd
        .as_raw_fd();

    writer.write_all(b"x").unwrap();
    dispatch_poll(&mut app, token);

    assert_eq!(data.calls.get(), 1);
    assert_eq!(data.event.get(), watch);
    assert_eq!(data.fd.get(), reader.as_raw_fd());
    assert_ne!(data.fd.get(), duplicate, "libpulse expects its original fd");
    assert_eq!(data.flags.get(), PA_IO_EVENT_INPUT);
    assert_eq!(
        data.api.get(),
        ptr::from_ref(app.resource::<PulseAudio>().mainloop_api.as_ref())
    );
    assert!(app.resource::<PulseAudio>().pending_polls.is_empty());
    assert!(app.resource_mut::<Ring>().finish(token).is_none());
    let mut byte = [0];
    reader.read_exact(&mut byte).unwrap();
    assert_eq!(byte, *b"x", "dispatch does not consume socket data");

    app.signal(BeforeWait);
    app.flush();
    let next = *app
        .resource::<PulseAudio>()
        .pending_polls
        .keys()
        .next()
        .unwrap();
    assert_ne!(next, token);
    writer.write_all(b"y").unwrap();
    dispatch_poll(&mut app, next);
    assert_eq!(data.calls.get(), 2);
    assert!(app.resource::<PulseAudio>().pending_polls.is_empty());
}

#[test]
fn on_io_ignores_an_obsolete_generation_then_dispatches_the_current_one() {
    let (reader, mut writer) = UnixStream::pair().unwrap();
    let data = Readiness::default();
    let mut app = polling_app();
    let (watch, old) = arm_callback(
        &mut app,
        reader.as_raw_fd(),
        PA_IO_EVENT_INPUT,
        Some(record_readiness),
        &data,
    );
    app.resource::<PulseAudio>().mainloop_api.io_enable.unwrap()(watch, PA_IO_EVENT_OUTPUT);

    writer.write_all(b"x").unwrap();
    dispatch_poll(&mut app, old);
    assert_eq!(data.calls.get(), 0);
    assert!(app.resource::<PulseAudio>().pending_polls.is_empty());
    assert!(app.resource_mut::<Ring>().finish(old).is_none());

    app.signal(BeforeWait);
    app.flush();
    let current = *app
        .resource::<PulseAudio>()
        .pending_polls
        .keys()
        .next()
        .unwrap();
    dispatch_poll(&mut app, current);
    assert_eq!(data.calls.get(), 1);
    assert_eq!(data.flags.get(), PA_IO_EVENT_OUTPUT);
}

#[test]
fn on_io_finishes_a_freed_watchs_poll_without_calling_it() {
    let (reader, mut writer) = UnixStream::pair().unwrap();
    let data = Readiness::default();
    let mut app = polling_app();
    let (watch, token) = arm_callback(
        &mut app,
        reader.as_raw_fd(),
        PA_IO_EVENT_INPUT,
        Some(record_readiness),
        &data,
    );
    app.resource::<PulseAudio>().mainloop_api.io_free.unwrap()(watch);
    drop(reader);

    writer.write_all(b"x").unwrap();
    dispatch_poll(&mut app, token);
    assert_eq!(data.calls.get(), 0);
    assert!(app.resource::<PulseAudio>().pending_polls.is_empty());
    assert!(app.resource_mut::<Ring>().finish(token).is_none());
}

#[test]
fn on_io_does_not_notify_a_disabled_watch() {
    let (reader, mut writer) = UnixStream::pair().unwrap();
    let data = Readiness::default();
    let mut app = polling_app();
    let (watch, token) = arm_callback(
        &mut app,
        reader.as_raw_fd(),
        PA_IO_EVENT_INPUT,
        Some(record_readiness),
        &data,
    );
    app.resource::<PulseAudio>().mainloop_api.io_enable.unwrap()(watch, PA_IO_EVENT_NULL);

    writer.write_all(b"x").unwrap();
    dispatch_poll(&mut app, token);
    app.signal(BeforeWait);
    app.flush();
    assert_eq!(data.calls.get(), 0);
    assert!(app.resource::<PulseAudio>().pending_polls.is_empty());
}

#[test]
fn on_io_leaves_other_modules_completions_unfinished() {
    let mut app = polling_app();
    let token = app.resource_mut::<Ring>().timeout(Duration::ZERO);
    let event = app.resource_mut::<Ring>().wait()[0];
    assert_eq!(event.token, token);

    app.signal(event);
    app.flush();

    assert!(app.resource_mut::<Ring>().finish(token).is_some());
}

extern "C" fn replace_on_readiness(
    api: *const pa_mainloop_api,
    event: *mut pa_io_event,
    fd: i32,
    flags: pa_io_event_flags_t,
    userdata: *mut c_void,
) {
    record_readiness(api, event, fd, flags, userdata);
    // SAFETY: the API is alive throughout dispatch. The Readiness data is owned
    // by the test, so freeing the watch does not free that data.
    let api = unsafe { &*api };
    api.io_free.unwrap()(event);
    let replacement =
        api.io_new.unwrap()(api, fd, PA_IO_EVENT_INPUT, Some(on_ready), ptr::null_mut());
    // SAFETY: the test's Readiness value is still alive after removing the watch.
    unsafe { &*userdata.cast::<Readiness>() }
        .replacement
        .set(replacement);
}

#[test]
fn readiness_callbacks_can_free_their_watch_and_register_a_replacement() {
    let (reader, mut writer) = UnixStream::pair().unwrap();
    let data = Readiness::default();
    let mut app = polling_app();
    let (_, token) = arm_callback(
        &mut app,
        reader.as_raw_fd(),
        PA_IO_EVENT_INPUT,
        Some(replace_on_readiness),
        &data,
    );
    let old_id = app.resource::<PulseAudio>().pending_polls[&token].watch_id;

    writer.write_all(b"x").unwrap();
    dispatch_poll(&mut app, token);

    assert_eq!(data.calls.get(), 1);
    assert!(!data.replacement.get().is_null());
    let audio = app.resource::<PulseAudio>();
    assert!(audio.pending_polls.is_empty());
    // SAFETY: the callback has returned, so the registry is not borrowed by FFI.
    let state = unsafe { &*audio.mainloop_state.get() };
    let registrations: Vec<_> = state.io_watches().collect();
    assert_eq!(registrations.len(), 1);
    assert_ne!(registrations[0].0, old_id);
}

#[test]
fn on_io_reports_negative_poll_results_to_libpulse_without_crashing() {
    let (reader, _writer) = UnixStream::pair().unwrap();
    let data = Readiness::default();
    let mut app = polling_app();
    let (_, token) = arm_callback(
        &mut app,
        reader.as_raw_fd(),
        PA_IO_EVENT_OUTPUT,
        Some(record_readiness),
        &data,
    );
    let mut event = wait_for_poll(&mut app.resource_mut::<Ring>(), token);
    // Inject an operation failure after obtaining a real, finishable completion.
    event.result = -libc::EIO;
    app.signal(event);
    app.flush();
    assert_eq!(data.calls.get(), 1);
    assert_eq!(data.flags.get(), PA_IO_EVENT_ERROR);
}

#[test]
fn on_io_forwards_socket_hangups_to_libpulse() {
    let (reader, writer) = UnixStream::pair().unwrap();
    let data = Readiness::default();
    let mut app = polling_app();
    let (_, token) = arm_callback(
        &mut app,
        reader.as_raw_fd(),
        PA_IO_EVENT_INPUT,
        Some(record_readiness),
        &data,
    );

    drop(writer);
    dispatch_poll(&mut app, token);

    assert_eq!(data.calls.get(), 1);
    assert_ne!(data.flags.get() & PA_IO_EVENT_HANGUP, 0);
}

#[derive(Default)]
struct DeferredData {
    calls: Cell<usize>,
    other: Cell<*mut pa_defer_event>,
    replacement: Cell<*mut pa_defer_event>,
}

extern "C" fn record_deferred(
    _: *const pa_mainloop_api,
    _: *mut pa_defer_event,
    userdata: *mut c_void,
) {
    // SAFETY: the test keeps DeferredData alive throughout dispatch.
    let data = unsafe { &*userdata.cast::<DeferredData>() };
    data.calls.set(data.calls.get() + 1);
}

extern "C" fn disable_deferred(
    api: *const pa_mainloop_api,
    event: *mut pa_defer_event,
    userdata: *mut c_void,
) {
    record_deferred(api, event, userdata);
    // SAFETY: the API is live throughout the callback.
    unsafe { &*api }.defer_enable.unwrap()(event, 0);
}

fn add_deferred(
    app: &App,
    callback: pa_defer_event_cb_t,
    data: &DeferredData,
) -> *mut pa_defer_event {
    let audio = app.resource::<PulseAudio>();
    audio.mainloop_api.defer_new.unwrap()(
        audio.mainloop_api.as_ref(),
        callback,
        ptr::from_ref(data).cast_mut().cast(),
    )
}

fn before_wait(app: &mut App) {
    app.signal(BeforeWait);
    app.flush();
}

fn dispatch_deferred_wake(app: &mut App, token: Token) {
    // These tests queue only the immediate wake, with no socket operations.
    let event = app.resource_mut::<Ring>().wait()[0];
    assert_eq!(event.token, token);
    assert_eq!(event.result, -libc::ETIME);
    app.signal(event);
    app.flush();
}

#[test]
fn before_wait_runs_deferred_callbacks_that_can_disable_themselves() {
    let data = DeferredData::default();
    let mut app = polling_app();
    let event = add_deferred(&app, Some(disable_deferred), &data);
    assert_eq!(data.calls.get(), 0, "registration does not invoke work");

    before_wait(&mut app);
    assert_eq!(data.calls.get(), 1);
    assert!(app.resource::<PulseAudio>().defer_wake.is_none());
    before_wait(&mut app);
    assert_eq!(data.calls.get(), 1);

    app.resource::<PulseAudio>()
        .mainloop_api
        .defer_enable
        .unwrap()(event, 1);
    before_wait(&mut app);
    assert_eq!(data.calls.get(), 2);
    assert!(app.resource::<PulseAudio>().defer_wake.is_none());
}

#[test]
fn enabled_deferred_work_runs_one_pass_and_queues_only_one_immediate_wake() {
    let data = DeferredData::default();
    let mut app = polling_app();
    let event = add_deferred(&app, Some(record_deferred), &data);

    before_wait(&mut app);
    assert_eq!(data.calls.get(), 1, "do not loop on an enabled callback");
    let wake = app.resource::<PulseAudio>().defer_wake.unwrap();
    before_wait(&mut app);
    assert_eq!(data.calls.get(), 2);
    assert_eq!(app.resource::<PulseAudio>().defer_wake, Some(wake));

    dispatch_deferred_wake(&mut app, wake);
    assert!(app.resource::<PulseAudio>().defer_wake.is_none());
    assert_eq!(data.calls.get(), 2, "the wake itself does not invoke work");
    before_wait(&mut app);
    let next = app.resource::<PulseAudio>().defer_wake.unwrap();
    assert_ne!(next, wake);
    assert_eq!(data.calls.get(), 3);

    app.resource::<PulseAudio>()
        .mainloop_api
        .defer_enable
        .unwrap()(event, 0);
    dispatch_deferred_wake(&mut app, next);
    before_wait(&mut app);
    assert_eq!(data.calls.get(), 3);
    assert!(app.resource::<PulseAudio>().defer_wake.is_none());
}

extern "C" fn replace_deferred(
    api: *const pa_mainloop_api,
    event: *mut pa_defer_event,
    userdata: *mut c_void,
) {
    record_deferred(api, event, userdata);
    // SAFETY: the API is live and the test owns userdata independently of the
    // removed registration. No state borrow is held by the dispatcher.
    let api = unsafe { &*api };
    api.defer_free.unwrap()(event);
    let replacement = api.defer_new.unwrap()(api, Some(disable_deferred), userdata);
    // SAFETY: DeferredData remains alive after the registration is freed.
    unsafe { &*userdata.cast::<DeferredData>() }
        .replacement
        .set(replacement);
}

#[test]
fn deferred_callbacks_can_replace_themselves_with_work_for_the_next_turn() {
    let data = DeferredData::default();
    let mut app = polling_app();
    add_deferred(&app, Some(replace_deferred), &data);

    before_wait(&mut app);
    assert_eq!(data.calls.get(), 1, "new work waits for the next pass");
    assert!(!data.replacement.get().is_null());
    let wake = app.resource::<PulseAudio>().defer_wake.unwrap();

    before_wait(&mut app);
    assert_eq!(data.calls.get(), 2);
    dispatch_deferred_wake(&mut app, wake);
    before_wait(&mut app);
    assert_eq!(data.calls.get(), 2);
    assert!(app.resource::<PulseAudio>().defer_wake.is_none());
}

extern "C" fn free_other_deferred(
    api: *const pa_mainloop_api,
    event: *mut pa_defer_event,
    userdata: *mut c_void,
) {
    record_deferred(api, event, userdata);
    // SAFETY: these test-owned callback arguments and API remain live.
    let data = unsafe { &*userdata.cast::<DeferredData>() };
    let api = unsafe { &*api };
    let other = data.other.replace(ptr::null_mut());
    if !other.is_null() {
        api.defer_free.unwrap()(other);
    }
    api.defer_enable.unwrap()(event, 0);
}

#[test]
fn deferred_entries_removed_by_an_earlier_callback_are_not_invoked() {
    let first = DeferredData::default();
    let second = DeferredData::default();
    let mut app = polling_app();
    add_deferred(&app, Some(free_other_deferred), &first);
    let other = add_deferred(&app, Some(disable_deferred), &second);
    first.other.set(other);

    before_wait(&mut app);

    assert_eq!(first.calls.get(), 1);
    assert_eq!(second.calls.get(), 0);
    assert!(app.resource::<PulseAudio>().defer_wake.is_none());
}

struct DeferredIo {
    fd: RawFd,
    watch: Cell<*mut pa_io_event>,
}

extern "C" fn create_io_from_deferred(
    api: *const pa_mainloop_api,
    event: *mut pa_defer_event,
    userdata: *mut c_void,
) {
    // SAFETY: the test keeps DeferredIo and its descriptor alive for dispatch.
    let data = unsafe { &*userdata.cast::<DeferredIo>() };
    let api = unsafe { &*api };
    let watch = api.io_new.unwrap()(
        api,
        data.fd,
        PA_IO_EVENT_INPUT,
        Some(on_ready),
        ptr::null_mut(),
    );
    data.watch.set(watch);
    api.defer_enable.unwrap()(event, 0);
}

#[test]
fn io_watches_created_by_deferred_work_are_armed_before_waiting() {
    let (reader, mut writer) = UnixStream::pair().unwrap();
    let data = DeferredIo {
        fd: reader.as_raw_fd(),
        watch: Cell::new(ptr::null_mut()),
    };
    let mut app = polling_app();
    let audio = app.resource::<PulseAudio>();
    audio.mainloop_api.defer_new.unwrap()(
        audio.mainloop_api.as_ref(),
        Some(create_io_from_deferred),
        ptr::from_ref(&data).cast_mut().cast(),
    );

    before_wait(&mut app);

    assert!(!data.watch.get().is_null());
    let audio = app.resource::<PulseAudio>();
    assert!(audio.defer_wake.is_none());
    assert_eq!(audio.pending_polls.len(), 1);
    let token = *audio.pending_polls.keys().next().unwrap();
    writer.write_all(b"x").unwrap();
    dispatch_poll(&mut app, token);
    assert!(app.resource::<PulseAudio>().pending_polls.is_empty());
}
