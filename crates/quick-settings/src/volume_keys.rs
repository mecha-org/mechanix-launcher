//! The hardware volume keys, grabbed through `phosh_private_keyboard_event`.
//!
//! The compositor intercepts a grabbed key and sends one
//! `accelerator_activated_event` per press, and again for each step of its
//! own hold-to-repeat, so each activation is one step here and no repeat
//! timer runs in the launcher.

use mecha_pulseaudio::{SetVolume, VolumeState};
use mecha_wayland::prelude::*;

const RAISE: &str = "XF86AudioRaiseVolume";
const LOWER: &str = "XF86AudioLowerVolume";
/// Percent per key press.
const STEP: i32 = 5;
/// `phosh_private.get_keyboard_event` is `since="5"`.
const MIN_VERSION: u32 = 5;
/// A step within this many ms of the previous one continues from the
/// volume that step asked for, since PulseAudio may not have reported it
/// yet; after a longer pause the reported volume is the base again.
pub(crate) const PENDING_WINDOW_MS: u32 = 500;

/// Grabs the volume keys and steps the volume with [`SetVolume`]. Install
/// after `WaylandModule`. It binds `phosh_private` itself, independently of
/// `ToplevelsModule`'s bind. Without the global it does nothing.
pub struct VolumeKeysModule;

impl Module for VolumeKeysModule {
    fn install(self, app: &mut App) {
        let Some(global) = app
            .resource::<Globals>()
            .find(PhoshPrivate::NAME)
            .filter(|g| g.version >= MIN_VERSION)
            .cloned()
        else {
            return;
        };
        let keyboard = {
            let (globals, mut wl) = app.query::<(Res<Globals>, ResMut<Wayland>)>();
            let phosh = globals.bind::<PhoshPrivate>(&global, &mut wl);
            let keyboard = phosh.get_keyboard_event(&mut wl);
            keyboard.grab_accelerator_request(&mut wl, RAISE);
            keyboard.grab_accelerator_request(&mut wl, LOWER);
            keyboard
        };
        app.init_resource::<VolumeState>()
            .insert_resource(VolumeKeys {
                keyboard,
                actions: Vec::new(),
                stepper: Stepper::default(),
            });
        app.system(on_keyboard_event).system(on_volume_changed);
    }
}

struct VolumeKeys {
    keyboard: PhoshPrivateKeyboardEvent,
    /// Granted action ids and the step each one makes.
    actions: Vec<(u32, i32)>,
    stepper: Stepper,
}
impl Resource for VolumeKeys {}

fn on_keyboard_event(app: &mut App, e: &PhoshPrivateKeyboardEventEvent) {
    use PhoshPrivateKeyboardEventEvent as E;
    let state = *app.resource::<VolumeState>();
    let mut keys = app.resource_mut::<VolumeKeys>();
    let set = match e {
        E::GrabSuccessEvent {
            phosh_private_keyboard_event: k,
            accelerator,
            action_id,
        } if *k == keys.keyboard => {
            let delta = match accelerator.as_str() {
                RAISE => STEP,
                LOWER => -STEP,
                _ => return,
            };
            keys.actions.retain(|&(id, _)| id != *action_id);
            keys.actions.push((*action_id, delta));
            None
        }
        E::GrabFailedEvent {
            phosh_private_keyboard_event: k,
            accelerator,
            error,
        } if *k == keys.keyboard => {
            tracing::warn!("volume keys: grabbing {accelerator} failed (error {error})");
            None
        }
        E::AcceleratorActivatedEvent {
            phosh_private_keyboard_event: k,
            action_id,
            timestamp,
        } if *k == keys.keyboard => {
            let delta = keys
                .actions
                .iter()
                .find(|&&(id, _)| id == *action_id)
                .map(|&(_, d)| d);
            delta.and_then(|d| keys.stepper.step(state, d, *timestamp))
        }
        _ => None,
    };
    drop(keys);
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
