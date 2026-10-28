//! Print the current default output state, then exit.
//! Run with `cargo run --example volume_status` under PulseAudio/pipewire-pulse.

use mecha_pulseaudio::{
    ConnectionState, ConnectionStateChanged, PulseAudioModule, VolumeControlModule, VolumeState,
};
use mecha_wayland::prelude::*;

fn on_volume(app: &mut App, _: &OnChanged<VolumeState>) {
    match app.resource::<VolumeState>() {
        VolumeState::Loading => return,
        VolumeState::Unavailable => println!("Default output unavailable"),
        VolumeState::Ready {
            volume_percent,
            muted,
        } => println!("Default output: {volume_percent}%, muted: {muted}"),
    }
}

fn on_connection(app: &mut App, event: &ConnectionStateChanged) {
    if let ConnectionState::Failed(error) = &event.0 {
        eprintln!("PulseAudio connection failed: {error}");
        app.signal(Stop);
    }
}

fn main() {
    let mut app = App::new();
    app.add_module(RingModule::default())
        .add_module(PulseAudioModule)
        .add_module(VolumeControlModule)
        .system(on_connection)
        .system(on_volume);
    app.run();
}
