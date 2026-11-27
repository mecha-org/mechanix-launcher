//! The order of operations for each contact event, over an abstract
//! [`Board`] so it can be tested without widgets: hold a press back until its
//! view is laid out ([`Deferral`]), finish an earlier finger before a new one
//! is resolved, resolve against the current view with the prior taken at the
//! press, and hand the tracker's outputs to the board. A fast downward
//! [`Flick`] closes the keyboard instead of typing, and a sideways swipe from
//! a gliding key (the space bar) starts a [`Glide`](super::glide::Glide) that
//! moves the caret instead.

use mecha_wayland::prelude::*;

use super::defer::{Contact, Deferral, PendingPress, laid_out};
use super::flick::Flick;
use super::glide::GlideSpec;
use super::long_press::ChoicesRow;
use super::pointing_stick::PointingStickPad;
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
    /// The tracked contact's motion, watched for a closing flick.
    flick: Option<(ContactId, Flick)>,
    /// Where the tracked contact pressed, or slid onto the key it is on:
    /// the pointing stick measures its dead zone and directions from here,
    /// and a glide its threshold and first arrow.
    origin: Point,
    /// The event time at `origin`.
    origin_time: u32,
}

impl Default for Flow {
    fn default() -> Self {
        Self {
            deferral: Deferral::default(),
            prior: FLAT,
            strength: 0.0,
            flick: None,
            origin: Point::default(),
            origin_time: 0,
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
    /// Whether key `index` of the current view has long-press choices.
    fn has_choices(&mut self, index: usize) -> bool;
    /// The choices row key `index` of the current view opens, if it has one.
    fn choices_row(&mut self, index: usize) -> Option<ChoicesRow>;
    /// Whether key `index` of the current view is the pointing stick.
    fn is_pointing_stick(&mut self, index: usize) -> bool;
    /// The direction pad the pointing stick at key `index` of the current
    /// view opens, measuring directions from `origin`.
    fn pointing_stick_pad(&mut self, index: usize, origin: Point) -> Option<PointingStickPad>;
    /// How key `index` of the current view glides the caret when swiped
    /// from, if it does.
    fn glide(&mut self, index: usize) -> Option<GlideSpec>;
    fn tracker(&mut self) -> &mut TouchTracker;
    fn flow(&mut self) -> &mut Flow;
    /// Act on tracker outputs (indices into the current view), then cancel
    /// the active contact if acting changed the view.
    fn act(&mut self, out: Vec<TouchOut>, time: u32);
    /// Hide the keyboard (a closing flick).
    fn close(&mut self);
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

/// A contact moved to `position`. While its choices row, direction pad or
/// glide is open, that tracks it instead of the keys (and no flick is
/// watched: none closes over an overlay). On the pointing stick the finger
/// stays put inside the dead zone, and pushing past it opens the direction
/// pad at once, without waiting for the hold: a quick push is arrow input,
/// never a slide onto the keys around or a closing flick. On a gliding key,
/// a swipe past its threshold starts the glide.
pub(crate) fn moved(b: &mut impl Board, c: ContactId, position: Point, time: u32) {
    if gate(b, Contact::Moved(c)) {
        if b.tracker().overlay_open() {
            let out = b.tracker().track(c, position, time);
            b.act(out, time);
            return;
        }
        if let Some((_, f)) = b.flow().flick.as_mut().filter(|(fc, _)| *fc == c) {
            f.sample(position, time);
        }
        let pressed = b.tracker().pressed().filter(|(pc, _)| *pc == c);
        if let Some((_, key)) = pressed {
            if b.is_pointing_stick(key) {
                push_pointing_stick(b, c, key, position, time);
                return;
            }
            if start_glide(b, c, key, position, time) {
                return;
            }
        }
        let geoms = b.geoms();
        let key = key_at(b, &geoms, position);
        let out = b.tracker().moved(c, key, time);
        b.act(out, time);
        let now = b.tracker().pressed().filter(|(pc, _)| *pc == c);
        if now.map(|(_, k)| k) != pressed.map(|(_, k)| k) {
            let flow = b.flow();
            flow.origin = position;
            flow.origin_time = time;
        }
    }
}

/// Contact `c`, on the pointing stick at key `key` with no pad open yet,
/// moved to `position`: past the dead zone around its origin, open the pad
/// and hold the direction it points in; inside it, stay on the stick.
fn push_pointing_stick(b: &mut impl Board, c: ContactId, key: usize, position: Point, time: u32) {
    let origin = b.flow().origin;
    let Some(pad) = b.pointing_stick_pad(key, origin) else {
        return;
    };
    let (dx, dy) = (position.x - origin.x, position.y - origin.y);
    if (dx * dx + dy * dy).sqrt() < pad.dead_zone {
        return;
    }
    let out = b.tracker().pointing_stick_held(c, pad);
    b.act(out, time);
    let out = b.tracker().track(c, position, time);
    b.act(out, time);
}

/// Contact `c`, on key `key` with nothing open, moved to `position`: if the
/// key glides and the swipe from its origin passed the threshold, open the
/// glide (travel so far counts toward the first arrow) and whether it did.
fn start_glide(b: &mut impl Board, c: ContactId, key: usize, position: Point, time: u32) -> bool {
    let Some(spec) = b.glide(key) else {
        return false;
    };
    let flow = b.flow();
    let (origin, since) = (flow.origin, flow.origin_time);
    if !spec.starts(position.x - origin.x, position.y - origin.y) {
        return false;
    }
    flow.flick = None;
    let out = b.tracker().glide(c, spec.open(origin, since));
    b.act(out, time);
    let out = b.tracker().track(c, position, time);
    b.act(out, time);
    true
}

/// A contact lifted. The commit uses the last resolved key (the highlighted
/// one), not a fresh resolve at the release position, by design. A closing
/// flick (never while a choices row or direction pad is open) types nothing: the contact is
/// cancelled, which also closes its row, ends its hold and its long-press
/// wait, and the keyboard hides.
pub(crate) fn release(b: &mut impl Board, c: ContactId, time: u32) {
    if gate(b, Contact::Release(c)) {
        let flow = b.flow();
        let flick = flow.flick.take_if(|(fc, _)| *fc == c);
        let closing = flick.is_some_and(|(_, mut f)| {
            f.end(time);
            f.is_close()
        }) && b.tracker().contact() == Some(c)
            && !b.tracker().overlay_open();
        if closing {
            let out = b.tracker().cancel(c, time);
            b.act(out, time);
            b.close();
            return;
        }
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

/// The keyboard was refocused or hidden under the contact: cancel it and
/// drop any held-back press. Nothing is typed; its row closes, its hold and
/// long-press wait end.
pub(crate) fn abort(b: &mut impl Board, time: u32) {
    let flow = b.flow();
    flow.deferral = Deferral::default();
    flow.flick = None;
    let out = b.tracker().cancel_active(time);
    b.act(out, time);
}

/// The long-press timer fired at `time` for the key under the active
/// contact: open its choices row, or the pointing stick's direction pad.
pub(crate) fn long_pressed(b: &mut impl Board, time: u32) {
    let Some((c, key)) = b.tracker().pressed() else {
        return;
    };
    let out = if b.is_pointing_stick(key) {
        let origin = b.flow().origin;
        let Some(pad) = b.pointing_stick_pad(key, origin) else {
            return;
        };
        b.tracker().pointing_stick_held(c, pad)
    } else {
        let Some(row) = b.choices_row(key) else {
            return;
        };
        b.tracker().long_pressed(c, row)
    };
    b.act(out, time);
}

/// Pass a non-press `event` through the deferral, starting a held-back press
/// it releases; whether to act on the event itself.
fn gate(b: &mut impl Board, event: Contact) -> bool {
    // Measuring the layout walks every key: only a held-back press needs it.
    let ready = !b.flow().deferral.waiting() || laid_out(&b.geoms());
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
    flow.flick = Some((p.contact, Flick::start(p.position, p.time)));
    flow.origin = p.position;
    flow.origin_time = p.time;
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
    let choices = b.has_choices(index);
    Some(KeyRef {
        index,
        repeats,
        choices,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::touch::pointing_stick::Direction;

    const A: ContactId = ContactId::Touch(1);
    const B: ContactId = ContactId::Touch(2);

    /// What a fake key does when committed.
    #[derive(Clone, Copy)]
    enum Act {
        Letter(char),
        View(usize),
        /// The pointing stick: a tap toggles select mode.
        Stick,
        /// The space bar: types a space, glides when swiped.
        Space,
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
        /// Long-press choices per key index, in every view.
        choices: [Vec<&'static str>; 2],
        /// The key the long-press timer runs for, as the glue starts and
        /// cancels it.
        timer: Option<usize>,
        /// Every timer start: key and time.
        timer_starts: Vec<(usize, u32)>,
        /// Whether the choices row is shown.
        row_shown: bool,
        /// The key index that repeats while held, in every view.
        repeat: Option<usize>,
        /// Whether a hold is running.
        holding: bool,
        /// Whether the keyboard was closed.
        closed: bool,
        /// Whether the pointing stick is in select mode.
        select: bool,
        /// Whether the direction pad is shown.
        pad_shown: bool,
        /// Every direction the pointing stick lit, in order.
        directions: Vec<Option<Direction>>,
        /// Whether a glide opened.
        gliding: bool,
        /// The caret's position, moved by glide arrows (right positive).
        caret: i32,
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
                choices: [Vec::new(), Vec::new()],
                timer: None,
                timer_starts: Vec::new(),
                row_shown: false,
                repeat: None,
                holding: false,
                closed: false,
                select: false,
                pad_shown: false,
                directions: Vec::new(),
                gliding: false,
                caret: 0,
            }
        }

        fn apply(&mut self, out: Vec<TouchOut>, time: u32) {
            for o in out {
                match o {
                    TouchOut::Pressed(i)
                        if !self.choices[i].is_empty()
                            || matches!(self.views[self.current][i], Act::Stick) =>
                    {
                        self.timer = Some(i);
                        self.timer_starts.push((i, time));
                    }
                    TouchOut::Unpressed(_) => self.timer = None,
                    TouchOut::Commit(i) => match self.views[self.current][i] {
                        Act::Letter(c) => self.typed.push(c),
                        Act::View(v) => self.current = v,
                        Act::Stick => self.select = !self.select,
                        Act::Space => self.typed.push(' '),
                    },
                    TouchOut::OpenChoices(_) => self.row_shown = true,
                    TouchOut::CommitChoice { key, choice } => {
                        self.row_shown = false;
                        self.typed.push_str(self.choices[key][choice]);
                    }
                    TouchOut::CloseChoices => self.row_shown = false,
                    TouchOut::Hold(_) => self.holding = true,
                    TouchOut::StopHold { .. } => self.holding = false,
                    TouchOut::OpenPointingStick(_) => self.pad_shown = true,
                    TouchOut::PointingStick(d) => {
                        self.holding = d.is_some();
                        self.directions.push(d);
                    }
                    TouchOut::ClosePointingStick => self.pad_shown = false,
                    TouchOut::OpenGlide(_) => self.gliding = true,
                    TouchOut::Glide(s) => self.caret += s.x,
                    _ => {}
                }
            }
        }

        /// The long-press timer fires, if running.
        fn fire(&mut self, time: u32) {
            if self.timer.take().is_some() {
                long_pressed(self, time);
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
                        Act::View(_) | Act::Stick | Act::Space => None,
                    },
                })
                .collect()
        }
        fn predict(&mut self) -> (LetterPrior, f32) {
            self.predicted.push(self.typed.clone());
            (FLAT, 0.0)
        }
        fn repeats(&mut self, index: usize) -> bool {
            self.repeat == Some(index)
        }
        fn has_choices(&mut self, index: usize) -> bool {
            !self.choices[index].is_empty()
        }
        fn choices_row(&mut self, index: usize) -> Option<ChoicesRow> {
            let list: Vec<String> = self.choices[index].iter().map(|c| c.to_string()).collect();
            let key = self.geoms()[index].rect;
            (!list.is_empty())
                .then(|| ChoicesRow::open(key, Rect::new(0.0, -60.0, 400.0, 100.0), 4.0, list, 0))
        }
        fn is_pointing_stick(&mut self, index: usize) -> bool {
            matches!(self.views[self.current][index], Act::Stick)
        }
        fn pointing_stick_pad(&mut self, index: usize, origin: Point) -> Option<PointingStickPad> {
            let key = self.geoms()[index].rect;
            Some(PointingStickPad::open(
                key,
                Rect::new(0.0, -60.0, 400.0, 200.0),
                4.0,
                origin,
            ))
        }
        fn glide(&mut self, index: usize) -> Option<GlideSpec> {
            matches!(self.views[self.current][index], Act::Space)
                .then(|| GlideSpec::horizontal(40.0))
        }
        fn tracker(&mut self) -> &mut TouchTracker {
            &mut self.tracker
        }
        fn flow(&mut self) -> &mut Flow {
            &mut self.flow
        }
        fn act(&mut self, out: Vec<TouchOut>, time: u32) {
            let view = self.current;
            self.apply(out, time);
            if self.current != view {
                let out = self.tracker.cancel_active(time);
                self.apply(out, time);
            }
        }
        fn close(&mut self) {
            self.closed = true;
        }
    }

    fn at(c: ContactId, x: f32, time: u32) -> PendingPress {
        PendingPress {
            contact: c,
            position: Point::new(x, 20.0),
            time,
        }
    }

    /// A view switch on the left; `h` (view 0) or `H` (view 1) on the right.
    fn shift_board() -> Fake {
        Fake::new(vec![
            [Act::View(1), Act::Letter('h')],
            [Act::View(0), Act::Letter('H')],
        ])
    }

    /// `q` and `h`, with `h` long-pressing to `4 ĥ` and `q` to `1`.
    fn choices_board() -> Fake {
        let mut b = Fake::new(vec![[Act::Letter('q'), Act::Letter('h')]]);
        b.choices = [vec!["1"], vec!["4", "ĥ"]];
        b
    }

    /// Contact `c`, pressed at `x`, moves straight down by `dy` over `ms`
    /// (a move every 8 ms from time `t`) and lifts.
    fn swipe_down(b: &mut Fake, c: ContactId, x: f32, t: u32, dy: f32, ms: u32) {
        press(b, at(c, x, t));
        let steps = ms / 8;
        for i in 1..=steps {
            let y = 20.0 + dy * i as f32 / steps as f32;
            moved(b, c, Point::new(x, y), t + i * 8);
        }
        release(b, c, t + steps * 8);
    }

    #[test]
    fn flick_types_nothing() {
        let mut b = choices_board();
        swipe_down(&mut b, A, 60.0, 0, 150.0, 80);
        assert!(b.closed);
        assert_eq!(b.typed, "");
        assert_eq!(b.timer, None, "the long-press wait is cancelled");
        assert_eq!(b.tracker.contact(), None);
    }

    #[test]
    fn slow_drag_down_does_not_close() {
        let mut b = Fake::new(vec![[Act::Letter('q'), Act::Letter('h')]]);
        swipe_down(&mut b, A, 60.0, 0, 150.0, 600);
        assert!(!b.closed);
    }

    #[test]
    fn flick_from_held_key_stops_the_hold() {
        let mut b = Fake::new(vec![[Act::Letter('q'), Act::Letter('h')]]);
        b.repeat = Some(1);
        press(&mut b, at(A, 60.0, 0));
        assert!(b.holding);
        for i in 1..=10u32 {
            moved(&mut b, A, Point::new(60.0, 20.0 + 15.0 * i as f32), i * 8);
        }
        release(&mut b, A, 80);
        assert!(b.closed);
        assert!(!b.holding, "no stuck repeat");
    }

    #[test]
    fn earlier_finger_lifting_keeps_the_later_flick() {
        let mut b = Fake::new(vec![[Act::Letter('q'), Act::Letter('h')]]);
        press(&mut b, at(A, 20.0, 0));
        press(&mut b, at(B, 60.0, 10));
        release(&mut b, A, 12);
        for i in 1..=10u32 {
            moved(
                &mut b,
                B,
                Point::new(60.0, 20.0 + 15.0 * i as f32),
                10 + i * 8,
            );
        }
        release(&mut b, B, 90);
        assert!(b.closed);
        assert_eq!(b.typed, "q");
    }

    /// `q` and the pointing stick.
    fn stick_board() -> Fake {
        Fake::new(vec![[Act::Letter('q'), Act::Stick]])
    }

    #[test]
    fn pointing_stick_tap_toggles_mode_and_starts_its_timer() {
        let mut b = stick_board();
        press(&mut b, at(A, 60.0, 0));
        assert_eq!(b.timer, Some(1));
        release(&mut b, A, 100);
        assert_eq!(b.timer, None);
        assert!(b.select);
        assert!(!b.pad_shown);
        press(&mut b, at(A, 60.0, 200));
        release(&mut b, A, 250);
        assert!(!b.select);
        assert_eq!(b.typed, "");
    }

    #[test]
    fn pointing_stick_hold_measures_from_the_press() {
        let mut b = stick_board();
        // Pressed left of the key's centre, drifting a little (inside the
        // dead zone) before the timer fires: the pad measures from the press.
        press(&mut b, at(A, 50.0, 0));
        moved(&mut b, A, Point::new(52.0, 22.0), 50);
        assert!(!b.pad_shown);
        b.fire(150);
        assert!(b.pad_shown);
        assert_eq!(
            b.tracker.pointing_stick().unwrap().origin,
            Point::new(50.0, 20.0)
        );
        // Moving back to the press point is inside the dead zone.
        moved(&mut b, A, Point::new(50.0, 20.0), 160);
        assert_eq!(b.directions, vec![]);
        moved(&mut b, A, Point::new(52.0, 60.0), 200);
        assert!(b.holding);
        moved(&mut b, A, Point::new(10.0, 22.0), 300);
        assert_eq!(
            b.directions,
            vec![Some(Direction::Down), Some(Direction::Left)]
        );
        release(&mut b, A, 400);
        assert!(!b.holding);
        assert!(!b.pad_shown);
        assert!(!b.select, "a hold doesn't toggle the mode");
        assert_eq!(b.typed, "");
    }

    #[test]
    fn quick_push_down_from_the_stick_holds_down_and_never_closes() {
        let mut b = stick_board();
        // Straight down off the stick, fast, lifted before the hold time:
        // a closing flick anywhere else.
        swipe_down(&mut b, A, 60.0, 0, 150.0, 80);
        assert_eq!(b.directions, vec![Some(Direction::Down)]);
        assert!(!b.closed);
        assert!(!b.pad_shown);
        assert!(!b.holding);
        assert!(!b.select, "a push doesn't toggle the mode");
        assert_eq!(b.typed, "");
    }

    #[test]
    fn stick_holds_the_finger_inside_the_dead_zone() {
        let mut b = stick_board();
        press(&mut b, at(A, 75.0, 0));
        // 10 px right: past the key's edge (80), inside the 14 px dead zone.
        moved(&mut b, A, Point::new(85.0, 20.0), 50);
        assert_eq!(b.tracker.pressed(), Some((A, 1)));
        assert!(!b.pad_shown);
        assert_eq!(b.timer, Some(1), "the hold wait keeps running");
        release(&mut b, A, 100);
        assert!(b.select, "still a tap");
    }

    #[test]
    fn sliding_onto_the_stick_measures_from_where_it_arrived() {
        let mut b = stick_board();
        press(&mut b, at(A, 20.0, 0));
        moved(&mut b, A, Point::new(45.0, 20.0), 50);
        assert_eq!(b.tracker.pressed(), Some((A, 1)));
        assert!(!b.pad_shown, "arriving far from the press opens nothing");
        moved(&mut b, A, Point::new(50.0, 20.0), 60);
        assert!(!b.pad_shown);
        moved(&mut b, A, Point::new(45.0, 60.0), 70);
        assert!(b.pad_shown);
        assert_eq!(b.directions, vec![Some(Direction::Down)]);
    }

    #[test]
    fn flick_ignored_while_pointing_stick_pad_open() {
        let mut b = stick_board();
        press(&mut b, at(A, 60.0, 0));
        b.fire(150);
        for i in 1..=10u32 {
            moved(
                &mut b,
                A,
                Point::new(60.0, 20.0 + 15.0 * i as f32),
                150 + i * 8,
            );
        }
        release(&mut b, A, 230);
        assert!(!b.closed);
        assert!(!b.pad_shown);
        assert!(!b.holding);
    }

    /// `q` and the space bar (x 40 to 80).
    fn space_board() -> Fake {
        Fake::new(vec![[Act::Letter('q'), Act::Space]])
    }

    /// Contact `c`, pressed at `x`, moves straight sideways by `dx` over `ms`
    /// (a move every 8 ms from time `t`) and lifts.
    fn swipe_x(b: &mut Fake, c: ContactId, x: f32, t: u32, dx: f32, ms: u32) {
        press(b, at(c, x, t));
        let steps = ms / 8;
        for i in 1..=steps {
            let px = x + dx * i as f32 / steps as f32;
            moved(b, c, Point::new(px, 20.0), t + i * 8);
        }
        release(b, c, t + steps * 8);
    }

    #[test]
    fn space_tap_and_jitter_type_a_space() {
        let mut b = space_board();
        press(&mut b, at(A, 60.0, 0));
        moved(&mut b, A, Point::new(66.0, 23.0), 30);
        release(&mut b, A, 60);
        assert_eq!(b.typed, " ");
        assert!(!b.gliding);
        assert_eq!(b.caret, 0);
    }

    #[test]
    fn space_swipe_glides_across_the_whole_keyboard_and_types_nothing() {
        let mut b = space_board();
        // Slowly left, over `q` and past the keyboard's left edge.
        swipe_x(&mut b, A, 60.0, 0, -240.0, 1600);
        assert!(b.gliding);
        assert_eq!(b.typed, "", "no space, no `q`");
        // 240 px at 24 px an arrow.
        assert!((-10..=-9).contains(&b.caret), "{}", b.caret);
        assert_eq!(b.tracker.contact(), None);
        assert!(!b.closed);
    }

    #[test]
    fn faster_space_swipe_moves_further() {
        let mut slow = space_board();
        swipe_x(&mut slow, A, 60.0, 0, -50.0, 400);
        let mut fast = space_board();
        swipe_x(&mut fast, A, 60.0, 0, -50.0, 40);
        assert!((-2..=-1).contains(&slow.caret), "{}", slow.caret);
        assert!(fast.caret <= -3, "{}", fast.caret);
    }

    #[test]
    fn glide_then_downward_flick_does_not_close() {
        let mut b = space_board();
        press(&mut b, at(A, 60.0, 0));
        moved(&mut b, A, Point::new(90.0, 20.0), 50);
        assert!(b.gliding);
        for i in 1..=10u32 {
            moved(
                &mut b,
                A,
                Point::new(90.0, 20.0 + 15.0 * i as f32),
                50 + i * 8,
            );
        }
        release(&mut b, A, 130);
        assert!(!b.closed);
        assert_eq!(b.typed, "");
    }

    #[test]
    fn downward_flick_from_space_still_closes() {
        let mut b = space_board();
        swipe_down(&mut b, A, 60.0, 0, 150.0, 80);
        assert!(b.closed);
        assert!(!b.gliding);
        assert_eq!(b.typed, "");
    }

    #[test]
    fn flick_ignored_while_choices_open() {
        let mut b = choices_board();
        press(&mut b, at(A, 60.0, 0));
        b.fire(300);
        assert!(b.row_shown);
        for i in 1..=10u32 {
            moved(
                &mut b,
                A,
                Point::new(60.0, 20.0 + 15.0 * i as f32),
                300 + i * 8,
            );
        }
        release(&mut b, A, 380);
        assert!(!b.closed);
        assert!(!b.row_shown);
    }

    #[test]
    fn release_before_timeout_types_the_key() {
        let mut b = choices_board();
        press(&mut b, at(A, 60.0, 0));
        assert_eq!(b.timer, Some(1));
        release(&mut b, A, 200);
        assert_eq!(b.timer, None, "release cancels the timer");
        assert_eq!(b.typed, "h");
        assert!(!b.row_shown);
    }

    #[test]
    fn timeout_opens_choices_and_release_commits_highlighted() {
        let mut b = choices_board();
        press(&mut b, at(A, 60.0, 0));
        b.fire(300);
        assert!(b.row_shown);
        assert!(b.tracker.overlay_open());
        release(&mut b, A, 400);
        assert_eq!(b.typed, "4");
        assert!(!b.row_shown);
        // Slide to the second cell (the row runs right from x = 40).
        press(&mut b, at(A, 60.0, 500));
        b.fire(800);
        moved(&mut b, A, Point::new(100.0, -20.0), 850);
        release(&mut b, A, 900);
        assert_eq!(b.typed, "4ĥ");
    }

    #[test]
    fn slide_to_neighbour_before_timeout_switches_key_and_restarts_timer() {
        let mut b = choices_board();
        press(&mut b, at(A, 60.0, 0));
        moved(&mut b, A, Point::new(20.0, 20.0), 200);
        assert_eq!(b.timer_starts, vec![(1, 0), (0, 200)]);
        assert_eq!(b.timer, Some(0));
        b.fire(500);
        release(&mut b, A, 600);
        assert_eq!(b.typed, "1");
    }

    #[test]
    fn second_finger_while_open_commits_highlighted_then_presses() {
        let mut b = choices_board();
        press(&mut b, at(A, 60.0, 0));
        b.fire(300);
        press(&mut b, at(B, 20.0, 400));
        assert_eq!(b.typed, "4");
        assert!(!b.row_shown);
        release(&mut b, A, 450);
        release(&mut b, B, 500);
        assert_eq!(b.typed, "4q");
    }

    #[test]
    fn slide_off_the_row_and_release_types_nothing() {
        let mut b = choices_board();
        press(&mut b, at(A, 60.0, 0));
        b.fire(300);
        moved(&mut b, A, Point::new(60.0, 100.0), 350);
        release(&mut b, A, 400);
        assert_eq!(b.typed, "");
        assert!(!b.row_shown);
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

    #[test]
    fn abort_while_key_held_types_nothing() {
        // A focus change to a field on the same view, mid-press.
        let mut b = choices_board();
        press(&mut b, at(A, 60.0, 0));
        abort(&mut b, 100);
        assert_eq!(b.timer, None, "the long-press wait is cancelled");
        release(&mut b, A, 150);
        assert_eq!(b.typed, "");
        assert_eq!(b.tracker.contact(), None);
    }

    #[test]
    fn abort_while_choices_open_closes_row_and_types_nothing() {
        let mut b = choices_board();
        press(&mut b, at(A, 60.0, 0));
        b.fire(300);
        assert!(b.row_shown);
        abort(&mut b, 350);
        assert!(!b.row_shown, "the row closes");
        release(&mut b, A, 400);
        // The next press does not finish (commit) the aborted contact.
        press(&mut b, at(B, 20.0, 500));
        release(&mut b, B, 550);
        assert_eq!(b.typed, "q");
    }

    #[test]
    fn abort_stops_a_running_hold() {
        let mut b = Fake::new(vec![[Act::Letter('q'), Act::Letter('h')]]);
        b.repeat = Some(1);
        press(&mut b, at(A, 60.0, 0));
        assert!(b.holding);
        abort(&mut b, 100);
        assert!(!b.holding, "no stuck repeat");
        release(&mut b, A, 150);
        assert_eq!(b.typed, "");
    }

    #[test]
    fn abort_drops_a_held_back_press() {
        let mut b = Fake::new(vec![[Act::Letter('q'), Act::Letter('h')]]);
        b.laid[0] = false;
        press(&mut b, at(A, 60.0, 0));
        abort(&mut b, 5);
        b.laid[0] = true;
        release(&mut b, A, 10);
        assert_eq!(b.typed, "");
    }
}
