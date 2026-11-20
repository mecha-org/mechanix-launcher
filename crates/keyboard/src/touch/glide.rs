//! Glide: the caret follows the finger, like Gboard's spacebar. Once a swipe
//! from a gliding key (the space bar) passes a small threshold, the finger's
//! motion, anywhere on the keyboard until it lifts, becomes arrow key taps:
//! one per [`GlideSpec`] step of travel, multiplied by a gain that grows with
//! the finger's *current* speed (smoothed over the last few moves, not
//! measured from the press). A slow drag moves a character or two; a quick
//! flick from wherever the finger is moves several.
//!
//! Pure and allocation-free per move: the [`tracker`](super::tracker) owns an
//! open [`Glide`] and turns its [`Steps`] into outputs. Each axis is enabled
//! separately with its own step, so the same engine serves a horizontal-only
//! space bar and could drive a two-axis pointing stick by absolute position
//! and velocity instead of holding arrows.

use super::pointing_stick::Direction;
use mecha_wayland::prelude::*;

/// Travel before a glide starts, as a fraction of the key's height.
const ACTIVATION: f32 = 0.4;
/// Travel per arrow at slow speed, as a fraction of the key's height.
const STEP: f32 = 0.6;
/// At or below this speed (logical px/ms) every step is one arrow.
const SLOW: f32 = 0.25;
/// At or above this speed the gain is [`MAX_GAIN`].
const FAST: f32 = 1.5;
/// Arrows per step at full speed.
const MAX_GAIN: f32 = 2.5;
/// Weight of the newest move in the smoothed speed.
const SMOOTHING: f32 = 0.5;
/// A move this long (ms) after the last one starts the speed afresh: the
/// finger paused, so the earlier speed says nothing about this move.
const PAUSE_MS: u32 = 40;
/// Most arrows one move sends per axis, against a stray jump.
const MAX_PER_MOVE: i32 = 12;

/// How a key glides: which axes move the caret, with the travel per arrow at
/// slow speed on each, and the travel that starts the glide.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct GlideSpec {
    /// Travel per Left/Right arrow at slow speed; `None` leaves x alone.
    pub step_x: Option<f32>,
    /// Travel per Up/Down arrow at slow speed; `None` leaves y alone.
    pub step_y: Option<f32>,
    /// Travel from the press along an enabled axis, dominating the other,
    /// that starts the glide.
    pub activation: f32,
}

impl GlideSpec {
    /// The space bar's glide, for a key `key_height` tall: horizontal only.
    pub(crate) fn horizontal(key_height: f32) -> GlideSpec {
        GlideSpec {
            step_x: Some(key_height * STEP),
            step_y: None,
            activation: key_height * ACTIVATION,
        }
    }

    /// A two-axis glide for a key `key_height` tall (a pointing stick driven
    /// by position and velocity rather than held arrows).
    #[allow(dead_code)]
    pub(crate) fn both(key_height: f32) -> GlideSpec {
        GlideSpec {
            step_x: Some(key_height * STEP),
            step_y: Some(key_height * STEP),
            activation: key_height * ACTIVATION,
        }
    }

    /// Whether a finger offset `(dx, dy)` from the press starts the glide:
    /// far enough along an enabled axis, and more along it than the other
    /// (so a downward closing flick or a slide up to the keys above doesn't).
    pub(crate) fn starts(&self, dx: f32, dy: f32) -> bool {
        let (ax, ay) = (dx.abs(), dy.abs());
        (self.step_x.is_some() && ax >= self.activation && ax > ay)
            || (self.step_y.is_some() && ay >= self.activation && ay > ax)
    }

    /// Start gliding from `origin`, where the finger was at `time`: travel
    /// since then counts toward the first arrow.
    pub(crate) fn open(self, origin: Point, time: u32) -> Glide {
        Glide {
            spec: self,
            last: origin,
            last_time: time,
            speed: 0.0,
            acc: (0.0, 0.0),
        }
    }
}

/// Arrows to send for one move: signed counts per axis (x right, y down).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Steps {
    pub x: i32,
    pub y: i32,
}

impl Steps {
    pub(crate) fn is_empty(self) -> bool {
        self.x == 0 && self.y == 0
    }

    /// The arrows to send, as at most two `(direction, count)` runs.
    pub(crate) fn runs(self) -> impl Iterator<Item = (Direction, u32)> {
        let x = match self.x {
            0 => None,
            n if n < 0 => Some((Direction::Left, n.unsigned_abs())),
            n => Some((Direction::Right, n as u32)),
        };
        let y = match self.y {
            0 => None,
            n if n < 0 => Some((Direction::Up, n.unsigned_abs())),
            n => Some((Direction::Down, n as u32)),
        };
        x.into_iter().chain(y)
    }
}

/// An open glide, following one finger.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Glide {
    spec: GlideSpec,
    /// The finger's last position and its time.
    last: Point,
    last_time: u32,
    /// Smoothed speed, logical px/ms.
    speed: f32,
    /// Fractions of an arrow carried to the next move, per axis.
    acc: (f32, f32),
}

impl Glide {
    /// The finger moved to `p` at `time`: the arrows that motion is worth.
    pub(crate) fn track(&mut self, p: Point, time: u32) -> Steps {
        let dx = if self.spec.step_x.is_some() {
            p.x - self.last.x
        } else {
            0.0
        };
        let dy = if self.spec.step_y.is_some() {
            p.y - self.last.y
        } else {
            0.0
        };
        let dt = time.wrapping_sub(self.last_time);
        self.last = p;
        self.last_time = time;
        if dx == 0.0 && dy == 0.0 {
            return Steps::default();
        }
        // Same-timestamp moves keep the speed; a pause starts it afresh.
        if dt > 0 {
            let now = (dx * dx + dy * dy).sqrt() / dt as f32;
            self.speed = if dt >= PAUSE_MS {
                now
            } else {
                self.speed + SMOOTHING * (now - self.speed)
            };
        }
        let gain = gain(self.speed);
        Steps {
            x: advance(&mut self.acc.0, dx, self.spec.step_x, gain),
            y: advance(&mut self.acc.1, dy, self.spec.step_y, gain),
        }
    }
}

/// Arrows per step at `speed`: 1 when slow, rising linearly to
/// [`MAX_GAIN`] at [`FAST`].
fn gain(speed: f32) -> f32 {
    let t = ((speed - SLOW) / (FAST - SLOW)).clamp(0.0, 1.0);
    1.0 + (MAX_GAIN - 1.0) * t
}

/// Add travel `d` (scaled by `gain`) to one axis's carried fraction `acc`,
/// taking out whole arrows. Turning back drops the fraction carried the
/// other way, so the caret reverses at once.
fn advance(acc: &mut f32, d: f32, step: Option<f32>, gain: f32) -> i32 {
    let Some(step) = step.filter(|s| *s > 0.0) else {
        return 0;
    };
    if d == 0.0 {
        return 0;
    }
    if *acc != 0.0 && acc.signum() != d.signum() {
        *acc = 0.0;
    }
    *acc += d * gain / step;
    // A hair of slack so exactly one step's travel is one arrow despite
    // rounding.
    let n = (*acc + acc.signum() * 1e-4).trunc();
    *acc -= n;
    let n = n as i32;
    if n.abs() > MAX_PER_MOVE {
        *acc = 0.0;
        return n.signum() * MAX_PER_MOVE;
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 50 px key: activation 20 px, 30 px per arrow when slow.
    fn spec() -> GlideSpec {
        GlideSpec::horizontal(50.0)
    }

    /// A straight move from `x0` by `dx` over `ms`, a move every 8 ms from
    /// time `t0`; the total arrows sent along x.
    fn swipe(g: &mut Glide, x0: f32, dx: f32, ms: u32, t0: u32) -> i32 {
        let steps = ms / 8;
        (1..=steps)
            .map(|i| {
                let x = x0 + dx * i as f32 / steps as f32;
                g.track(Point::new(x, 0.0), t0 + i * 8).x
            })
            .sum()
    }

    #[test]
    fn starts_only_along_an_enabled_dominant_axis() {
        let s = spec();
        assert!(!s.starts(10.0, 0.0), "inside the threshold");
        assert!(s.starts(-21.0, 3.0));
        assert!(s.starts(21.0, 0.0));
        assert!(!s.starts(21.0, 30.0), "mostly vertical");
        assert!(!s.starts(0.0, 60.0), "x only");
        assert!(GlideSpec::both(50.0).starts(0.0, -25.0));
    }

    #[test]
    fn slow_drag_moves_a_character_or_two() {
        let mut g = spec().open(Point::new(0.0, 0.0), 0);
        // 60 px over 400 ms: 0.15 px/ms.
        let n = swipe(&mut g, 0.0, -60.0, 400, 0);
        assert!((-2..=-1).contains(&n), "{n}");
    }

    #[test]
    fn quick_flick_moves_several() {
        let mut g = spec().open(Point::new(0.0, 0.0), 0);
        // 60 px over 40 ms: 1.5 px/ms.
        let n = swipe(&mut g, 0.0, -60.0, 40, 0);
        assert!((-6..=-4).contains(&n), "{n}");
    }

    #[test]
    fn speed_is_the_current_one_not_since_the_press() {
        let mut g = spec().open(Point::new(0.0, 0.0), 0);
        // A long slow drag right, a rest, then a quick flick left.
        swipe(&mut g, 0.0, 120.0, 800, 0);
        g.track(Point::new(120.0, 0.0), 1500);
        let n = swipe(&mut g, 120.0, -60.0, 40, 1500);
        assert!((-6..=-4).contains(&n), "{n}");
    }

    #[test]
    fn reversing_drops_the_carried_fraction() {
        let mut g = spec().open(Point::new(0.0, 0.0), 0);
        // 25 px right, slowly: no arrow yet, 0.83 carried.
        assert_eq!(g.track(Point::new(25.0, 0.0), 200).x, 0);
        // 30 px back left: one Left, not cancelled by the carried Right.
        assert_eq!(g.track(Point::new(-5.0, 0.0), 400).x, -1);
    }

    #[test]
    fn disabled_axis_sends_nothing() {
        let mut g = spec().open(Point::new(0.0, 0.0), 0);
        assert_eq!(g.track(Point::new(0.0, 300.0), 100), Steps::default());
    }

    #[test]
    fn a_stray_jump_is_capped() {
        let mut g = spec().open(Point::new(0.0, 0.0), 0);
        assert_eq!(g.track(Point::new(5000.0, 0.0), 8).x, MAX_PER_MOVE);
    }

    #[test]
    fn same_timestamp_moves_do_not_explode_the_speed() {
        let mut g = spec().open(Point::new(0.0, 0.0), 100);
        assert_eq!(g.track(Point::new(-30.0, 0.0), 100).x, -1);
    }

    #[test]
    fn runs_name_each_direction() {
        let s = Steps { x: -3, y: 2 };
        let v: Vec<_> = s.runs().collect();
        assert_eq!(v, vec![(Direction::Left, 3), (Direction::Down, 2)]);
        assert!(Steps::default().is_empty());
    }
}
