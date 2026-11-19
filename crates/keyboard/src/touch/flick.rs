//! Swipe down to close: a fast, mostly vertical downward flick that starts
//! on the keyboard closes it. It need not end below the keys.

use std::time::{Duration, Instant};

use gestures::{Axis, AxisLock, VelocityTracker};
use mecha_wayland::prelude::*;

/// Downward speed at release a closing flick needs, in logical px/ms.
pub(crate) const MIN_VELOCITY: f32 = 1.2;
/// Downward travel from the press a closing flick needs, in logical px.
pub(crate) const MIN_TRAVEL: f32 = 60.0;
/// Movement (logical px) after which the flick is locked to one axis.
const AXIS_LOCK: f32 = 10.0;

/// One contact's motion, watched for a closing flick.
pub(crate) struct Flick {
    tracker: VelocityTracker,
    start: Point,
    lock: AxisLock,
    /// The last position seen.
    last: Point,
    /// The axis the motion locked to, once it did.
    axis: Option<Axis>,
    /// `Instant` standing for event time `t0`, so event times map onto the
    /// tracker's clock.
    base: Instant,
    t0: u32,
    /// The latest event time: the moment velocity is read at.
    now: u32,
}

impl Flick {
    /// A contact went down at `p` at event time `time` (ms).
    pub(crate) fn start(p: Point, time: u32) -> Flick {
        let mut tracker = VelocityTracker::new(
            Duration::from_millis(80),
            Duration::from_millis(80),
            Duration::from_millis(8),
        );
        let base = Instant::now();
        tracker.push(base, p.x, p.y);
        Flick {
            tracker,
            start: p,
            lock: AxisLock::new(AXIS_LOCK),
            last: p,
            axis: None,
            base,
            t0: time,
            now: time,
        }
    }

    /// The contact moved to `p` at `time`.
    pub(crate) fn sample(&mut self, p: Point, time: u32) {
        self.now = time;
        self.last = p;
        self.tracker.push(self.instant(time), p.x, p.y);
        self.axis = self.lock.update(p.x - self.start.x, p.y - self.start.y);
    }

    /// The contact lifted at `time` without moving further: a finger that
    /// stopped before lifting has no speed left.
    pub(crate) fn end(&mut self, time: u32) {
        self.now = time;
    }

    /// Whether the motion so far is a closing flick: locked vertical,
    /// moving down at [`MIN_VELOCITY`] or faster, and [`MIN_TRAVEL`] below
    /// the press.
    pub(crate) fn is_close(&self) -> bool {
        let (_, vy) = self.tracker.velocity(self.instant(self.now));
        self.axis == Some(Axis::Vertical)
            && vy >= MIN_VELOCITY
            && self.last.y - self.start.y >= MIN_TRAVEL
    }

    fn instant(&self, time: u32) -> Instant {
        self.base + Duration::from_millis(time.wrapping_sub(self.t0) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A straight move from `(x0, y0)` by `(dx, dy)` over `ms`, sampled every
    /// 8 ms, starting at time 1000; ends at time `1000 + ms`.
    fn swipe(x0: f32, y0: f32, dx: f32, dy: f32, ms: u32) -> Flick {
        let mut f = Flick::start(Point::new(x0, y0), 1000);
        let steps = ms / 8;
        for i in 1..=steps {
            let k = i as f32 / steps as f32;
            f.sample(Point::new(x0 + dx * k, y0 + dy * k), 1000 + i * 8);
        }
        f.end(1000 + steps * 8);
        f
    }

    #[test]
    fn fast_downward_swipe_closes() {
        assert!(swipe(100.0, 20.0, 0.0, 150.0, 80).is_close());
        // A little sideways drift is still mostly vertical.
        assert!(swipe(100.0, 20.0, 25.0, 150.0, 80).is_close());
    }

    #[test]
    fn slow_downward_drag_does_not_close() {
        assert!(!swipe(100.0, 20.0, 0.0, 150.0, 600).is_close());
    }

    #[test]
    fn horizontal_slide_between_keys_does_not_close() {
        assert!(!swipe(100.0, 20.0, 120.0, 0.0, 64).is_close());
        // Sideways first, then down: the lock stays horizontal.
        let mut f = swipe(100.0, 20.0, 40.0, 0.0, 24);
        for i in 1..=10u32 {
            f.sample(Point::new(140.0, 20.0 + 15.0 * i as f32), 1024 + i * 8);
        }
        assert!(!f.is_close());
    }

    #[test]
    fn upward_swipe_does_not_close() {
        assert!(!swipe(100.0, 200.0, 0.0, -150.0, 80).is_close());
    }

    #[test]
    fn short_or_stopped_flick_does_not_close() {
        // Fast but short.
        assert!(!swipe(100.0, 20.0, 0.0, 40.0, 24).is_close());
        // Fast, then held still before lifting.
        let mut f = swipe(100.0, 20.0, 0.0, 150.0, 80);
        f.end(1000 + 80 + 200);
        assert!(!f.is_close());
    }
}
