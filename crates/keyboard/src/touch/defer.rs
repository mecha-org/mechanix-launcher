//! Holding back a press until its view is laid out.
//!
//! A newly shown view has no key rectangles until the next layout pass, so a
//! tap that lands right after a view switch (Shift, then a letter at once)
//! would resolve against empty rectangles. Such a press waits here and is
//! resolved at its contact's next move or release, once the layout has run;
//! if it still hasn't by release, the tap is dropped rather than guessed.

use mecha_wayland::prelude::*;

use super::resolve::KeyGeom;

/// Whether `keys` come from a layout pass: some key has a non-empty
/// rectangle. A view that hasn't been laid out has only empty ones.
pub(crate) fn laid_out(keys: &[KeyGeom]) -> bool {
    keys.iter()
        .any(|k| k.rect.width() > 0.0 && k.rect.height() > 0.0)
}

/// A press held back until its view is laid out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PendingPress {
    pub contact: ContactId,
    pub position: Point,
    pub time: u32,
}

/// A contact event, as far as the deferral cares.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Contact {
    Press(PendingPress),
    Moved(ContactId),
    Release(ContactId),
    Cancel(ContactId),
}

/// What to do with a contact event.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Gate {
    /// A held-back press to act on first, as if it had just arrived.
    pub replay: Option<PendingPress>,
    /// Whether to act on the event itself.
    pub forward: bool,
}

/// Holds at most one press back until the layout catches up.
#[derive(Default)]
pub(crate) struct Deferral {
    pending: Option<PendingPress>,
}

impl Deferral {
    /// Decide what to do with `event`, given whether the current view is
    /// [`laid_out`] now.
    pub(crate) fn gate(&mut self, event: Contact, laid_out: bool) -> Gate {
        const HOLD: Gate = Gate {
            replay: None,
            forward: false,
        };
        match event {
            Contact::Press(p) => {
                if laid_out {
                    // An older held-back press goes first, as if its
                    // contact were still down when this one arrived.
                    Gate {
                        replay: self.pending.take(),
                        forward: true,
                    }
                } else {
                    // Replaces any older held-back press, never pressed.
                    self.pending = Some(p);
                    HOLD
                }
            }
            Contact::Moved(c) | Contact::Release(c) | Contact::Cancel(c)
                if self.pending.is_some_and(|p| p.contact == c) =>
            {
                match event {
                    Contact::Cancel(_) => {
                        self.pending = None;
                        HOLD
                    }
                    _ if laid_out => Gate {
                        replay: self.pending.take(),
                        forward: true,
                    },
                    Contact::Release(_) => {
                        self.pending = None;
                        HOLD
                    }
                    _ => HOLD,
                }
            }
            _ => Gate {
                replay: None,
                forward: true,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::predict::FLAT;
    use crate::touch::resolve::{ResolveParams, resolve};
    use crate::touch::tracker::{KeyRef, TouchOut, TouchTracker};

    const A: ContactId = ContactId::Touch(1);
    const B: ContactId = ContactId::Touch(2);

    fn press(c: ContactId, x: f32, time: u32) -> PendingPress {
        PendingPress {
            contact: c,
            position: Point::new(x, 10.0),
            time,
        }
    }

    /// Two 40x40 letter keys side by side, or the same two un-laid-out.
    fn keys(laid: bool) -> Vec<KeyGeom> {
        let r = |x: f32| {
            if laid {
                Rect::new(x, 0.0, 40.0, 40.0)
            } else {
                Rect::default()
            }
        };
        vec![
            KeyGeom {
                rect: r(0.0),
                letter: Some('q'),
            },
            KeyGeom {
                rect: r(40.0),
                letter: Some('h'),
            },
        ]
    }

    /// Feed `events` through a deferral, resolver and tracker as the glue
    /// does, with `laid[i]` saying whether the view is laid out at event `i`;
    /// returns the tracker outputs.
    fn drive(events: &[(Contact, bool)], positions: &[f32]) -> Vec<TouchOut> {
        let mut d = Deferral::default();
        let mut t = TouchTracker::default();
        let mut out = Vec::new();
        let key = |laid: bool, x: f32| {
            let k = keys(laid);
            let p = Point::new(x, 10.0);
            resolve(&k, p, &FLAT, &ResolveParams::for_keys(&k, 0.0)).map(|index| KeyRef {
                index,
                repeats: false,
            })
        };
        for (i, &(event, laid)) in events.iter().enumerate() {
            let gate = d.gate(event, laid);
            if let Some(p) = gate.replay {
                out.extend(t.press(p.contact, key(laid, p.position.x), p.time));
            }
            if !gate.forward {
                continue;
            }
            out.extend(match event {
                Contact::Press(p) => t.press(p.contact, key(laid, p.position.x), p.time),
                Contact::Moved(c) => t.moved(c, key(laid, positions[i]), i as u32),
                Contact::Release(c) => t.release(c, i as u32),
                Contact::Cancel(c) => t.cancel(c, i as u32),
            });
        }
        out
    }

    #[test]
    fn laid_out_needs_a_non_empty_rect() {
        assert!(!laid_out(&keys(false)));
        assert!(laid_out(&keys(true)));
        assert!(!laid_out(&[]));
    }

    #[test]
    fn laid_out_press_passes_through() {
        let mut d = Deferral::default();
        let g = d.gate(Contact::Press(press(A, 60.0, 0)), true);
        assert_eq!(
            g,
            Gate {
                replay: None,
                forward: true
            }
        );
        assert_eq!(
            d.gate(Contact::Release(A), true),
            Gate {
                replay: None,
                forward: true
            }
        );
    }

    #[test]
    fn early_press_commits_key_under_press_once_laid_out() {
        // Press at h's centre before layout; layout lands before release.
        let out = drive(
            &[
                (Contact::Press(press(A, 60.0, 0)), false),
                (Contact::Release(A), true),
            ],
            &[0.0, 0.0],
        );
        assert_eq!(
            out,
            vec![
                TouchOut::Pressed(1),
                TouchOut::Commit(1),
                TouchOut::Unpressed(1)
            ]
        );
    }

    #[test]
    fn early_press_resolves_on_first_laid_out_move() {
        let out = drive(
            &[
                (Contact::Press(press(A, 60.0, 0)), false),
                (Contact::Moved(A), false),
                (Contact::Moved(A), true),
                (Contact::Release(A), true),
            ],
            &[0.0, 61.0, 62.0, 0.0],
        );
        assert_eq!(
            out,
            vec![
                TouchOut::Pressed(1),
                TouchOut::Commit(1),
                TouchOut::Unpressed(1)
            ]
        );
    }

    #[test]
    fn never_laid_out_tap_types_nothing() {
        let out = drive(
            &[
                (Contact::Press(press(A, 60.0, 0)), false),
                (Contact::Release(A), false),
            ],
            &[0.0, 0.0],
        );
        assert_eq!(out, vec![]);
    }

    #[test]
    fn cancelled_early_press_types_nothing() {
        let out = drive(
            &[
                (Contact::Press(press(A, 60.0, 0)), false),
                (Contact::Cancel(A), true),
            ],
            &[0.0, 0.0],
        );
        assert_eq!(out, vec![]);
    }

    #[test]
    fn other_contacts_pass_while_a_press_waits() {
        let mut d = Deferral::default();
        d.gate(Contact::Press(press(A, 60.0, 0)), false);
        assert_eq!(
            d.gate(Contact::Release(B), false),
            Gate {
                replay: None,
                forward: true
            }
        );
        // A later laid-out press replays the waiting one first.
        let b = press(B, 10.0, 5);
        assert_eq!(
            d.gate(Contact::Press(b), true),
            Gate {
                replay: Some(press(A, 60.0, 0)),
                forward: true
            }
        );
    }
}
