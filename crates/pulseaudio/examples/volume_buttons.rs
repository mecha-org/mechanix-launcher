//! Four buttons for trying the default-output volume control.
//! Run under Wayland with PulseAudio or pipewire-pulse:
//! `cargo run --example volume_buttons`.
//! Clicking a button changes your actual output volume.

use mecha_pulseaudio::{PulseAudioModule, SetVolume, VolumeControlModule};
use mecha_wayland::mechanix_widgets::prelude::{FontBook, button};
use mecha_wayland::prelude::*;

struct Shell;
struct ShellBuilder {
    root: NodeId,
}

impl Build for ShellBuilder {
    type Widget = Shell;
}

impl Widget for Shell {
    type Builder = ShellBuilder;

    fn build(builder: ShellBuilder, _: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        let win = s.spawn(
            builder.root,
            window()
                .title("Volume presets")
                .layout(LayoutStyle::default().center().size(px(440.0), px(160.0))),
        );
        s.on::<CloseRequested>(win, |ctx, _| ctx.signal(Stop));

        let row = s.spawn(
            win,
            div().style(LayoutStyle::default().row().center().fill().gap(px(12.0))),
        );

        for percent in [25, 50, 75, 100] {
            let preset = s.spawn(row, button().label(format!("{percent}%")));
            s.on::<Clicked>(preset, move |ctx, _| ctx.signal(SetVolume(percent)));
        }
        Shell
    }
}

fn main() {
    let mut app = App::new();
    app.add_module(LayoutModule)
        .add_module(PaintModule)
        .add_module(WindowModule)
        .add_module(InteractivityModule)
        .add_module(AnimationModule)
        .add_module(RenderModule::default())
        .insert_resource(Atlas::new());
    app.add_module(MechanixTheme::dark());

    let font = app
        .resource_mut::<Atlas>()
        .add_font(include_bytes!(
            "../../mecha-wayland/crates/atlas/tests/fixtures/Inter-Regular.ttf"
        ))
        .expect("example font loads");
    app.insert_resource(FontBook::new(font));

    app.add_module(RingModule::default())
        .add_module(PulseAudioModule)
        .add_module(VolumeControlModule)
        .add_module(
            WaylandModule::new()
                .bind::<WlCompositor>()
                .bind::<ZwpLinuxDmabufV1>()
                .bind::<XdgWmBase>()
                .bind::<WlSeat>(),
        )
        .add_module(PresentationModule {
            app_id: "mecha.volume_buttons".into(),
            budget: Budget::default(),
        });

    let root = app.root();
    app.spawn(root, ShellBuilder { root });
    app.run();
}
