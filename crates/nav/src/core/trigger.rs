use std::cell::Cell;
use std::rc::Rc;

use gestures::Axis;

use super::config::NavConfig;
use super::gesture::{Detect, Gesture, Phase};
use super::{AreaId, EXTERNAL};

/// Events from outside the touch stream.
#[non_exhaustive]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ExternalEvent {
    /// A screen started an app.
    AppLaunched,
}

pub trait Trigger: 'static {
    fn area(&self) -> AreaId;
    fn detect(&self, g: &Gesture, phase: Phase, cfg: &NavConfig) -> Detect;
    fn on_external(&self, _e: &ExternalEvent) -> bool {
        false
    }
    fn mirrored(&self) -> Option<Box<dyn Trigger>> {
        None
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    fn opposite(self) -> Self {
        match self {
            Direction::Up => Direction::Down,
            Direction::Down => Direction::Up,
            Direction::Left => Direction::Right,
            Direction::Right => Direction::Left,
        }
    }

    fn axis(self) -> Axis {
        match self {
            Direction::Up | Direction::Down => Axis::Vertical,
            Direction::Left | Direction::Right => Axis::Horizontal,
        }
    }

    /// Projects a screen-space (dx, dy) so that movement along `self` is positive.
    fn along(self, (x, y): (f32, f32)) -> f32 {
        match self {
            Direction::Up => -y,
            Direction::Down => y,
            Direction::Left => -x,
            Direction::Right => x,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Timing {
    Any,
    Quick,
    Held,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Span {
    Whole,
    LeftHalf,
    RightHalf,
    /// Starts within [`NavConfig::corner_fraction`] of the left end.
    LeftCorner,
    /// Starts within [`NavConfig::corner_fraction`] of the right end.
    RightCorner,
    /// Starts in neither corner.
    Middle,
}

#[derive(Clone, Copy)]
pub struct Swipe {
    pub area: AreaId,
    pub dir: Direction,
    pub timing: Timing,
    pub span: Span,
}

pub const fn swipe(area: AreaId, dir: Direction, timing: Timing) -> Swipe {
    Swipe {
        area,
        dir,
        timing,
        span: Span::Whole,
    }
}

impl Swipe {
    pub const fn span(self, span: Span) -> Swipe {
        Swipe { span, ..self }
    }

    fn starts_in_span(&self, g: &Gesture, cfg: &NavConfig) -> bool {
        let width = g.area_size.width;
        let half = width / 2.0;
        let corner = width * cfg.corner_fraction;
        let x = g.start.x;
        match self.span {
            Span::Whole => true,
            Span::LeftHalf => x < half,
            Span::RightHalf => x >= half,
            Span::LeftCorner => x < corner,
            Span::RightCorner => x >= width - corner,
            Span::Middle => x >= corner && x < width - corner,
        }
    }
}

impl Trigger for Swipe {
    fn area(&self) -> AreaId {
        self.area
    }

    fn detect(&self, g: &Gesture, phase: Phase, cfg: &NavConfig) -> Detect {
        if !self.starts_in_span(g, cfg) || g.axis.is_some_and(|a| a != self.dir.axis()) {
            return Detect::Ignore;
        }
        let needed = match self.timing {
            Timing::Any => cfg.swipe_commit_px,
            Timing::Quick | Timing::Held => cfg.min_travel_px,
        };
        let travel = self.dir.along(g.delta());
        match phase {
            Phase::Moving => Detect::Progress((travel / needed).clamp(0.0, 1.0)),
            Phase::Released => {
                let held =
                    g.elapsed >= cfg.hold_time && self.dir.along(g.velocity) < cfg.flick_px_per_ms;
                let timing_ok = match self.timing {
                    Timing::Any => true,
                    Timing::Held => held,
                    Timing::Quick => !held,
                };
                if travel >= needed && timing_ok {
                    Detect::Commit
                } else {
                    Detect::Cancel
                }
            }
        }
    }

    fn mirrored(&self) -> Option<Box<dyn Trigger>> {
        Some(Box::new(Swipe {
            dir: self.dir.opposite(),
            ..*self
        }))
    }
}

/// Fires on one [`ExternalEvent`] and on no gesture.
pub struct OnExternal(pub ExternalEvent);

impl Trigger for OnExternal {
    fn area(&self) -> AreaId {
        EXTERNAL
    }

    fn detect(&self, _g: &Gesture, _phase: Phase, _cfg: &NavConfig) -> Detect {
        Detect::Ignore
    }

    fn on_external(&self, e: &ExternalEvent) -> bool {
        *e == self.0
    }
}

/// A trigger that only detects while its gate is open; built with [`gated`].
pub struct Gated<T> {
    inner: T,
    gate: Rc<Cell<bool>>,
}

/// `t`, ignoring every gesture while `gate` is false. An ignored trigger
/// is dropped for the rest of its gesture, so a gate closed at any point
/// of a drag keeps it from firing for that whole drag.
pub fn gated<T: Trigger>(t: T, gate: Rc<Cell<bool>>) -> Gated<T> {
    Gated { inner: t, gate }
}

impl<T: Trigger> Trigger for Gated<T> {
    fn area(&self) -> AreaId {
        self.inner.area()
    }

    fn detect(&self, g: &Gesture, phase: Phase, cfg: &NavConfig) -> Detect {
        if !self.gate.get() {
            return Detect::Ignore;
        }
        self.inner.detect(g, phase, cfg)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;
    use std::time::Duration;

    use mecha_wayland::prelude::{Point, Size};

    use super::*;
    use crate::core::{BOTTOM_EDGE, TOP_EDGE};

    fn gesture(
        area_size: Size,
        start: Point,
        end: Point,
        elapsed_ms: u64,
        velocity: (f32, f32),
    ) -> Gesture {
        Gesture {
            area: BOTTOM_EDGE,
            area_size,
            start,
            pos: end,
            elapsed: Duration::from_millis(elapsed_ms),
            velocity,
            axis: None,
        }
    }

    fn up(dy: f32, ms: u64, vy: f32) -> Gesture {
        gesture(
            Size {
                width: 600.0,
                height: 20.0,
            },
            Point { x: 300.0, y: 10.0 },
            Point {
                x: 300.0,
                y: 10.0 + dy,
            },
            ms,
            (0.0, vy),
        )
    }

    fn down(dy: f32) -> Gesture {
        gesture(
            Size {
                width: 600.0,
                height: 20.0,
            },
            Point { x: 300.0, y: 0.0 },
            Point { x: 300.0, y: dy },
            100,
            (0.0, 0.0),
        )
    }

    fn run(t: Swipe, g: &Gesture, p: Phase) -> Detect {
        t.detect(g, p, &NavConfig::default())
    }

    const QUICK: Swipe = swipe(BOTTOM_EDGE, Direction::Up, Timing::Quick);
    const HELD: Swipe = swipe(BOTTOM_EDGE, Direction::Up, Timing::Held);

    #[test]
    fn quick_up_commits_quick_not_held() {
        let g = up(-80.0, 100, -1.5);
        assert_eq!(run(QUICK, &g, Phase::Released), Detect::Commit);
        assert_eq!(run(HELD, &g, Phase::Released), Detect::Cancel);
    }

    #[test]
    fn slow_up_commits_held() {
        let g = up(-80.0, 400, -0.2);
        assert_eq!(run(HELD, &g, Phase::Released), Detect::Commit);
        assert_eq!(run(QUICK, &g, Phase::Released), Detect::Cancel);
    }

    #[test]
    fn swipe_and_hold_commits_held() {
        let g = up(-120.0, 600, 0.0);
        assert_eq!(run(HELD, &g, Phase::Released), Detect::Commit);
    }

    #[test]
    fn short_travel_cancels() {
        let g = up(-40.0, 100, -1.5);
        assert_eq!(run(QUICK, &g, Phase::Released), Detect::Cancel);
        assert_eq!(run(HELD, &g, Phase::Released), Detect::Cancel);
    }

    #[test]
    fn any_needs_90px() {
        let t = swipe(TOP_EDGE, Direction::Down, Timing::Any);
        assert_eq!(run(t, &down(89.0), Phase::Released), Detect::Cancel);
        assert_eq!(run(t, &down(90.0), Phase::Released), Detect::Commit);
    }

    #[test]
    fn progress_is_travel_over_needed() {
        let t = swipe(TOP_EDGE, Direction::Down, Timing::Any);
        assert_eq!(run(t, &down(45.0), Phase::Moving), Detect::Progress(0.5));
        assert_eq!(run(t, &down(200.0), Phase::Moving), Detect::Progress(1.0));
    }

    #[test]
    fn span_filters_by_start_x() {
        let g = gesture(
            Size {
                width: 600.0,
                height: 20.0,
            },
            Point { x: 100.0, y: 0.0 },
            Point { x: 100.0, y: 100.0 },
            100,
            (0.0, 0.0),
        );
        let base = swipe(TOP_EDGE, Direction::Down, Timing::Any);
        assert_eq!(
            run(base.span(Span::RightHalf), &g, Phase::Released),
            Detect::Ignore
        );
        assert_eq!(
            run(base.span(Span::LeftHalf), &g, Phase::Released),
            Detect::Commit
        );
    }

    #[test]
    fn corner_spans_split_the_edge_into_corners_and_middle() {
        // 600 wide at 25%: corners end at 150 and start at 450.
        let at = |x: f32| {
            gesture(
                Size {
                    width: 600.0,
                    height: 20.0,
                },
                Point { x, y: 10.0 },
                Point { x, y: -90.0 },
                100,
                (0.0, -1.5),
            )
        };
        let spans = [Span::LeftCorner, Span::Middle, Span::RightCorner];
        for (x, want) in [
            (0.0, 0),
            (149.0, 0),
            (150.0, 1),
            (449.0, 1),
            (450.0, 2),
            (599.0, 2),
        ] {
            for (i, span) in spans.into_iter().enumerate() {
                let d = run(QUICK.span(span), &at(x), Phase::Released);
                let expect = if i == want {
                    Detect::Commit
                } else {
                    Detect::Ignore
                };
                assert_eq!(d, expect, "x {x}, {span:?}");
            }
        }
    }

    #[test]
    fn gated_ignores_while_closed() {
        let bare = swipe(TOP_EDGE, Direction::Down, Timing::Any);
        let gate = Rc::new(Cell::new(false));
        let t = gated(bare, gate.clone());
        let cfg = NavConfig::default();
        for dy in [0.0, 45.0, 200.0] {
            for phase in [Phase::Moving, Phase::Released] {
                assert_eq!(t.detect(&down(dy), phase, &cfg), Detect::Ignore, "{dy}");
            }
        }
        gate.set(true);
        for dy in [0.0, 45.0, 200.0] {
            for phase in [Phase::Moving, Phase::Released] {
                assert_eq!(
                    t.detect(&down(dy), phase, &cfg),
                    bare.detect(&down(dy), phase, &cfg),
                    "{dy}"
                );
            }
        }
        assert_eq!(t.area(), TOP_EDGE);
        assert!(t.mirrored().is_none());
    }

    #[test]
    fn locked_other_axis_ignores() {
        let mut g = up(-80.0, 100, -1.5);
        g.axis = Some(Axis::Horizontal);
        assert_eq!(run(QUICK, &g, Phase::Moving), Detect::Ignore);
    }

    #[test]
    fn mirrored_flips_direction() {
        let t = swipe(TOP_EDGE, Direction::Down, Timing::Quick).span(Span::LeftHalf);
        let m = t.mirrored().unwrap();
        assert_eq!(m.area(), TOP_EDGE);
        // An Up swipe of the mirrored trigger commits where the original cancels.
        let g = gesture(
            Size {
                width: 600.0,
                height: 20.0,
            },
            Point { x: 100.0, y: 100.0 },
            Point { x: 100.0, y: 0.0 },
            100,
            (0.0, -1.5),
        );
        let cfg = NavConfig::default();
        assert_eq!(m.detect(&g, Phase::Released, &cfg), Detect::Commit);
        assert_eq!(t.detect(&g, Phase::Released, &cfg), Detect::Cancel);
        let mm = Swipe {
            dir: Direction::Up,
            ..t
        }
        .mirrored()
        .unwrap();
        assert_eq!(mm.detect(&g, Phase::Released, &cfg), Detect::Cancel);
    }
}
