use std::os::fd::OwnedFd;

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
