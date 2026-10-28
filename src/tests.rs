use std::ffi::c_void;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::ptr;
use std::time::Duration;

use super::*;

const FLAGS: [(pa_io_event_flags_t, u32); 4] = [
    (PA_IO_EVENT_INPUT, libc::POLLIN as u32),
    (PA_IO_EVENT_OUTPUT, libc::POLLOUT as u32),
    (PA_IO_EVENT_HANGUP, libc::POLLHUP as u32),
    (PA_IO_EVENT_ERROR, libc::POLLERR as u32),
];

#[test]
fn module_initializes_the_pulse_audio_resource() {
    let mut app = App::new();
    app.add_module(PulseAudioModule);

    let audio = app.resource::<PulseAudio>();
    assert_eq!(
        audio.mainloop_api.userdata,
        audio.mainloop_state.get().cast()
    );
    assert!(audio.pending_polls.is_empty());
    assert!(audio.mainloop_api.io_new.is_some());
    assert!(audio.mainloop_api.io_enable.is_some());
    assert!(audio.mainloop_api.io_free.is_some());
    assert!(audio.mainloop_api.io_set_destroy.is_some());
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
fn installing_the_module_again_preserves_the_existing_resource() {
    let mut app = App::new();
    app.add_module(PulseAudioModule);
    let api = std::ptr::from_ref(app.resource::<PulseAudio>().mainloop_api.as_ref());

    app.add_module(PulseAudioModule);

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
// Production completion dispatch will later finish and remove these polls.
fn finish_poll(ring: &mut Ring, token: Token) -> IoEvent {
    let deadline = ring.timeout(Duration::from_secs(1));
    loop {
        for event in ring.wait() {
            let completed = ring.finish(event.token).expect("operation completed");
            if event.token == token {
                assert!(event.result >= 0, "poll failed: {event:?}");
                assert!(completed.buf.is_empty() && completed.fds.is_empty());
                return event;
            }
            assert_ne!(event.token, deadline, "poll did not complete");
        }
    }
}

#[test]
fn before_wait_arms_readable_watches_without_duplicate_polls() {
    let (mut reader, mut writer) = UnixStream::pair().unwrap();
    reader.set_nonblocking(true).unwrap();
    let mut app = App::new();
    app.add_module(RingModule::default())
        .add_module(PulseAudioModule);
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
    audio.arm_polls(&mut ring).unwrap();
    assert_eq!(
        audio.pending_polls.len(),
        1,
        "old polls are retained for now"
    );
}

#[test]
fn changed_interests_arm_a_new_poll_without_discarding_the_old_one() {
    let (reader, _writer) = UnixStream::pair().unwrap();
    let mut audio = PulseAudio::new();
    let mut ring = Ring::new(8);
    let event = add_watch(&audio, reader.as_raw_fd(), PA_IO_EVENT_INPUT);

    audio.arm_polls(&mut ring).unwrap();
    let (&old_token, old_poll) = audio.pending_polls.iter().next().unwrap();
    let watch_id = old_poll.watch_id;

    audio.mainloop_api.io_enable.unwrap()(event, PA_IO_EVENT_OUTPUT);
    audio.arm_polls(&mut ring).unwrap();
    audio.arm_polls(&mut ring).unwrap();
    assert_eq!(audio.pending_polls.len(), 2);
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
