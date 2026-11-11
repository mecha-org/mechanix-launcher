//! The screen's power: every output on or off, through
//! wlr-output-power-management.
//!
//! - Each output gets a power control as it appears. The compositor reports
//!   its mode at once and on every change, whoever made it, so the screen is
//!   off exactly when every output says so.
//! - Going off brings the lock screen back to its glance screen, for when
//!   the screen is next on. Each change goes out as [`ScreenPowered`].
//! - With every output off, the compositor delivers no input and counts
//!   none as activity, so nothing but the power key ([`crate::power`])
//!   turns them back on.
//! - Waking from sleep turns them on: the key press that wakes the system
//!   never reaches userspace.
//!
//! Without the global, the screen is always on.

use mecha_wayland::prelude::*;
use services::PrepareForSleep;

/// The screen went on (`true`) or off.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ScreenPowered(pub bool);
impl Signal for ScreenPowered {}

struct Screen {
    power: Option<ZwlrOutputPowerManagerV1>,
    controls: Vec<Control>,
    off: bool,
}
impl Resource for Screen {}

struct Control {
    output: WlOutput,
    power: ZwlrOutputPowerV1,
    /// The output's mode, once the compositor said.
    mode: Option<ZwlrOutputPowerV1Mode>,
}

pub(crate) fn install(app: &mut App) {
    let power = {
        let (globals, mut wl) = app.query::<(Res<Globals>, ResMut<Wayland>)>();
        globals.bind_if_offered::<ZwlrOutputPowerManagerV1>(&mut wl, 1)
    };
    if power.is_none() {
        tracing::warn!("no zwlr_output_power_manager_v1: the screen never turns off");
    }
    app.insert_resource(Screen {
        power,
        controls: Vec::new(),
        off: false,
    });
    let outputs: Vec<WlOutput> = app.resource::<Outputs>().iter().collect();
    for o in outputs {
        add(app, o);
    }
    app.system(|app, e: &OutputAdded| add(app, e.0))
        .system(on_output_removed)
        .system(on_power)
        .system(|app, e: &PrepareForSleep| {
            if !e.0 {
                turn_on(app);
            }
        });
}

/// Whether every output is off.
pub(crate) fn is_off(app: &App) -> bool {
    app.resource::<Screen>().off
}

/// Turn every output off. False when the compositor has no power control.
pub(crate) fn turn_off(app: &mut App) -> bool {
    set(app, ZwlrOutputPowerV1Mode::Off)
}

/// Turn every output on.
pub(crate) fn turn_on(app: &mut App) {
    set(app, ZwlrOutputPowerV1Mode::On);
}

fn set(app: &mut App, mode: ZwlrOutputPowerV1Mode) -> bool {
    // A control the compositor gave up on is made again.
    let missing: Vec<WlOutput> = {
        let screen = app.resource::<Screen>();
        app.resource::<Outputs>()
            .iter()
            .filter(|o| !screen.controls.iter().any(|c| c.output == *o))
            .collect()
    };
    for o in missing {
        add(app, o);
    }
    let (screen, mut wl) = app.query::<(Res<Screen>, ResMut<Wayland>)>();
    if screen.power.is_none() {
        return false;
    }
    for c in &screen.controls {
        c.power.set_mode(&mut wl, mode);
    }
    true
}

fn add(app: &mut App, output: WlOutput) {
    let (mut screen, mut wl) = app.query::<(ResMut<Screen>, ResMut<Wayland>)>();
    let Some(manager) = screen.power else {
        return;
    };
    let power = manager.get_output_power(&mut wl, output);
    screen.controls.push(Control {
        output,
        power,
        mode: None,
    });
}

fn on_output_removed(app: &mut App, e: &OutputRemoved) {
    {
        let (mut screen, mut wl) = app.query::<(ResMut<Screen>, ResMut<Wayland>)>();
        if let Some(i) = screen.controls.iter().position(|c| c.output == e.0) {
            screen.controls.remove(i).power.destroy(&mut wl);
        }
    }
    update(app);
}

fn on_power(app: &mut App, e: &ZwlrOutputPowerV1Event) {
    {
        let (mut screen, mut wl) = app.query::<(ResMut<Screen>, ResMut<Wayland>)>();
        match e {
            ZwlrOutputPowerV1Event::Mode { output_power, mode } => {
                if let Some(c) = screen
                    .controls
                    .iter_mut()
                    .find(|c| c.power == *output_power)
                {
                    c.mode = Some(*mode);
                }
            }
            // Made again by the next `set`.
            ZwlrOutputPowerV1Event::Failed { output_power } => {
                if let Some(i) = screen
                    .controls
                    .iter()
                    .position(|c| c.power == *output_power)
                {
                    screen.controls.remove(i).power.destroy(&mut wl);
                }
            }
        }
    }
    update(app);
}

/// Work out whether the screen is off, and tell if that changed.
fn update(app: &mut App) {
    let off = {
        let mut screen = app.resource_mut::<Screen>();
        let off = !screen.controls.is_empty()
            && screen
                .controls
                .iter()
                .all(|c| c.mode == Some(ZwlrOutputPowerV1Mode::Off));
        if off == screen.off {
            return;
        }
        screen.off = off;
        off
    };
    if off {
        crate::lock::collapse(app);
    }
    app.signal(ScreenPowered(!off));
}
