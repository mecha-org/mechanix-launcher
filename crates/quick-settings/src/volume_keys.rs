//! The hardware volume keys, grabbed through [`services::keys`].
//!
//! The compositor intercepts a grabbed key and reports one press, and
//! again for each step of its own hold-to-repeat, so each press is one
//! step here and no repeat timer runs in the launcher.

use mecha_pulseaudio::{SetVolume, VolumeState};
use mecha_wayland::prelude::*;
use services::{Key, KeyAction, grab_key};

const RAISE: &str = "XF86AudioRaiseVolume";
const LOWER: &str = "XF86AudioLowerVolume";
/// Percent per key press.
const STEP: i32 = 5;
/// A step within this many ms of the previous one continues from the
/// volume that step asked for, since PulseAudio may not have reported it
/// yet; after a longer pause the reported volume is the base again.
pub(crate) const PENDING_WINDOW_MS: u32 = 500;

/// Grabs the volume keys and steps the volume with [`SetVolume`]. Install
/// after `WaylandModule`. Without `phosh_private` it does nothing.
pub struct VolumeKeysModule;

impl Module for VolumeKeysModule {
    fn install(self, app: &mut App) {
        if !(grab_key(app, RAISE) && grab_key(app, LOWER)) {
            return;
        }
        app.init_resource::<VolumeState>()
            .insert_resource(VolumeKeys {
                stepper: Stepper::default(),
            });
        app.system(on_key).system(on_volume_changed);
    }
}

struct VolumeKeys {
    stepper: Stepper,
}
impl Resource for VolumeKeys {}

fn on_key(app: &mut App, e: &Key) {
    let delta = match e.accelerator {
        RAISE => STEP,
        LOWER => -STEP,
        _ => return,
    };
    let timestamp = match e.action {
        KeyAction::Pressed { timestamp } => timestamp,
        KeyAction::Failed(error) => {
            tracing::warn!(
                "volume keys: grabbing {} failed (error {error})",
                e.accelerator
            );
            return;
        }
        _ => return,
    };
    let state = *app.resource::<VolumeState>();
    let set = app
        .resource_mut::<VolumeKeys>()
        .stepper
        .step(state, delta, timestamp);
    if let Some(v) = set {
        app.signal(SetVolume(v));
    }
}

fn on_volume_changed(app: &mut App, _: &OnChanged<VolumeState>) {
    let state = *app.resource::<VolumeState>();
    app.resource_mut::<VolumeKeys>().stepper.observe(state);
}

/// Steps from the volume last asked for while presses come close
/// together, so presses faster than PulseAudio's round-trip each count;
/// after a pause of more than [`PENDING_WINDOW_MS`] it steps from the
/// reported volume, so a slider drag or another tool is picked up.
#[derive(Default)]
pub(crate) struct Stepper {
    /// The volume last asked for and the timestamp (ms) of that step.
    target: Option<(u32, u32)>,
}

impl Stepper {
    /// The volume after stepping by `delta` at `at` (the event's ms
    /// timestamp, wrapping), 0..=100. The base is the pending target if
    /// the previous step was at most [`PENDING_WINDOW_MS`] away (either
    /// way, as timestamps can arrive slightly out of order), else the
    /// reported volume clamped to 100; `None` with neither. A raise never
    /// lowers: from a reported volume above 100 it gives `None`.
    pub(crate) fn step(&mut self, state: VolumeState, delta: i32, at: u32) -> Option<u32> {
        let pending = self
            .target
            .filter(|&(_, last)| (at.wrapping_sub(last) as i32).unsigned_abs() <= PENDING_WINDOW_MS)
            .map(|(v, _)| v);
        let base = match (pending, state) {
            (Some(t), _) => t,
            (None, VolumeState::Ready { volume_percent, .. }) => {
                if delta > 0 && volume_percent > 100 {
                    // Clamping to 100 would lower a volume set above it.
                    return None;
                }
                volume_percent.min(100)
            }
            (None, _) => return None,
        };
        let v = (base as i32 + delta).clamp(0, 100) as u32;
        self.target = Some((v, at));
        Some(v)
    }

    /// Drops the target when PulseAudio goes away, so a press after it
    /// comes back steps from its report even within the window.
    pub(crate) fn observe(&mut self, state: VolumeState) {
        if state == VolumeState::Unavailable {
            self.target = None;
        }
    }
}

#[cfg(test)]
mod tests;
