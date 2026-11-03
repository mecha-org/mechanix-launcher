//! Press/move/release state machine for a single touch contact.
//!
//! Pure bookkeeping: callers feed it contact events (already resolved to keys)
//! and act on the returned [`TouchOut`] list.

use mecha_wayland::prelude::*;

/// A key under a contact: its index and whether holding it auto-repeats.
#[derive(Clone, Copy, PartialEq, Debug)]
pub(crate) struct KeyRef {
    pub index: usize,
    pub repeats: bool,
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
            if k.repeats {
                out.push(TouchOut::Hold(k.index));
                held = true;
            }
        }
        self.active = Some(Active { contact: c, key: key.map(|k| k.index), held, press_time: time });
        out
    }

    /// The active contact moved onto `key` (or onto no key) at `time`.
    pub(crate) fn moved(&mut self, c: ContactId, key: Option<KeyRef>, time: u32) -> Vec<TouchOut> {
        let mut out = Vec::new();
        let Some(active) = self.active.as_mut().filter(|a| a.contact == c) else {
            return out;
        };
        let new = key.map(|k| k.index);
        if active.key == new {
            return out;
        }
        if let Some(old) = active.key {
            if active.held {
                out.push(TouchOut::StopHold { held_ms: time.wrapping_sub(active.press_time) });
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

    /// The active contact lifted at `time`.
    pub(crate) fn release(&mut self, c: ContactId, time: u32) -> Vec<TouchOut> {
        self.end(c, true, time)
    }

    /// The active contact was cancelled; nothing is typed.
    pub(crate) fn cancel(&mut self, c: ContactId, time: u32) -> Vec<TouchOut> {
        self.end(c, false, time)
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
        if active.held {
            out.push(TouchOut::StopHold { held_ms: time.wrapping_sub(active.press_time) });
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
        Some(KeyRef { index, repeats: false })
    }
    fn rep(index: usize) -> Option<KeyRef> {
        Some(KeyRef { index, repeats: true })
    }

    #[test]
    fn tap_commits_on_release() {
        let mut t = TouchTracker::default();
        assert_eq!(t.press(A, key(3), 0), vec![TouchOut::Pressed(3)]);
        assert_eq!(t.release(A, 50), vec![TouchOut::Commit(3), TouchOut::Unpressed(3)]);
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
        assert_eq!(t.release(A, 30), vec![TouchOut::Commit(4), TouchOut::Unpressed(4)]);
    }

    #[test]
    fn repeat_key_holds_and_release_does_not_commit() {
        let mut t = TouchTracker::default();
        assert_eq!(t.press(A, rep(7), 1000), vec![TouchOut::Pressed(7), TouchOut::Hold(7)]);
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
        assert_eq!(t.moved(A, rep(7), 10), vec![TouchOut::Unpressed(1), TouchOut::Pressed(7)]);
        assert_eq!(t.release(A, 20), vec![TouchOut::Commit(7), TouchOut::Unpressed(7)]);
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
            vec![TouchOut::Commit(1), TouchOut::Unpressed(1), TouchOut::Pressed(2)]
        );
        assert_eq!(t.release(B, 20), vec![TouchOut::Commit(2), TouchOut::Unpressed(2)]);
    }

    #[test]
    fn foreign_contact_release_is_ignored() {
        let mut t = TouchTracker::default();
        t.press(A, key(1), 0);
        assert_eq!(t.release(B, 5), vec![]);
        assert_eq!(t.moved(B, key(2), 6), vec![]);
        assert_eq!(t.cancel(B, 7), vec![]);
        assert_eq!(t.release(A, 8), vec![TouchOut::Commit(1), TouchOut::Unpressed(1)]);
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
    fn no_key_press_then_slide_onto_key() {
        let mut t = TouchTracker::default();
        assert_eq!(t.press(A, None, 0), vec![]);
        assert_eq!(t.moved(A, rep(5), 10), vec![TouchOut::Pressed(5)]);
        assert_eq!(t.release(A, 20), vec![TouchOut::Commit(5), TouchOut::Unpressed(5)]);
        t.press(A, None, 30);
        assert_eq!(t.release(A, 40), vec![]);
    }
}
