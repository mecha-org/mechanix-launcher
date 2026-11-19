//! Press/move/release state machine for a single touch contact.
//!
//! Pure bookkeeping: callers feed it contact events (already resolved to keys)
//! and act on the returned [`TouchOut`] list.

use mecha_wayland::prelude::*;

use super::long_press::ChoicesRow;

/// A key under a contact: its index, whether holding it auto-repeats and
/// whether it has long-press choices (which rule out repeating).
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct KeyRef {
    pub index: usize,
    pub repeats: bool,
    pub choices: bool,
}

/// An action the caller should take in response to a contact event.
#[derive(PartialEq, Debug)]
pub(crate) enum TouchOut {
    /// Highlight the key.
    Pressed(usize),
    /// Remove the highlight from the key.
    Unpressed(usize),
    /// Type the key.
    Commit(usize),
    /// Start auto-repeating the key.
    Hold(usize),
    /// Stop the auto-repeat; the key was held for `held_ms`.
    StopHold { held_ms: u32 },
    /// The key was long-pressed: show its choices row.
    OpenChoices(usize),
    /// The finger moved the choices row's highlight.
    HighlightChoice(Option<usize>),
    /// Type choice `choice` of key `key`, and close the row.
    CommitChoice { key: usize, choice: usize },
    /// Close the choices row; nothing is typed.
    CloseChoices,
}

/// The contact currently being tracked.
struct Active {
    contact: ContactId,
    /// Key under the contact, if any.
    key: Option<usize>,
    /// Whether a hold is running on `key`.
    held: bool,
    /// Time of the press, for hold durations.
    press_time: u32,
    /// The choices row, once the key was long-pressed.
    choices: Option<ChoicesRow>,
}

/// Tracks one active contact; events from other contacts are ignored.
#[derive(Default)]
pub(crate) struct TouchTracker {
    active: Option<Active>,
}

impl TouchTracker {
    /// A contact went down. Any contact still active is finished as if
    /// released at `time`.
    pub(crate) fn press(&mut self, c: ContactId, key: Option<KeyRef>, time: u32) -> Vec<TouchOut> {
        let mut out = Vec::new();
        if let Some(active) = self.active.take() {
            Self::finish(active, true, time, &mut out);
        }
        let mut held = false;
        if let Some(k) = key {
            out.push(TouchOut::Pressed(k.index));
            if k.repeats && !k.choices {
                out.push(TouchOut::Hold(k.index));
                held = true;
            }
        }
        self.active = Some(Active {
            contact: c,
            key: key.map(|k| k.index),
            held,
            press_time: time,
            choices: None,
        });
        out
    }

    /// The active contact moved onto `key` (or onto no key) at `time`.
    pub(crate) fn moved(&mut self, c: ContactId, key: Option<KeyRef>, time: u32) -> Vec<TouchOut> {
        let mut out = Vec::new();
        let Some(active) = self
            .active
            .as_mut()
            .filter(|a| a.contact == c && a.choices.is_none())
        else {
            return out;
        };
        let new = key.map(|k| k.index);
        if active.key == new {
            return out;
        }
        if let Some(old) = active.key {
            if active.held {
                out.push(TouchOut::StopHold {
                    held_ms: time.wrapping_sub(active.press_time),
                });
                active.held = false;
            }
            out.push(TouchOut::Unpressed(old));
        }
        if let Some(i) = new {
            out.push(TouchOut::Pressed(i));
        }
        active.key = new;
        out
    }

    /// Contact `c` was held on its key long enough: open `row`, the key's
    /// choices. Ignored unless `c` is on a key that isn't holding and the row
    /// isn't open yet.
    pub(crate) fn long_pressed(&mut self, c: ContactId, row: ChoicesRow) -> Vec<TouchOut> {
        match self.active.as_mut() {
            Some(a) if a.contact == c && !a.held && a.choices.is_none() => match a.key {
                Some(i) => {
                    a.choices = Some(row);
                    vec![TouchOut::OpenChoices(i)]
                }
                None => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    /// Contact `c` moved to `p` while its choices row is open.
    pub(crate) fn track(&mut self, c: ContactId, p: Point) -> Vec<TouchOut> {
        let Some(row) = self
            .active
            .as_mut()
            .filter(|a| a.contact == c)
            .and_then(|a| a.choices.as_mut())
        else {
            return Vec::new();
        };
        let before = row.highlighted;
        row.track(p);
        if row.highlighted == before {
            Vec::new()
        } else {
            vec![TouchOut::HighlightChoice(row.highlighted)]
        }
    }

    /// Whether a choices row is open (gestures stand down while it is).
    pub(crate) fn choices_open(&self) -> bool {
        self.active.as_ref().is_some_and(|a| a.choices.is_some())
    }

    /// The open choices row, if any.
    pub(crate) fn choices(&self) -> Option<&ChoicesRow> {
        self.active.as_ref().and_then(|a| a.choices.as_ref())
    }

    /// The active contact and the key under it, if it is on one.
    pub(crate) fn pressed(&self) -> Option<(ContactId, usize)> {
        let a = self.active.as_ref()?;
        Some((a.contact, a.key?))
    }

    /// The active contact lifted at `time`.
    pub(crate) fn release(&mut self, c: ContactId, time: u32) -> Vec<TouchOut> {
        self.end(c, true, time)
    }

    /// The active contact was cancelled; nothing is typed.
    pub(crate) fn cancel(&mut self, c: ContactId, time: u32) -> Vec<TouchOut> {
        self.end(c, false, time)
    }

    /// Cancel whichever contact is active at `time` (its key went away, e.g.
    /// the view changed under it); nothing is typed.
    pub(crate) fn cancel_active(&mut self, time: u32) -> Vec<TouchOut> {
        let mut out = Vec::new();
        if let Some(active) = self.active.take() {
            Self::finish(active, false, time, &mut out);
        }
        out
    }

    /// Finish whichever contact is active at `time` as if it lifted (another
    /// finger came down); a tap commits.
    pub(crate) fn finish_active(&mut self, time: u32) -> Vec<TouchOut> {
        let mut out = Vec::new();
        if let Some(active) = self.active.take() {
            Self::finish(active, true, time, &mut out);
        }
        out
    }

    /// The contact being tracked, if any.
    pub(crate) fn contact(&self) -> Option<ContactId> {
        self.active.as_ref().map(|a| a.contact)
    }

    fn end(&mut self, c: ContactId, commit: bool, time: u32) -> Vec<TouchOut> {
        let mut out = Vec::new();
        if self.active.as_ref().is_some_and(|a| a.contact == c) {
            let active = self.active.take().unwrap();
            Self::finish(active, commit, time, &mut out);
        }
        out
    }

    /// Emit the closing actions for `active`.
    fn finish(active: Active, commit: bool, time: u32, out: &mut Vec<TouchOut>) {
        let Some(i) = active.key else { return };
        if let Some(row) = active.choices {
            match row.highlighted.filter(|_| commit) {
                Some(choice) => out.push(TouchOut::CommitChoice { key: i, choice }),
                None => out.push(TouchOut::CloseChoices),
            }
        } else if active.held {
            out.push(TouchOut::StopHold {
                held_ms: time.wrapping_sub(active.press_time),
            });
        } else if commit {
            out.push(TouchOut::Commit(i));
        }
        out.push(TouchOut::Unpressed(i));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: ContactId = ContactId::Touch(1);
    const B: ContactId = ContactId::Touch(2);

    fn key(index: usize) -> Option<KeyRef> {
        Some(KeyRef {
            index,
            repeats: false,
            choices: false,
        })
    }
    fn rep(index: usize) -> Option<KeyRef> {
        Some(KeyRef {
            index,
            repeats: true,
            choices: false,
        })
    }
    fn long(index: usize) -> Option<KeyRef> {
        Some(KeyRef {
            index,
            repeats: false,
            choices: true,
        })
    }

    /// A two-choice row over a 40x40 key at the origin.
    fn row() -> ChoicesRow {
        ChoicesRow::open(
            Rect::new(0.0, 100.0, 40.0, 40.0),
            Rect::new(0.0, 0.0, 400.0, 200.0),
            4.0,
            vec!["1".into(), "é".into()],
        )
    }

    #[test]
    fn keys_with_choices_do_not_repeat() {
        let mut t = TouchTracker::default();
        let k = Some(KeyRef {
            index: 7,
            repeats: true,
            choices: true,
        });
        assert_eq!(t.press(A, k, 0), vec![TouchOut::Pressed(7)]);
        assert_eq!(
            t.release(A, 500),
            vec![TouchOut::Commit(7), TouchOut::Unpressed(7)]
        );
    }

    #[test]
    fn long_press_opens_row_and_slide_moves_highlight() {
        let mut t = TouchTracker::default();
        t.press(A, long(2), 0);
        assert!(!t.choices_open());
        assert_eq!(t.pressed(), Some((A, 2)));
        assert_eq!(t.long_pressed(A, row()), vec![TouchOut::OpenChoices(2)]);
        assert!(t.choices_open());
        assert_eq!(t.long_pressed(A, row()), vec![], "opens once");
        // Key changes no longer apply; the row tracks the finger.
        assert_eq!(t.moved(A, long(3), 10), vec![]);
        assert_eq!(t.track(A, Point::new(10.0, 70.0)), vec![]);
        assert_eq!(
            t.track(A, Point::new(50.0, 70.0)),
            vec![TouchOut::HighlightChoice(Some(1))]
        );
        assert_eq!(
            t.release(A, 400),
            vec![
                TouchOut::CommitChoice { key: 2, choice: 1 },
                TouchOut::Unpressed(2)
            ]
        );
        assert!(!t.choices_open());
    }

    #[test]
    fn release_off_the_row_or_cancel_closes_it() {
        let mut t = TouchTracker::default();
        t.press(A, long(2), 0);
        t.long_pressed(A, row());
        assert_eq!(
            t.track(A, Point::new(10.0, 200.0)),
            vec![TouchOut::HighlightChoice(None)]
        );
        assert_eq!(
            t.release(A, 400),
            vec![TouchOut::CloseChoices, TouchOut::Unpressed(2)]
        );
        t.press(A, long(2), 500);
        t.long_pressed(A, row());
        assert_eq!(
            t.cancel(A, 900),
            vec![TouchOut::CloseChoices, TouchOut::Unpressed(2)]
        );
    }

    #[test]
    fn long_press_ignores_other_contacts_and_held_keys() {
        let mut t = TouchTracker::default();
        assert_eq!(t.long_pressed(A, row()), vec![]);
        t.press(A, long(2), 0);
        assert_eq!(t.long_pressed(B, row()), vec![]);
        t.press(A, None, 0);
        assert_eq!(t.long_pressed(A, row()), vec![]);
        t.press(A, rep(5), 0);
        assert_eq!(t.long_pressed(A, row()), vec![]);
    }

    #[test]
    fn tap_commits_on_release() {
        let mut t = TouchTracker::default();
        assert_eq!(t.press(A, key(3), 0), vec![TouchOut::Pressed(3)]);
        assert_eq!(
            t.release(A, 50),
            vec![TouchOut::Commit(3), TouchOut::Unpressed(3)]
        );
    }

    #[test]
    fn slide_moves_highlight_and_commits_last_key() {
        let mut t = TouchTracker::default();
        t.press(A, key(3), 0);
        assert_eq!(t.moved(A, key(3), 10), vec![]);
        assert_eq!(
            t.moved(A, key(4), 20),
            vec![TouchOut::Unpressed(3), TouchOut::Pressed(4)]
        );
        assert_eq!(
            t.release(A, 30),
            vec![TouchOut::Commit(4), TouchOut::Unpressed(4)]
        );
    }

    #[test]
    fn repeat_key_holds_and_release_does_not_commit() {
        let mut t = TouchTracker::default();
        assert_eq!(
            t.press(A, rep(7), 1000),
            vec![TouchOut::Pressed(7), TouchOut::Hold(7)]
        );
        assert_eq!(
            t.release(A, 1100),
            vec![TouchOut::StopHold { held_ms: 100 }, TouchOut::Unpressed(7)]
        );
    }

    #[test]
    fn slide_off_held_key_stops_hold() {
        let mut t = TouchTracker::default();
        t.press(A, rep(7), 0);
        assert_eq!(
            t.moved(A, key(8), 40),
            vec![
                TouchOut::StopHold { held_ms: 40 },
                TouchOut::Unpressed(7),
                TouchOut::Pressed(8)
            ]
        );
    }

    #[test]
    fn slide_onto_repeat_key_does_not_hold_but_release_commits() {
        let mut t = TouchTracker::default();
        t.press(A, key(1), 0);
        assert_eq!(
            t.moved(A, rep(7), 10),
            vec![TouchOut::Unpressed(1), TouchOut::Pressed(7)]
        );
        assert_eq!(
            t.release(A, 20),
            vec![TouchOut::Commit(7), TouchOut::Unpressed(7)]
        );
    }

    #[test]
    fn cancel_stops_hold_and_types_nothing() {
        let mut t = TouchTracker::default();
        t.press(A, rep(7), 0);
        assert_eq!(
            t.cancel(A, 90),
            vec![TouchOut::StopHold { held_ms: 90 }, TouchOut::Unpressed(7)]
        );
        t.press(A, key(2), 100);
        assert_eq!(t.cancel(A, 110), vec![TouchOut::Unpressed(2)]);
    }

    #[test]
    fn second_contact_commits_first_then_presses() {
        let mut t = TouchTracker::default();
        t.press(A, key(1), 0);
        assert_eq!(
            t.press(B, key(2), 10),
            vec![
                TouchOut::Commit(1),
                TouchOut::Unpressed(1),
                TouchOut::Pressed(2)
            ]
        );
        assert_eq!(
            t.release(B, 20),
            vec![TouchOut::Commit(2), TouchOut::Unpressed(2)]
        );
    }

    #[test]
    fn foreign_contact_release_is_ignored() {
        let mut t = TouchTracker::default();
        t.press(A, key(1), 0);
        assert_eq!(t.release(B, 5), vec![]);
        assert_eq!(t.moved(B, key(2), 6), vec![]);
        assert_eq!(t.cancel(B, 7), vec![]);
        assert_eq!(
            t.release(A, 8),
            vec![TouchOut::Commit(1), TouchOut::Unpressed(1)]
        );
    }

    #[test]
    fn stop_hold_reports_held_time() {
        let mut t = TouchTracker::default();
        t.press(A, rep(7), 1000);
        assert_eq!(
            t.release(A, 1600),
            vec![TouchOut::StopHold { held_ms: 600 }, TouchOut::Unpressed(7)]
        );
    }

    #[test]
    fn cancel_active_finishes_without_commit() {
        let mut t = TouchTracker::default();
        assert_eq!(t.cancel_active(5), vec![]);
        t.press(A, key(2), 0);
        assert_eq!(t.cancel_active(10), vec![TouchOut::Unpressed(2)]);
        assert_eq!(t.release(A, 20), vec![]);
        t.press(B, rep(7), 100);
        assert_eq!(
            t.cancel_active(250),
            vec![TouchOut::StopHold { held_ms: 150 }, TouchOut::Unpressed(7)]
        );
        assert!(t.contact().is_none());
    }

    #[test]
    fn finish_active_ends_like_release() {
        let mut t = TouchTracker::default();
        assert_eq!(t.finish_active(5), vec![]);
        t.press(A, key(2), 0);
        assert_eq!(
            t.finish_active(10),
            vec![TouchOut::Commit(2), TouchOut::Unpressed(2)]
        );
        assert_eq!(t.release(A, 20), vec![]);
        t.press(B, rep(7), 100);
        assert_eq!(
            t.finish_active(250),
            vec![TouchOut::StopHold { held_ms: 150 }, TouchOut::Unpressed(7)]
        );
    }

    #[test]
    fn contact_reports_active_contact() {
        let mut t = TouchTracker::default();
        assert_eq!(t.contact(), None);
        t.press(A, None, 0);
        assert_eq!(t.contact(), Some(A));
        t.release(A, 1);
        assert_eq!(t.contact(), None);
    }

    #[test]
    fn no_key_press_then_slide_onto_key() {
        let mut t = TouchTracker::default();
        assert_eq!(t.press(A, None, 0), vec![]);
        assert_eq!(t.moved(A, rep(5), 10), vec![TouchOut::Pressed(5)]);
        assert_eq!(
            t.release(A, 20),
            vec![TouchOut::Commit(5), TouchOut::Unpressed(5)]
        );
        t.press(A, None, 30);
        assert_eq!(t.release(A, 40), vec![]);
    }
}
