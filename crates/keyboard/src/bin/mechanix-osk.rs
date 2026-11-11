//! `mechanix-osk` — the on-screen keyboard as its own process, for shells and
//! compositors other than the Mechanix launcher.
//!
//! Hosts [`KeyboardModule`] on a bare app: no nav, so nothing calls
//! `show_keyboard`; the keyboard shows and hides as text inputs activate and
//! deactivate `zwp_input_method_v2`. Needs a compositor with
//! `zwlr_layer_shell_v1`, `zwp_input_method_v2` and
//! `zwp_virtual_keyboard_v1`.
//!
//! Build with `cargo build -p keyboard --features standalone --release`.

use keyboard::KeyboardModule;
use mecha_wayland::prelude::*;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let mut app = App::new();
    app.add_module(LayoutModule)
        .add_module(PaintModule)
        .add_module(WindowModule)
        .add_module(InteractivityModule)
        .add_module(AnimationModule)
        .add_module(RenderModule::default())
        .insert_resource(Atlas::new());

    app.add_module(MechanixTheme::dark());

    app.add_module(RingModule::default())
        .add_module(TimerModule)
        // The session bus, for feedbackd haptics; the keyboard skips them
        // when there is none.
        .add_module(DBusModule::new().session())
        .add_module(
            WaylandModule::new()
                .bind::<WlCompositor>()
                .bind::<ZwpLinuxDmabufV1>()
                // Presentation requires it, though the keyboard is a layer surface.
                .bind::<XdgWmBase>()
                .bind::<WlSeat>()
                .bind::<ZwlrLayerShellV1>(),
        )
        .add_module(PresentationModule {
            app_id: "mechanix.osk".into(),
            budget: Budget::default(),
        })
        // After presentation, which registers the `Role` its surface carries.
        // No host swipe strip under the keys.
        .add_module(KeyboardModule { bottom_gap: 0.0 });

    app.run();
}
