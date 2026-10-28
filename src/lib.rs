use mecha_wayland::prelude::*;

mod mainloop;

struct PulseAudioModule;

impl Module for PulseAudioModule {
    fn install(self, app: &mut App) {}
}
