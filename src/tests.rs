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
