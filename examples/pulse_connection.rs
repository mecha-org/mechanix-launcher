//! Headless connection check: `cargo run --example pulse_connection`.
//! Requires a running PulseAudio or pipewire-pulse server in this user session.
//! This stops after the first outcome; the library itself never stops the App.

use mecha_pulseaudio::{ConnectionState, ConnectionStateChanged, PulseAudioModule};
use mecha_wayland::prelude::*;

fn on_connection(app: &mut App, event: &ConnectionStateChanged) {
    match &event.0 {
        ConnectionState::Ready => {
            println!("PulseAudio connection ready");
            app.signal(Stop);
        }
        ConnectionState::Failed(error) => {
            eprintln!("PulseAudio connection failed: {error}");
            app.signal(Stop);
        }
        ConnectionState::Terminated => {
            eprintln!("PulseAudio connection terminated");
            app.signal(Stop);
        }
        ConnectionState::Connecting => {}
    }
}

fn main() {
    let mut app = App::new();
    // Ring owns the only runner; the PulseAudio module uses its completions.
    app.add_module(RingModule::default())
        .add_module(PulseAudioModule)
        .system(on_connection);
    app.run();
}
