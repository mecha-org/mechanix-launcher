//! The order of operations for each contact event, over an abstract
//! [`Board`] so it can be tested without widgets: hold a press back until its
//! view is laid out ([`Deferral`]), finish an earlier finger before a new one
//! is resolved, resolve against the current view with the prior taken at the
//! press, and hand the tracker's outputs to the board.

use mecha_wayland::prelude::*;

use super::defer::{Contact, Deferral, PendingPress, laid_out};
use super::resolve::{KeyGeom, ResolveParams, resolve};
use super::tracker::{KeyRef, TouchOut, TouchTracker};
use crate::predict::{FLAT, LetterPrior};

/// The input layer's per-contact state beside the tracker.
pub(crate) struct Flow {
    /// A press waiting for its view's layout.
    deferral: Deferral,
    /// The next-letter prior for the current contact, taken at its press:
    /// the word can't change while a finger is down.
    prior: LetterPrior,
    /// The prediction strength, taken with `prior`.
    strength: f32,
}

impl Default for Flow {
    fn default() -> Self {
        Self {
            deferral: Deferral::default(),
            prior: FLAT,
            strength: 0.0,
        }
    }
}

/// What the flow needs from the keyboard.
pub(crate) trait Board {
    /// The current view's keys, with their laid-out rectangles.
    fn geoms(&mut self) -> Vec<KeyGeom>;
    /// The prior over the next letter and the prediction strength, for the
    /// word typed so far.
    fn predict(&mut self) -> (LetterPrior, f32);
    /// Whether key `index` of the current view repeats while held.
    fn repeats(&mut self, index: usize) -> bool;
    fn tracker(&mut self) -> &mut TouchTracker;
    fn flow(&mut self) -> &mut Flow;
    /// Act on tracker outputs (indices into the current view), then cancel
    /// the active contact if acting changed the view.
    fn act(&mut self, out: Vec<TouchOut>, time: u32);
}

/// A contact went down.
pub(crate) fn press(b: &mut impl Board, p: PendingPress) {
    // Finish the earlier finger first, so this one sees its letter in the
    // prefix and the view it may have switched to.
    finish_others(b, p.contact, p.time);
    let ready = laid_out(&b.geoms());
    let gate = b.flow().deferral.gate(Contact::Press(p), ready);
    if let Some(old) = gate.replay {
        start(b, old);
    }
    if gate.forward {
        start(b, p);
    }
}

/// A contact moved to `position`.
pub(crate) fn moved(b: &mut impl Board, c: ContactId, position: Point, time: u32) {
    if gate(b, Contact::Moved(c)) {
        let geoms = b.geoms();
        let key = key_at(b, &geoms, position);
        let out = b.tracker().moved(c, key, time);
        b.act(out, time);
    }
}

/// A contact lifted. The commit uses the last resolved key (the highlighted
/// one), not a fresh resolve at the release position, by design.
pub(crate) fn release(b: &mut impl Board, c: ContactId, time: u32) {
    if gate(b, Contact::Release(c)) {
        let out = b.tracker().release(c, time);
        b.act(out, time);
    }
}

/// A contact was cancelled; nothing is typed.
pub(crate) fn cancel(b: &mut impl Board, c: ContactId, time: u32) {
    if gate(b, Contact::Cancel(c)) {
        let out = b.tracker().cancel(c, time);
        b.act(out, time);
    }
}

/// Pass a non-press `event` through the deferral, starting a held-back press
/// it releases; whether to act on the event itself.
fn gate(b: &mut impl Board, event: Contact) -> bool {
    let ready = laid_out(&b.geoms());
    let gate = b.flow().deferral.gate(event, ready);
    if let Some(p) = gate.replay
        && !start(b, p)
    {
        // Finishing another finger switched to a view not laid out yet: the
        // press is held again, and a release or cancel now drops it.
        b.flow().deferral.gate(event, false);
        return false;
    }
    gate.forward
}

/// If a contact other than `c` is active, finish it as if lifted at `time`.
fn finish_others(b: &mut impl Board, c: ContactId, time: u32) {
    if b.tracker().contact().is_some_and(|a| a != c) {
        let out = b.tracker().finish_active(time);
        b.act(out, time);
    }
}

/// Resolve and track press `p`, after finishing any other finger; `false`
/// (and the press held back) if the view is not laid out by then.
fn start(b: &mut impl Board, p: PendingPress) -> bool {
    finish_others(b, p.contact, p.time);
    let geoms = b.geoms();
    if !laid_out(&geoms) {
        b.flow().deferral.gate(Contact::Press(p), false);
        return false;
    }
    let (prior, strength) = b.predict();
    let flow = b.flow();
    flow.prior = prior;
    flow.strength = strength;
    let key = key_at(b, &geoms, p.position);
    let out = b.tracker().press(p.contact, key, p.time);
    b.act(out, p.time);
    true
}

/// The key of the current view (whose keys are `geoms`) a touch at `p`
/// means, if any, using the prior taken at the contact's press.
fn key_at(b: &mut impl Board, geoms: &[KeyGeom], p: Point) -> Option<KeyRef> {
    let flow = b.flow();
    let params = ResolveParams::for_keys(geoms, flow.strength);
    let index = resolve(geoms, p, &flow.prior, &params)?;
    let repeats = b.repeats(index);
    Some(KeyRef { index, repeats })
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: ContactId = ContactId::Touch(1);
    const B: ContactId = ContactId::Touch(2);

    /// What a fake key does when committed.
    #[derive(Clone, Copy)]
    enum Act {
        Letter(char),
        View(usize),
    }

    /// Two views of two 40x40 keys at x 0 and 40: a view switch and a letter.
    struct Fake {
        views: Vec<[Act; 2]>,
        laid: Vec<bool>,
        current: usize,
        typed: String,
        /// The prefix each `predict` call saw.
        predicted: Vec<String>,
        tracker: TouchTracker,
        flow: Flow,
    }

    impl Fake {
        fn new(views: Vec<[Act; 2]>) -> Self {
            let n = views.len();
            Self {
                views,
                laid: vec![true; n],
                current: 0,
                typed: String::new(),
                predicted: Vec::new(),
                tracker: TouchTracker::default(),
                flow: Flow::default(),
            }
        }

        fn apply(&mut self, out: Vec<TouchOut>) {
            for o in out {
                if let TouchOut::Commit(i) = o {
                    match self.views[self.current][i] {
                        Act::Letter(c) => self.typed.push(c),
                        Act::View(v) => self.current = v,
                    }
                }
            }
        }
    }

    impl Board for Fake {
        fn geoms(&mut self) -> Vec<KeyGeom> {
            let laid = self.laid[self.current];
            self.views[self.current]
                .iter()
                .enumerate()
                .map(|(i, a)| KeyGeom {
                    rect: if laid {
                        Rect::new(40.0 * i as f32, 0.0, 40.0, 40.0)
                    } else {
                        Rect::default()
                    },
                    letter: match a {
                        Act::Letter(c) => Some(c.to_ascii_lowercase()),
                        Act::View(_) => None,
                    },
                })
                .collect()
        }
        fn predict(&mut self) -> (LetterPrior, f32) {
            self.predicted.push(self.typed.clone());
            (FLAT, 0.0)
        }
        fn repeats(&mut self, _: usize) -> bool {
            false
        }
        fn tracker(&mut self) -> &mut TouchTracker {
            &mut self.tracker
        }
        fn flow(&mut self) -> &mut Flow {
            &mut self.flow
        }
        fn act(&mut self, out: Vec<TouchOut>, time: u32) {
            let view = self.current;
            self.apply(out);
            if self.current != view {
                let out = self.tracker.cancel_active(time);
                self.apply(out);
            }
        }
    }

    fn at(c: ContactId, x: f32, time: u32) -> PendingPress {
        PendingPress {
            contact: c,
            position: Point::new(x, 20.0),
            time,
        }
    }

    /// Shift on the left; `h` (base) or `H` (upper) on the right.
    fn shift_board() -> Fake {
        Fake::new(vec![
            [Act::View(1), Act::Letter('h')],
            [Act::View(0), Act::Letter('H')],
        ])
    }

    #[test]
    fn rolled_shift_then_letter_types_on_new_view() {
        let mut b = shift_board();
        press(&mut b, at(A, 20.0, 0));
        press(&mut b, at(B, 60.0, 10));
        assert_eq!(b.current, 1);
        release(&mut b, A, 20);
        release(&mut b, B, 30);
        assert_eq!(b.typed, "H");
    }

    #[test]
    fn rolled_shift_waits_for_new_views_layout() {
        let mut b = shift_board();
        b.laid[1] = false;
        press(&mut b, at(A, 20.0, 0));
        press(&mut b, at(B, 60.0, 10));
        assert_eq!(b.typed, "");
        b.laid[1] = true;
        release(&mut b, B, 30);
        assert_eq!(b.typed, "H");
    }

    #[test]
    fn rolled_letters_predict_from_earlier_letter() {
        let mut b = Fake::new(vec![[Act::Letter('q'), Act::Letter('h')]]);
        press(&mut b, at(A, 20.0, 0));
        press(&mut b, at(B, 60.0, 10));
        release(&mut b, A, 20);
        release(&mut b, B, 30);
        assert_eq!(b.typed, "qh");
        assert_eq!(b.predicted, vec!["".to_string(), "q".to_string()]);
    }

    #[test]
    fn early_press_commits_once_laid_out() {
        let mut b = shift_board();
        b.laid[0] = false;
        press(&mut b, at(A, 60.0, 0));
        b.laid[0] = true;
        release(&mut b, A, 10);
        assert_eq!(b.typed, "h");
    }

    #[test]
    fn never_laid_out_tap_types_nothing() {
        let mut b = shift_board();
        b.laid[0] = false;
        press(&mut b, at(A, 60.0, 0));
        release(&mut b, A, 10);
        assert_eq!(b.typed, "");
        b.laid[0] = true;
        press(&mut b, at(B, 20.0, 20));
        release(&mut b, B, 30);
        assert_eq!(b.typed, "", "the dropped tap never comes back");
    }

    #[test]
    fn held_back_press_then_second_finger_types_both_in_order() {
        let mut b = Fake::new(vec![[Act::Letter('q'), Act::Letter('h')]]);
        b.laid[0] = false;
        press(&mut b, at(A, 20.0, 0));
        b.laid[0] = true;
        press(&mut b, at(B, 60.0, 10));
        release(&mut b, B, 20);
        assert_eq!(b.typed, "qh");
        assert_eq!(b.predicted, vec!["".to_string(), "q".to_string()]);
    }
}
