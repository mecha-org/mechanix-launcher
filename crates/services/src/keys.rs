//! Hardware keys grabbed from the compositor, through one
//! `mx_accelerators_v1` for the whole shell.
//!
//! - [`grab_key`] asks for an accelerator (`XF86PowerOff`,
//!   `XF86AudioRaiseVolume`, ...). The first call binds it.
//! - Everything about a grabbed key comes back as a [`Key`]: whether the
//!   grab took, then its presses and releases.
//! - The compositor repeats a held grabbed key: each repeat is another
//!   [`KeyAction::Pressed`], until the [`KeyAction::Released`].

use mecha_wayland::prelude::*;

/// Something about a key grabbed with [`grab_key`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Key {
    pub accelerator: &'static str,
    pub action: KeyAction,
}
impl Signal for Key {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyAction {
    /// The compositor gave us the key.
    Grabbed,
    /// The compositor refused it, with its error code.
    Failed(u32),
    /// Pressed, or repeated while held. The timestamp is in ms.
    Pressed {
        timestamp: u32,
    },
    Released {
        timestamp: u32,
    },
}

struct Keys {
    accelerators: MxAcceleratorsV1,
    /// Requested accelerators, and their action ids once granted.
    grabs: Vec<(&'static str, Option<u32>)>,
}
impl Resource for Keys {}

/// Grab `accelerator`. False if the compositor does not offer
/// `mx_accelerators_v1`, and nothing will come of it.
pub fn grab_key(app: &mut App, accelerator: &'static str) -> bool {
    if !app.has_resource::<Keys>() {
        let bound = {
            let (globals, mut wl) = app.query::<(Res<Globals>, ResMut<Wayland>)>();
            globals.bind_if_offered::<MxAcceleratorsV1>(&mut wl, 1)
        };
        let Some(accelerators) = bound else {
            return false;
        };
        app.insert_resource(Keys {
            accelerators,
            grabs: Vec::new(),
        });
        app.system(on_accelerators);
    }
    let (mut keys, mut wl) = app.query::<(ResMut<Keys>, ResMut<Wayland>)>();
    if !keys.grabs.iter().any(|g| g.0 == accelerator) {
        keys.grabs.push((accelerator, None));
        keys.accelerators.grab(&mut wl, accelerator);
    }
    true
}

fn on_accelerators(app: &mut App, e: &MxAcceleratorsV1Event) {
    let key = heard(&mut app.resource_mut::<Keys>(), e);
    if let Some((accelerator, action)) = key {
        app.signal(Key {
            accelerator,
            action,
        });
    }
}

/// Which of our accelerators `e` is about, and what happened to it.
fn heard(keys: &mut Keys, e: &MxAcceleratorsV1Event) -> Option<(&'static str, KeyAction)> {
    use MxAcceleratorsV1Event as E;
    let by_action = |keys: &Keys, id: u32| keys.grabs.iter().find(|g| g.1 == Some(id)).map(|g| g.0);
    match e {
        E::Grabbed {
            accelerators,
            accelerator,
            action_id,
        } if *accelerators == keys.accelerators => {
            let grab = keys.grabs.iter_mut().find(|g| g.0 == *accelerator)?;
            grab.1 = Some(*action_id);
            Some((grab.0, KeyAction::Grabbed))
        }
        E::GrabFailed {
            accelerators,
            accelerator,
            error,
        } if *accelerators == keys.accelerators => {
            let grab = keys.grabs.iter().find(|g| g.0 == *accelerator)?;
            Some((grab.0, KeyAction::Failed(u32::from(*error))))
        }
        E::Pressed {
            accelerators,
            action_id,
            time,
        } if *accelerators == keys.accelerators => {
            let pressed = KeyAction::Pressed { timestamp: *time };
            Some((by_action(keys, *action_id)?, pressed))
        }
        E::Released {
            accelerators,
            action_id,
            time,
        } if *accelerators == keys.accelerators => {
            let released = KeyAction::Released { timestamp: *time };
            Some((by_action(keys, *action_id)?, released))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use wayland::fake::{Fake, Request};

    use super::*;

    const REGISTRY: u32 = 2;
    const OP_REGISTRY_BIND: u16 = 0;
    const OP_GRAB: u16 = 1;
    const EV_GRABBED: u16 = 0;
    const EV_GRAB_FAILED: u16 = 1;
    const EV_PRESSED: u16 = 2;
    const EV_RELEASED: u16 = 3;

    const POWER: &str = "XF86PowerOff";
    const RAISE: &str = "XF86AudioRaiseVolume";

    #[derive(Default)]
    struct Heard(Vec<Key>);
    impl Resource for Heard {}

    fn fake(globals: &[(&str, u32)]) -> Fake {
        let mut f = Fake::new(globals, |m| m);
        f.app
            .init_resource::<Heard>()
            .system(|app: &mut App, k: &Key| app.resource_mut::<Heard>().0.push(*k));
        f
    }

    /// The ids the app bound `mx_accelerators_v1` to, in bind order.
    fn binds(reqs: &[Request]) -> Vec<ObjectId> {
        reqs.iter()
            .filter(|r| r.sender == ObjectId(REGISTRY) && r.opcode == OP_REGISTRY_BIND)
            .filter_map(|r| {
                let mut rd = r.reader();
                rd.uint();
                (rd.string().as_deref() == Some("mx_accelerators_v1")).then(|| {
                    rd.uint();
                    rd.object().expect("new id")
                })
            })
            .collect()
    }

    /// The one bind of `mx_accelerators_v1`.
    fn ours(reqs: &[Request]) -> ObjectId {
        let binds = binds(reqs);
        assert_eq!(binds.len(), 1, "one bind among {reqs:?}");
        binds[0]
    }

    fn settle(f: &mut Fake) {
        f.turn();
        f.app.tick();
        f.app.flush();
    }

    fn heard(f: &Fake) -> Vec<Key> {
        f.app.resource::<Heard>().0.clone()
    }

    #[test]
    fn one_bind_for_every_key() {
        let mut f = fake(&[("mx_accelerators_v1", 1)]);
        assert!(grab_key(&mut f.app, POWER));
        assert!(grab_key(&mut f.app, RAISE));
        assert!(grab_key(&mut f.app, POWER));
        f.turn();
        let reqs = f.requests();
        let accelerators = ours(&reqs);
        let grabbed: Vec<String> = reqs
            .iter()
            .filter(|r| r.sender == accelerators && r.opcode == OP_GRAB)
            .map(|r| r.reader().string().expect("accelerator"))
            .collect();
        assert_eq!(grabbed, [POWER, RAISE]);
    }

    #[test]
    fn a_grabbed_key_is_heard_until_released() {
        let mut f = fake(&[("mx_accelerators_v1", 1)]);
        grab_key(&mut f.app, POWER);
        grab_key(&mut f.app, RAISE);
        f.turn();
        let accelerators = ours(&f.requests());
        f.send(accelerators.0, EV_GRABBED, |w| {
            w.string(POWER);
            w.uint(7);
        });
        f.send(accelerators.0, EV_GRAB_FAILED, |w| {
            w.string(RAISE);
            w.uint(1);
        });
        f.send(accelerators.0, EV_PRESSED, |w| {
            w.uint(7);
            w.uint(10);
        });
        // An action that isn't ours.
        f.send(accelerators.0, EV_PRESSED, |w| {
            w.uint(9);
            w.uint(11);
        });
        f.send(accelerators.0, EV_RELEASED, |w| {
            w.uint(7);
            w.uint(12);
        });
        settle(&mut f);
        let key = |accelerator, action| Key {
            accelerator,
            action,
        };
        assert_eq!(
            heard(&f),
            [
                key(POWER, KeyAction::Grabbed),
                key(RAISE, KeyAction::Failed(1)),
                key(POWER, KeyAction::Pressed { timestamp: 10 }),
                key(POWER, KeyAction::Released { timestamp: 12 }),
            ]
        );
    }

    #[test]
    fn without_the_global_nothing_is_grabbed() {
        let mut f = fake(&[("wl_seat", 9)]);
        f.requests();
        assert!(!grab_key(&mut f.app, POWER));
        f.turn();
        let sent = f.requests();
        assert!(sent.is_empty(), "requests without the global: {sent:?}");
    }

    #[test]
    fn its_own_bind_beside_toplevels() {
        let mut f = fake(&[
            ("mx_accelerators_v1", 1),
            ("mx_thumbnail_view_manager_v1", 1),
        ]);
        f.app.add_module(ToplevelsModule);
        grab_key(&mut f.app, POWER);
        f.turn();
        let accelerators = ours(&f.requests());
        f.send(accelerators.0, EV_GRABBED, |w| {
            w.string(POWER);
            w.uint(1);
        });
        settle(&mut f);
        assert_eq!(
            heard(&f),
            [Key {
                accelerator: POWER,
                action: KeyAction::Grabbed
            }]
        );
    }
}
