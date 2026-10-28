use super::*;

const FLAGS: [(pa_io_event_flags_t, u32); 4] = [
    (PA_IO_EVENT_INPUT, libc::POLLIN as u32),
    (PA_IO_EVENT_OUTPUT, libc::POLLOUT as u32),
    (PA_IO_EVENT_HANGUP, libc::POLLHUP as u32),
    (PA_IO_EVENT_ERROR, libc::POLLERR as u32),
];

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
