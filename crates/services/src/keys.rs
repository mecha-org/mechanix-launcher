//! Hardware keys grabbed from the compositor, through one
//! `phosh_private_keyboard_event` for the whole shell.
//!
//! - [`grab_key`] asks for an accelerator (`XF86PowerOff`,
//!   `XF86AudioRaiseVolume`, ...). The first call binds `phosh_private`.
//! - Everything about a grabbed key comes back as a [`Key`]: whether the
//!   grab took, then its presses and releases.
//! - The compositor repeats a held grabbed key: each repeat is another
//!   [`KeyAction::Pressed`], until the [`KeyAction::Released`].

use mecha_wayland::prelude::*;

/// `phosh_private.get_keyboard_event` is `since="5"`.
const MIN_VERSION: u32 = 5;

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
    events: PhoshPrivateKeyboardEvent,
    /// Requested accelerators, and their action ids once granted.
    grabs: Vec<(&'static str, Option<u32>)>,
}
impl Resource for Keys {}

/// Grab `accelerator`. False if the compositor has no `phosh_private`
/// recent enough to grab keys, and nothing will come of it.
pub fn grab_key(app: &mut App, accelerator: &'static str) -> bool {
    if !app.has_resource::<Keys>() {
        let events = {
            let (globals, mut wl) = app.query::<(Res<Globals>, ResMut<Wayland>)>();
            let Some(phosh) = globals.bind_if_offered::<PhoshPrivate>(&mut wl, MIN_VERSION) else {
                return false;
            };
            phosh.get_keyboard_event(&mut wl)
        };
        app.insert_resource(Keys {
            events,
            grabs: Vec::new(),
        });
        app.system(on_keyboard_event);
    }
    let (mut keys, mut wl) = app.query::<(ResMut<Keys>, ResMut<Wayland>)>();
    if !keys.grabs.iter().any(|g| g.0 == accelerator) {
        keys.grabs.push((accelerator, None));
        keys.events.grab_accelerator_request(&mut wl, accelerator);
    }
    true
}

fn on_keyboard_event(app: &mut App, e: &PhoshPrivateKeyboardEventEvent) {
    use PhoshPrivateKeyboardEventEvent as E;
    let key = {
        let mut keys = app.resource_mut::<Keys>();
        let ours = keys.events;
        let by_action =
            |keys: &Keys, id: u32| keys.grabs.iter().find(|g| g.1 == Some(id)).map(|g| g.0);
        match e {
            E::GrabSuccessEvent {
                phosh_private_keyboard_event: k,
                accelerator,
                action_id,
            } if *k == ours => keys
                .grabs
                .iter_mut()
                .find(|g| g.0 == *accelerator)
                .map(|g| {
                    g.1 = Some(*action_id);
                    (g.0, KeyAction::Grabbed)
                }),
            E::GrabFailedEvent {
                phosh_private_keyboard_event: k,
                accelerator,
                error,
            } if *k == ours => keys
                .grabs
                .iter()
                .find(|g| g.0 == *accelerator)
                .map(|g| (g.0, KeyAction::Failed(*error))),
            E::AcceleratorActivatedEvent {
                phosh_private_keyboard_event: k,
                action_id,
                timestamp,
            } if *k == ours => by_action(&keys, *action_id).map(|a| {
                (
                    a,
                    KeyAction::Pressed {
                        timestamp: *timestamp,
                    },
                )
            }),
            E::AcceleratorReleasedEvent {
                phosh_private_keyboard_event: k,
                action_id,
                timestamp,
            } if *k == ours => by_action(&keys, *action_id).map(|a| {
                (
                    a,
                    KeyAction::Released {
                        timestamp: *timestamp,
                    },
                )
            }),
            _ => None,
        }
    };
    if let Some((accelerator, action)) = key {
        app.signal(Key {
            accelerator,
            action,
        });
    }
}

#[cfg(test)]
mod tests {
    use wayland::fake::{Fake, Request};

    use super::*;

    const REGISTRY: u32 = 2;
    const OP_REGISTRY_BIND: u16 = 0;
    // Opcodes from phosh-private.xml, in the order it declares them.
    const OP_GET_KEYBOARD_EVENT: u16 = 3;
    const OP_GRAB: u16 = 0;
    const EV_ACTIVATED: u16 = 0;
    const EV_GRAB_FAILED: u16 = 1;
    const EV_GRAB_SUCCESS: u16 = 2;
    const EV_RELEASED: u16 = 5;

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

    /// The `phosh_private_keyboard_event` got on the last bind of
    /// `phosh_private`: ours.
    fn keyboard_of(reqs: &[Request]) -> ObjectId {
        let phosh = *phosh_binds(reqs).last().expect("a bind of phosh_private");
        let gets: Vec<_> = reqs
            .iter()
            .filter(|r| r.opcode == OP_GET_KEYBOARD_EVENT && r.sender == phosh)
            .collect();
        assert_eq!(gets.len(), 1, "one get_keyboard_event among {reqs:?}");
        gets[0].reader().object().expect("new id")
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
        let mut f = fake(&[("phosh_private", 8)]);
        assert!(grab_key(&mut f.app, POWER));
        assert!(grab_key(&mut f.app, RAISE));
        assert!(grab_key(&mut f.app, POWER));
        f.turn();
        let reqs = f.requests();
        assert_eq!(phosh_binds(&reqs).len(), 1);
        let keyboard = keyboard_of(&reqs);
        let grabbed: Vec<String> = reqs
            .iter()
            .filter(|r| r.sender == keyboard && r.opcode == OP_GRAB)
            .map(|r| r.reader().string().expect("accelerator"))
            .collect();
        assert_eq!(grabbed, [POWER, RAISE]);
    }

    #[test]
    fn a_grabbed_key_is_heard_until_released() {
        let mut f = fake(&[("phosh_private", 8)]);
        grab_key(&mut f.app, POWER);
        grab_key(&mut f.app, RAISE);
        f.turn();
        let keyboard = keyboard_of(&f.requests());
        f.send(keyboard.0, EV_GRAB_SUCCESS, |w| {
            w.string(POWER);
            w.uint(7);
        });
        f.send(keyboard.0, EV_GRAB_FAILED, |w| {
            w.string(RAISE);
            w.uint(2);
        });
        f.send(keyboard.0, EV_ACTIVATED, |w| {
            w.uint(7);
            w.uint(10);
        });
        // An action that isn't ours.
        f.send(keyboard.0, EV_ACTIVATED, |w| {
            w.uint(9);
            w.uint(11);
        });
        f.send(keyboard.0, EV_RELEASED, |w| {
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
                key(RAISE, KeyAction::Failed(2)),
                key(POWER, KeyAction::Pressed { timestamp: 10 }),
                key(POWER, KeyAction::Released { timestamp: 12 }),
            ]
        );
    }

    #[test]
    fn without_phosh_private_nothing_is_grabbed() {
        let mut f = fake(&[("wl_seat", 9)]);
        f.requests();
        assert!(!grab_key(&mut f.app, POWER));
        f.turn();
        let sent = f.requests();
        assert!(sent.is_empty(), "requests without phosh_private: {sent:?}");
    }

    #[test]
    fn its_own_bind_beside_toplevels() {
        let mut f = fake(&[("phosh_private", 8)]);
        f.app.add_module(ToplevelsModule);
        grab_key(&mut f.app, POWER);
        f.turn();
        let reqs = f.requests();
        let binds = phosh_binds(&reqs);
        assert_eq!(binds.len(), 2, "two phosh_private binds among {reqs:?}");
        let keyboard = keyboard_of(&reqs);
        f.send(keyboard.0, EV_GRAB_SUCCESS, |w| {
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
