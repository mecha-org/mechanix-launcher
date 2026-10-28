//! Volume-control service built on the shared PulseAudio connection.

use mecha_wayland::prelude::{App, Module};

/// Install after [`PulseAudioModule`](crate::PulseAudioModule).
/// Volume state, commands, and change signals will be added here.
pub struct VolumeControlModule;

impl Module for VolumeControlModule {
    fn install(self, _app: &mut App) {}
}
