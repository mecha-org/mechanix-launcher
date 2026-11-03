use std::path::PathBuf;

use mecha_pulseaudio::{SetVolume, VolumeState};
use mecha_wayland::prelude::*;
use wayland::fake::{Fake, Request};

use super::{PENDING_WINDOW_MS, Stepper, VolumeKeysModule};

const REGISTRY: u32 = 2;
const OP_REGISTRY_BIND: u16 = 0;

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
    assert_eq!(Stepper::default().step(ready(150), 5, 0), Some(100));
    assert_eq!(Stepper::default().step(ready(150), -5, 0), Some(95));
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

// ── Wayland ──────────────────────────────────────────────────────────────

/// The vendored `phosh-private.xml` of the locked mecha-wayland rev, from
/// cargo's git checkout of it.
fn phosh_private_xml() -> String {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let lock = std::fs::read_to_string(manifest.join("../../Cargo.lock")).expect("Cargo.lock");
    let rev = lock
        .lines()
        .find_map(|l| {
            l.strip_prefix("source = \"git+https://github.com/mecha-org/mecha-wayland")
                .and_then(|l| l.split('#').nth(1))
                .map(|r| r.trim_end_matches('"').to_string())
        })
        .expect("locked mecha-wayland rev");
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").expect("HOME")).join(".cargo"));
    let checkouts = cargo_home.join("git/checkouts");
    for repo in std::fs::read_dir(&checkouts).expect("cargo git checkouts") {
        let repo = repo.unwrap().path();
        if !repo
            .file_name()
            .is_some_and(|n| n.to_string_lossy().starts_with("mecha-wayland-"))
        {
            continue;
        }
        for co in std::fs::read_dir(&repo).unwrap() {
            let co = co.unwrap().path();
            let short = co.file_name().unwrap().to_string_lossy().into_owned();
            if !short.is_empty() && rev.starts_with(&short) {
                let xml = co.join("crates/wayland/protocols/phosh-private.xml");
                return std::fs::read_to_string(&xml).expect("phosh-private.xml");
            }
        }
    }
    panic!("no checkout of mecha-wayland {rev} under {checkouts:?}");
}

/// The index of `<kind name="name"` among the `kind`s of `interface`, in
/// the vendored XML's order: the wire opcode.
fn opcode(interface: &str, kind: &str, name: &str) -> u16 {
    let xml = phosh_private_xml();
    let start = xml
        .find(&format!("<interface name=\"{interface}\""))
        .unwrap_or_else(|| panic!("interface {interface}"));
    let body = &xml[start..];
    let body = &body[..body.find("</interface>").expect("interface end")];
    body.split(&format!("<{kind} name=\""))
        .skip(1)
        .position(|r| r.starts_with(&format!("{name}\"")))
        .unwrap_or_else(|| panic!("{interface}.{name}")) as u16
}

fn op_get_keyboard_event() -> u16 {
    opcode("phosh_private", "request", "get_keyboard_event")
}
fn op_grab() -> u16 {
    opcode(
        "phosh_private_keyboard_event",
        "request",
        "grab_accelerator_request",
    )
}
fn ev_grab_success() -> u16 {
    opcode(
        "phosh_private_keyboard_event",
        "event",
        "grab_success_event",
    )
}
fn ev_grab_failed() -> u16 {
    opcode("phosh_private_keyboard_event", "event", "grab_failed_event")
}
fn ev_activated() -> u16 {
    opcode(
        "phosh_private_keyboard_event",
        "event",
        "accelerator_activated_event",
    )
}

#[derive(Default)]
struct Sets(Vec<u32>);
impl Resource for Sets {}

fn record(app: &mut App, s: &SetVolume) {
    app.resource_mut::<Sets>().0.push(s.0);
}

fn fake_with(globals: &[(&str, u32)]) -> Fake {
    let mut f = Fake::new(globals, |m| m);
    f.app.init_resource::<Sets>().system(record);
    f.app.add_module(VolumeKeysModule);
    f.turn();
    f
}

/// Install over a fake offering `phosh_private` 8 and return the module's
/// `phosh_private_keyboard_event` id, with the install requests drained.
fn fake() -> (Fake, ObjectId) {
    let mut f = fake_with(&[("phosh_private", 8)]);
    let reqs = f.requests();
    let keyboard = keyboard_of(&reqs);
    (f, keyboard)
}

/// The ids the app bound `phosh_private` to, in bind order.
fn phosh_binds(reqs: &[Request]) -> Vec<ObjectId> {
    reqs.iter()
        .filter(|r| r.sender == ObjectId(REGISTRY) && r.opcode == OP_REGISTRY_BIND)
        .filter_map(|r| {
            let mut rd = r.reader();
            rd.uint();
            (rd.string().as_deref() == Some("phosh_private")).then(|| {
                rd.uint();
                rd.object().expect("new id")
            })
        })
        .collect()
}

/// The `phosh_private_keyboard_event` the app got on the last bind of
/// `phosh_private`: the module's own.
fn keyboard_of(reqs: &[Request]) -> ObjectId {
    let phosh = *phosh_binds(reqs)
        .last()
        .unwrap_or_else(|| panic!("no bind of phosh_private among {reqs:?}"));
    let gets: Vec<_> = reqs
        .iter()
        .filter(|r| r.opcode == op_get_keyboard_event() && phosh_binds(reqs).contains(&r.sender))
        .collect();
    assert_eq!(gets.len(), 1, "one get_keyboard_event among {reqs:?}");
    assert_eq!(
        gets[0].sender, phosh,
        "get_keyboard_event on the module's bind"
    );
    gets[0].reader().object().expect("new id")
}

fn sets(f: &Fake) -> Vec<u32> {
    f.app.resource::<Sets>().0.clone()
}

fn settle(f: &mut Fake) {
    f.turn();
    f.app.tick();
    f.app.flush();
}

#[test]
fn grabs_both_volume_keys() {
    let mut f = fake_with(&[("phosh_private", 8)]);
    let reqs = f.requests();
    let keyboard = keyboard_of(&reqs);
    let grabbed: Vec<String> = reqs
        .iter()
        .filter(|r| r.sender == keyboard && r.opcode == op_grab())
        .map(|r| r.reader().string().expect("accelerator"))
        .collect();
    assert_eq!(grabbed, ["XF86AudioRaiseVolume", "XF86AudioLowerVolume"]);
}

#[test]
fn activation_sets_volume() {
    let (mut f, keyboard) = fake();
    f.app.insert_resource(ready(40));
    settle(&mut f);
    f.send(keyboard.0, ev_grab_success(), |w| {
        w.string("XF86AudioLowerVolume");
        w.uint(7);
    });
    f.send(keyboard.0, ev_activated(), |w| {
        w.uint(7);
        w.uint(0);
    });
    settle(&mut f);
    assert_eq!(sets(&f), [35]);
}

#[test]
fn each_activation_is_one_step() {
    let (mut f, keyboard) = fake();
    f.app.insert_resource(ready(50));
    settle(&mut f);
    f.send(keyboard.0, ev_grab_success(), |w| {
        w.string("XF86AudioRaiseVolume");
        w.uint(3);
    });
    f.send(keyboard.0, ev_grab_success(), |w| {
        w.string("XF86AudioLowerVolume");
        w.uint(4);
    });
    for i in 0..5 {
        f.send(keyboard.0, ev_activated(), |w| {
            w.uint(3);
            w.uint(1000 + i * 120);
        });
    }
    settle(&mut f);
    assert_eq!(sets(&f), [55, 60, 65, 70, 75]);
    // The slider drags the volume to 30; a press after a pause steps from
    // there, not from the old target.
    f.app.insert_resource(ready(30));
    settle(&mut f);
    f.send(keyboard.0, ev_activated(), |w| {
        w.uint(4);
        w.uint(1000 + 4 * 120 + 600);
    });
    settle(&mut f);
    assert_eq!(sets(&f), [55, 60, 65, 70, 75, 25]);
}

#[test]
fn unknown_action_and_failed_grab_do_nothing() {
    let (mut f, keyboard) = fake();
    f.app.insert_resource(ready(50));
    settle(&mut f);
    f.send(keyboard.0, ev_grab_failed(), |w| {
        w.string("XF86AudioRaiseVolume");
        w.uint(2);
    });
    f.send(keyboard.0, ev_activated(), |w| {
        w.uint(9);
        w.uint(0);
    });
    settle(&mut f);
    assert!(sets(&f).is_empty());
}

#[test]
fn no_volume_no_set() {
    let (mut f, keyboard) = fake();
    f.app.insert_resource(VolumeState::Unavailable);
    settle(&mut f);
    f.send(keyboard.0, ev_grab_success(), |w| {
        w.string("XF86AudioRaiseVolume");
        w.uint(1);
    });
    f.send(keyboard.0, ev_activated(), |w| {
        w.uint(1);
        w.uint(0);
    });
    settle(&mut f);
    assert!(sets(&f).is_empty());
}

#[test]
fn no_phosh_private_no_grabs() {
    let mut f = Fake::new(&[("wl_seat", 9)], |m| m);
    f.requests();
    f.app.add_module(VolumeKeysModule);
    f.turn();
    let sent = f.requests();
    assert!(sent.is_empty(), "requests without phosh_private: {sent:?}");
}

#[test]
fn own_bind_beside_toplevels() {
    let mut f = Fake::new(&[("phosh_private", 8)], |m| m);
    f.app.init_resource::<Sets>().system(record);
    f.app.add_module(ToplevelsModule);
    f.app.add_module(VolumeKeysModule);
    f.turn();
    let reqs = f.requests();
    let binds = phosh_binds(&reqs);
    assert_eq!(binds.len(), 2, "two phosh_private binds among {reqs:?}");
    assert_ne!(binds[0], binds[1]);
    let keyboard = keyboard_of(&reqs);
    f.app.insert_resource(ready(40));
    settle(&mut f);
    f.send(keyboard.0, ev_grab_success(), |w| {
        w.string("XF86AudioRaiseVolume");
        w.uint(1);
    });
    f.send(keyboard.0, ev_activated(), |w| {
        w.uint(1);
        w.uint(0);
    });
    settle(&mut f);
    assert_eq!(sets(&f), [45]);
}
