//! The glance screen's swipe up, detected as the shell's own swipes are
//! (nav's): the view follows the finger, and on release the swipe either
//! commits or the view snaps back.

use std::time::Instant;

use gestures::AxisLock;
use mecha_wayland::prelude::{ContactId, Point, Size};
use nav::core::{AreaId, Detect, Direction, Gesture, NavConfig, Phase, Timing, Trigger, swipe};

/// The lock screen is its own area: no nav window has it.
const AREA: AreaId = AreaId("lock");

#[derive(Default)]
pub(super) struct SwipeUp {
    live: Option<Live>,
    cfg: NavConfig,
}

struct Live {
    contact: ContactId,
    start: Point,
    at: Instant,
    axis: AxisLock,
}

/// What a contact's move or release comes to.
#[derive(Debug, PartialEq)]
pub(super) enum Swiped {
    /// Not this swipe's: nothing to do.
    Ignore,
    /// Still moving: the view follows, this far up (px).
    Follow(f32),
    /// Let go far enough: expand, or unlock.
    Commit,
    /// Let go short, or sideways: back to rest.
    Rest,
}

impl SwipeUp {
    pub(super) fn press(&mut self, contact: ContactId, at: Point) {
        if self.live.is_none() {
            self.live = Some(Live {
                contact,
                start: at,
                at: Instant::now(),
                axis: AxisLock::new(self.cfg.axis_lock_px),
            });
        }
    }

    pub(super) fn moved(&mut self, contact: ContactId, at: Point, size: Size) -> Swiped {
        match self.detect(contact, at, size, Phase::Moving) {
            Some(Detect::Progress(_)) => Swiped::Follow((self.start_y() - at.y).max(0.0)),
            _ => Swiped::Ignore,
        }
    }

    pub(super) fn release(&mut self, contact: ContactId, at: Point, size: Size) -> Swiped {
        let detected = self.detect(contact, at, size, Phase::Released);
        if detected.is_some() {
            self.live = None;
        }
        match detected {
            Some(Detect::Commit) => Swiped::Commit,
            Some(_) => Swiped::Rest,
            None => Swiped::Ignore,
        }
    }

    pub(super) fn cancel(&mut self) {
        self.live = None;
    }

    /// How far up a swipe goes to commit: how far `up` is along it.
    pub(super) fn progress(&self, up: f32) -> f32 {
        (up / self.cfg.swipe_commit_px).clamp(0.0, 1.0)
    }

    fn start_y(&self) -> f32 {
        self.live.as_ref().map_or(0.0, |l| l.start.y)
    }

    fn detect(
        &mut self,
        contact: ContactId,
        at: Point,
        size: Size,
        phase: Phase,
    ) -> Option<Detect> {
        let live = self.live.as_mut().filter(|l| l.contact == contact)?;
        let axis = live.axis.update(at.x - live.start.x, at.y - live.start.y);
        let g = Gesture {
            area: AREA,
            area_size: size,
            start: live.start,
            pos: at,
            elapsed: live.at.elapsed(),
            // `Timing::Any` goes by travel alone.
            velocity: (0.0, 0.0),
            axis,
        };
        Some(swipe(AREA, Direction::Up, Timing::Any).detect(&g, phase, &self.cfg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: Size = Size::new(400.0, 800.0);
    const FINGER: ContactId = ContactId::Touch(0);

    #[test]
    fn follows_up_and_commits_past_the_shells_travel() {
        let mut s = SwipeUp::default();
        let commit = s.cfg.swipe_commit_px;
        s.press(FINGER, Point::new(200.0, 600.0));
        assert_eq!(
            s.moved(FINGER, Point::new(200.0, 560.0), SIZE),
            Swiped::Follow(40.0)
        );
        let end = Point::new(200.0, 600.0 - commit - 1.0);
        assert_eq!(s.moved(FINGER, end, SIZE), Swiped::Follow(commit + 1.0));
        assert_eq!(s.release(FINGER, end, SIZE), Swiped::Commit);
    }

    #[test]
    fn short_or_sideways_rests() {
        let mut s = SwipeUp::default();
        s.press(FINGER, Point::new(200.0, 600.0));
        assert_eq!(
            s.release(FINGER, Point::new(200.0, 570.0), SIZE),
            Swiped::Rest
        );

        s.press(FINGER, Point::new(200.0, 600.0));
        assert_eq!(
            s.moved(FINGER, Point::new(300.0, 590.0), SIZE),
            Swiped::Ignore
        );
        assert_eq!(
            s.release(FINGER, Point::new(300.0, 400.0), SIZE),
            Swiped::Rest
        );
    }
}
