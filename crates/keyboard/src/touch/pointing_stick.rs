//! The pointing stick's direction pad, as geometry: where its four cells open
//! around the held key and which direction a finger moving from where the
//! pad opened points in. Pure; the [`tracker`](super::tracker) owns the open
//! pad and the `pointing_stick` widget draws it.
//!
//! The direction is continuous, like a hardware pointing stick: it follows
//! the angle of the finger's offset from where it rested when the pad opened,
//! not which cell the finger is over. Within a dead zone around that point no
//! direction is held, and hysteresis keeps the direction from flickering on a
//! diagonal or at the dead zone's edge.

use std::f32::consts::FRAC_PI_4;
use std::time::Duration;

use mecha_wayland::prelude::*;
use xkbcommon::xkb::Keysym;

use super::long_press::LONG_PRESS;

/// How long the pointing stick is held before its pad opens: half a
/// long-press.
pub(crate) const POINTING_STICK_HOLD: Duration =
    Duration::from_millis(LONG_PRESS.as_millis() as u64 / 2);

/// The dead zone's radius, as a fraction of a cell's side.
const DEAD_ZONE: f32 = 0.35;
/// Once a direction is held, it lets go only inside this fraction of the
/// dead zone, so a finger resting at the zone's edge doesn't flicker.
const RELEASE: f32 = 0.7;
/// How far past the 45° diagonal (radians) the finger must turn before a held
/// direction gives way to its neighbour.
const HYSTERESIS: f32 = 10.0 * std::f32::consts::PI / 180.0;

/// A direction the pointing stick holds an arrow key in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    /// Every direction, in [`index`](Self::index) order.
    pub(crate) const ALL: [Direction; 4] = [
        Direction::Up,
        Direction::Down,
        Direction::Left,
        Direction::Right,
    ];

    /// The direction's position in [`ALL`](Self::ALL) and in a pad's cells.
    pub(crate) fn index(self) -> usize {
        self as usize
    }

    /// The arrow key the direction holds.
    pub(crate) fn keysym(self) -> Keysym {
        match self {
            Direction::Up => Keysym::Up,
            Direction::Down => Keysym::Down,
            Direction::Left => Keysym::Left,
            Direction::Right => Keysym::Right,
        }
    }

    /// The icon the direction's cell shows.
    pub(crate) fn icon(self) -> &'static str {
        match self {
            Direction::Up => "go-up-symbolic",
            Direction::Down => "go-down-symbolic",
            Direction::Left => "go-previous-symbolic",
            Direction::Right => "go-next-symbolic",
        }
    }

    /// The unit vector the direction points along (y grows downward).
    fn unit(self) -> (f32, f32) {
        match self {
            Direction::Up => (0.0, -1.0),
            Direction::Down => (0.0, 1.0),
            Direction::Left => (-1.0, 0.0),
            Direction::Right => (1.0, 0.0),
        }
    }

    /// The direction nearest an offset `(dx, dy)`: its dominant axis.
    fn of(dx: f32, dy: f32) -> Direction {
        if dx.abs() >= dy.abs() {
            if dx < 0.0 {
                Direction::Left
            } else {
                Direction::Right
            }
        } else if dy < 0.0 {
            Direction::Up
        } else {
            Direction::Down
        }
    }
}

/// An open direction pad.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PointingStickPad {
    /// Each direction's cell, by [`Direction::index`], in window coordinates.
    pub cells: [Rect; 4],
    /// Where the finger rested when the pad opened: directions are measured
    /// from here.
    pub origin: Point,
    /// The dead zone's radius around `origin`.
    pub dead_zone: f32,
    /// The direction being held, if any.
    pub direction: Option<Direction>,
}

impl PointingStickPad {
    /// The pad for the key at `key`, inside `bounds` (the keyboard plus the
    /// headroom above it): one square cell, the key's height on a side, on
    /// each side of the key, `gap` away from it, each pushed inside `bounds`.
    /// Directions are measured from `origin`, the finger's position. No
    /// direction is held yet.
    pub(crate) fn open(key: Rect, bounds: Rect, gap: f32, origin: Point) -> PointingStickPad {
        let side = key.size.height;
        let cx = key.origin.x + key.size.width / 2.0;
        let cy = key.origin.y + key.size.height / 2.0;
        let cell = |x: f32, y: f32| {
            let x = x.min(bounds.right() - side).max(bounds.origin.x);
            let y = y.min(bounds.bottom() - side).max(bounds.origin.y);
            Rect::new(x, y, side, side)
        };
        let mid_x = cx - side / 2.0;
        let mid_y = cy - side / 2.0;
        let cells = [
            cell(mid_x, key.origin.y - gap - side),
            cell(mid_x, key.bottom() + gap),
            cell(key.origin.x - gap - side, mid_y),
            cell(key.right() + gap, mid_y),
        ];
        PointingStickPad {
            cells,
            origin,
            dead_zone: side * DEAD_ZONE,
            direction: None,
        }
    }

    /// The smallest rectangle holding every cell.
    pub(crate) fn rect(&self) -> Rect {
        let left = self
            .cells
            .iter()
            .map(|c| c.origin.x)
            .fold(f32::MAX, f32::min);
        let top = self
            .cells
            .iter()
            .map(|c| c.origin.y)
            .fold(f32::MAX, f32::min);
        let right = self
            .cells
            .iter()
            .map(|c| c.right())
            .fold(f32::MIN, f32::max);
        let bottom = self
            .cells
            .iter()
            .map(|c| c.bottom())
            .fold(f32::MIN, f32::max);
        Rect::new(left, top, right - left, bottom - top)
    }

    /// The finger moved to `p`: hold the direction its offset from `origin`
    /// points in, or none inside the dead zone.
    pub(crate) fn track(&mut self, p: Point) {
        let (dx, dy) = (p.x - self.origin.x, p.y - self.origin.y);
        let r = (dx * dx + dy * dy).sqrt();
        self.direction = match self.direction {
            None if r < self.dead_zone => None,
            None => Some(Direction::of(dx, dy)),
            Some(_) if r < self.dead_zone * RELEASE => None,
            Some(d) => {
                let (ux, uy) = d.unit();
                let cos = (dx * ux + dy * uy) / r;
                if cos >= (FRAC_PI_4 + HYSTERESIS).cos() {
                    Some(d)
                } else {
                    Some(Direction::of(dx, dy))
                }
            }
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 500-wide keyboard under a 70 px headroom strip.
    const BOUNDS: Rect = Rect::new(0.0, 0.0, 500.0, 320.0);
    /// A narrow 30x50 key in the middle of the keyboard.
    const KEY: Rect = Rect::new(235.0, 150.0, 30.0, 50.0);
    const CENTRE: Point = Point::new(250.0, 175.0);

    fn pad() -> PointingStickPad {
        PointingStickPad::open(KEY, BOUNDS, 4.0, CENTRE)
    }

    fn at(dx: f32, dy: f32) -> Point {
        Point::new(CENTRE.x + dx, CENTRE.y + dy)
    }

    #[test]
    fn hold_is_half_a_long_press() {
        assert_eq!(POINTING_STICK_HOLD * 2, LONG_PRESS);
    }

    #[test]
    fn cells_surround_the_key() {
        let p = pad();
        assert_eq!(
            p.cells[Direction::Up.index()],
            Rect::new(225.0, 96.0, 50.0, 50.0)
        );
        assert_eq!(
            p.cells[Direction::Down.index()],
            Rect::new(225.0, 204.0, 50.0, 50.0)
        );
        assert_eq!(
            p.cells[Direction::Left.index()],
            Rect::new(181.0, 150.0, 50.0, 50.0)
        );
        assert_eq!(
            p.cells[Direction::Right.index()],
            Rect::new(269.0, 150.0, 50.0, 50.0)
        );
        assert_eq!(p.rect(), Rect::new(181.0, 96.0, 138.0, 158.0));
        assert_eq!(p.direction, None);
    }

    #[test]
    fn cells_are_pushed_inside_bounds() {
        // A key in the bottom-left corner: the left and down cells move in.
        let key = Rect::new(0.0, 270.0, 30.0, 50.0);
        let p = PointingStickPad::open(key, BOUNDS, 4.0, Point::new(15.0, 295.0));
        for c in p.cells {
            assert!(c.origin.x >= 0.0 && c.right() <= 500.0, "{c:?}");
            assert!(c.origin.y >= 0.0 && c.bottom() <= 320.0, "{c:?}");
        }
    }

    #[test]
    fn dead_zone_holds_nothing() {
        let mut p = pad();
        // 0.35 of a 50 px cell: 17.5 px.
        for (dx, dy) in [(0.0, 0.0), (10.0, 0.0), (0.0, -17.0), (-12.0, 12.0)] {
            p.track(at(dx, dy));
            assert_eq!(p.direction, None, "({dx}, {dy})");
        }
    }

    #[test]
    fn direction_follows_the_angle_not_the_cells() {
        let mut p = pad();
        for (dx, dy, d) in [
            (30.0, 5.0, Direction::Right),
            (-30.0, 10.0, Direction::Left),
            (8.0, -30.0, Direction::Up),
            (-5.0, 30.0, Direction::Down),
        ] {
            let mut p2 = pad();
            p2.track(at(dx, dy));
            assert_eq!(p2.direction, Some(d), "({dx}, {dy})");
        }
        // Far beyond any cell still points the same way.
        p.track(at(400.0, 20.0));
        assert_eq!(p.direction, Some(Direction::Right));
    }

    #[test]
    fn origin_is_where_the_finger_rested_not_the_key_centre() {
        // The finger opened the pad near the key's right edge: moving left
        // to the key's centre already points left.
        let mut p = PointingStickPad::open(KEY, BOUNDS, 4.0, at(25.0, 0.0));
        p.track(CENTRE);
        assert_eq!(p.direction, Some(Direction::Left));
    }

    #[test]
    fn hysteresis_keeps_a_direction_past_the_diagonal() {
        let mut p = pad();
        p.track(at(30.0, 0.0));
        assert_eq!(p.direction, Some(Direction::Right));
        // 50° from right: past the diagonal, within the hysteresis.
        let a = 50f32.to_radians();
        p.track(at(30.0 * a.cos(), -30.0 * a.sin()));
        assert_eq!(p.direction, Some(Direction::Right));
        // 60°: gives way to up.
        let a = 60f32.to_radians();
        p.track(at(30.0 * a.cos(), -30.0 * a.sin()));
        assert_eq!(p.direction, Some(Direction::Up));
        // Back to 50°: up holds now.
        let a = 50f32.to_radians();
        p.track(at(30.0 * a.cos(), -30.0 * a.sin()));
        assert_eq!(p.direction, Some(Direction::Up));
        // Straight through to the opposite side flips at once.
        p.track(at(0.0, 30.0));
        assert_eq!(p.direction, Some(Direction::Down));
    }

    #[test]
    fn letting_go_needs_the_inner_dead_zone() {
        let mut p = pad();
        p.track(at(30.0, 0.0));
        // Inside the dead zone (17.5) but outside 0.7 of it (12.25): held.
        p.track(at(15.0, 0.0));
        assert_eq!(p.direction, Some(Direction::Right));
        p.track(at(10.0, 0.0));
        assert_eq!(p.direction, None);
        // Re-entering needs the full dead zone again.
        p.track(at(15.0, 0.0));
        assert_eq!(p.direction, None);
        p.track(at(18.0, 0.0));
        assert_eq!(p.direction, Some(Direction::Right));
    }

    #[test]
    fn every_direction_has_an_arrow_keysym() {
        let syms: Vec<_> = Direction::ALL.iter().map(|d| d.keysym()).collect();
        assert_eq!(
            syms,
            [Keysym::Up, Keysym::Down, Keysym::Left, Keysym::Right]
        );
        for (i, d) in Direction::ALL.iter().enumerate() {
            assert_eq!(d.index(), i);
        }
    }
}
