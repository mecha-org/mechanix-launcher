use mecha_pulseaudio::{SetVolume, VolumeState};
use mecha_wayland::prelude::*;
use services::{Key, KeyAction};
use wayland::fake::Fake;

use super::{LOWER, PENDING_WINDOW_MS, RAISE, Stepper, VolumeKeysModule};

fn ready(volume_percent: u32) -> VolumeState {
    VolumeState::Ready {
        volume_percent,
        muted: false,
    }
}

// ── Stepper ──────────────────────────────────────────────────────────────

#[test]
fn stepper_counts_rapid_presses() {
    let mut s = Stepper::default();
    // PulseAudio has not caught up: every press still reports 50.
    let steps: Vec<_> = (0..5)
        .map(|i| s.step(ready(50), 5, 1000 + i * 120))
        .collect();
    assert_eq!(steps, [Some(55), Some(60), Some(65), Some(70), Some(75)]);
}

#[test]
fn stepper_clamps() {
    assert_eq!(Stepper::default().step(ready(98), 5, 0), Some(100));
    assert_eq!(Stepper::default().step(ready(3), -5, 0), Some(0));
    assert_eq!(Stepper::default().step(ready(100), 5, 0), Some(100));
    // A raise never lowers a volume set above 100 by another tool.
    assert_eq!(Stepper::default().step(ready(150), 5, 0), None);
    assert_eq!(Stepper::default().step(ready(150), -5, 0), Some(95));
}

#[test]
fn stepper_raise_above_100_keeps_no_target() {
    let mut s = Stepper::default();
    assert_eq!(s.step(ready(150), 5, 0), None);
    // No target was recorded: a lower step right after still starts from
    // the report.
    assert_eq!(s.step(ready(150), -5, 100), Some(95));
}

#[test]
fn stepper_window_allows_slightly_older_timestamps() {
    let mut s = Stepper::default();
    assert_eq!(s.step(ready(50), 5, 1000), Some(55));
    // A few ms earlier than the last step: still close, so still pending.
    assert_eq!(s.step(ready(50), 5, 995), Some(60));
    // Older than the window: the report is the base again.
    let mut s = Stepper::default();
    assert_eq!(s.step(ready(50), 5, 1000), Some(55));
    assert_eq!(s.step(ready(50), 5, 1000 - PENDING_WINDOW_MS - 1), Some(55));
}

#[test]
fn stepper_window_allows_slightly_older_timestamps_across_the_wrap() {
    let mut s = Stepper::default();
    assert_eq!(s.step(ready(50), 5, 3), Some(55));
    assert_eq!(s.step(ready(50), 5, u32::MAX - 2), Some(60));
}

#[test]
fn stepper_needs_a_volume() {
    assert_eq!(Stepper::default().step(VolumeState::Loading, 5, 0), None);
    assert_eq!(
        Stepper::default().step(VolumeState::Unavailable, -5, 0),
        None
    );
}

#[test]
fn stepper_resyncs_after_pause() {
    let mut s = Stepper::default();
    for i in 0..5 {
        s.step(ready(50), 5, i * 120);
    }
    // The slider moved the volume to 30 meanwhile; the target 75 is stale.
    assert_eq!(s.step(ready(30), 5, 4 * 120 + 600), Some(35));
}

#[test]
fn stepper_keeps_target_within_window() {
    let mut s = Stepper::default();
    assert_eq!(s.step(ready(50), 5, 0), Some(55));
    // Exactly the window after the last step still counts as pending.
    assert_eq!(s.step(ready(50), 5, PENDING_WINDOW_MS), Some(60));
    // The window runs from the last step, not the first.
    assert_eq!(s.step(ready(50), 5, 2 * PENDING_WINDOW_MS), Some(65));
    assert_eq!(s.step(ready(50), 5, 3 * PENDING_WINDOW_MS + 1), Some(55));
}

#[test]
fn stepper_window_wraps() {
    let mut s = Stepper::default();
    assert_eq!(s.step(ready(50), 5, u32::MAX - 50), Some(55));
    // 120 ms later, past the u32 wrap.
    assert_eq!(s.step(ready(50), 5, 69), Some(60));
}

#[test]
fn stepper_forgets_target_when_unavailable() {
    let mut s = Stepper::default();
    s.step(ready(50), 5, 0);
    s.observe(VolumeState::Unavailable);
    // Back within the window: steps from the new report, not the old target.
    assert_eq!(s.step(ready(20), 5, 100), Some(25));
}

// ── The keys ─────────────────────────────────────────────────────────────
//
// The grab itself is `services::keys`'s, tested there.

#[derive(Default)]
struct Sets(Vec<u32>);
impl Resource for Sets {}

/// The module over a compositor that grants grabs.
fn fake() -> Fake {
    let mut f = Fake::new(&[("phosh_private", 8)], |m| m);
    f.app
        .init_resource::<Sets>()
        .system(|app: &mut App, s: &SetVolume| app.resource_mut::<Sets>().0.push(s.0));
    f.app.add_module(VolumeKeysModule);
    f.turn();
    f
}

fn press(f: &mut Fake, accelerator: &'static str, timestamp: u32) {
    f.app.signal(Key {
        accelerator,
        action: KeyAction::Pressed { timestamp },
    });
}

fn sets(f: &mut Fake) -> Vec<u32> {
    f.app.flush();
    f.app.resource::<Sets>().0.clone()
}

#[test]
fn each_press_is_one_step() {
    let mut f = fake();
    f.app.insert_resource(ready(50));
    f.app.flush();
    for i in 0..5 {
        press(&mut f, RAISE, 1000 + i * 120);
    }
    assert_eq!(sets(&mut f), [55, 60, 65, 70, 75]);
    // The slider drags the volume to 30; a press after a pause steps from
    // there, not from the old target.
    f.app.insert_resource(ready(30));
    f.app.flush();
    press(&mut f, LOWER, 1000 + 4 * 120 + 600);
    assert_eq!(sets(&mut f), [55, 60, 65, 70, 75, 25]);
}

#[test]
fn other_keys_failures_and_no_volume_do_nothing() {
    let mut f = fake();
    f.app.insert_resource(ready(50));
    f.app.flush();
    press(&mut f, "XF86PowerOff", 0);
    f.app.signal(Key {
        accelerator: RAISE,
        action: KeyAction::Failed(2),
    });
    assert!(sets(&mut f).is_empty());
    f.app.insert_resource(VolumeState::Unavailable);
    f.app.flush();
    press(&mut f, RAISE, 0);
    assert!(sets(&mut f).is_empty());
}

#[test]
fn no_phosh_private_no_module() {
    let mut f = Fake::new(&[("wl_seat", 9)], |m| m);
    f.app.add_module(VolumeKeysModule);
    f.turn();
    assert!(!f.app.has_resource::<super::VolumeKeys>());
}
