//! The long-press choices row, as geometry: where it opens over the held key
//! and which choice a finger sliding over it picks. Pure; the
//! [`tracker`](super::tracker) owns the open row and the `more_keys` widget
//! draws it.

use std::time::Duration;

use mecha_wayland::prelude::*;

use crate::widget::popup::GAP;

/// How long a key with choices is held before its row opens (AOSP default).
pub(crate) const LONG_PRESS: Duration = Duration::from_millis(300);

/// An open choices row.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ChoicesRow {
    pub choices: Vec<String>,
    /// The whole row, in window coordinates.
    pub rect: Rect,
    /// The long-pressed key, in window coordinates.
    pub key: Rect,
    /// One cell's width.
    pub cell_w: f32,
    /// The choice a release would type, if any.
    pub highlighted: Option<usize>,
    /// Whether the choices run right to left from the key, because the row
    /// has no room to the key's right: the first choice stays over the key.
    pub mirrored: bool,
}

impl ChoicesRow {
    /// The row for a key at `key`, inside `bounds` (the keyboard plus the
    /// headroom above it): its bottom `GAP` above the key, one key-sized cell
    /// per choice with the first over the key, running right (or left when
    /// there is no room), then pushed inside `bounds` as `popup_rect` does.
    /// Cells shrink only when the row is wider than `bounds`. The first
    /// choice is highlighted.
    pub(crate) fn open(key: Rect, bounds: Rect, choices: Vec<String>) -> ChoicesRow {
        let n = choices.len().max(1) as f32;
        let cell_w = key.size.width.min(bounds.size.width / n);
        let (w, h) = (cell_w * n, key.size.height);
        let mirrored = key.origin.x + w > bounds.right();
        let x = if mirrored {
            key.origin.x + cell_w - w
        } else {
            key.origin.x
        };
        let y = key.origin.y - GAP - h;
        let x = x.min(bounds.right() - w).max(bounds.origin.x);
        let y = y.min(bounds.bottom() - h).max(bounds.origin.y);
        ChoicesRow {
            choices,
            rect: Rect::new(x, y, w, h),
            key,
            cell_w,
            highlighted: Some(0),
            mirrored,
        }
    }

    /// The finger moved to `p`: the cell under it (clamped to the row's
    /// ends) is highlighted, or none once it is more than half a key height
    /// below the long-pressed key (a release there cancels). Resting on the
    /// key, or just under it, keeps a highlight.
    pub(crate) fn track(&mut self, p: Point) {
        if p.y > self.key.bottom() + self.key.size.height / 2.0 {
            self.highlighted = None;
            return;
        }
        let last = self.choices.len().saturating_sub(1);
        let slot = ((p.x - self.rect.origin.x) / self.cell_w).floor();
        let slot = (slot.max(0.0) as usize).min(last);
        self.highlighted = Some(self.choice_at(slot));
    }

    /// The choice drawn in cell `slot`, counting cells left to right.
    pub(crate) fn choice_at(&self, slot: usize) -> usize {
        if self.mirrored {
            self.choices.len() - 1 - slot
        } else {
            slot
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 500-wide keyboard under a 70 px headroom strip.
    const BOUNDS: Rect = Rect::new(0.0, 0.0, 500.0, 320.0);

    fn choices(s: &str) -> Vec<String> {
        s.split(' ').map(str::to_string).collect()
    }

    #[test]
    fn opens_above_key_with_first_highlighted() {
        let key = Rect::new(100.0, 200.0, 50.0, 50.0);
        let row = ChoicesRow::open(key, BOUNDS, choices("3 é è ê ë ē"));
        assert_eq!(row.highlighted, Some(0));
        assert_eq!(row.cell_w, 50.0);
        assert_eq!(row.rect, Rect::new(100.0, 146.0, 300.0, 50.0));
        assert!(!row.mirrored);
    }

    #[test]
    fn edge_key_row_is_pushed_inside_bounds() {
        // `q` at the left edge, top row: the row rises into the headroom.
        let q = Rect::new(0.0, 80.0, 50.0, 50.0);
        let row = ChoicesRow::open(q, BOUNDS, choices("1"));
        assert_eq!(row.rect, Rect::new(0.0, 26.0, 50.0, 50.0));
        // `p` at the right edge: the choices run left, the first over `p`.
        let p = Rect::new(450.0, 80.0, 50.0, 50.0);
        let row = ChoicesRow::open(p, BOUNDS, choices("0 a b"));
        assert!(row.mirrored);
        assert_eq!(row.rect, Rect::new(350.0, 26.0, 150.0, 50.0));
        assert_eq!(row.choice_at(2), 0);
        // Too long for either side: pushed inside, cells shrunk to fit.
        let o = Rect::new(400.0, 80.0, 50.0, 50.0);
        let row = ChoicesRow::open(o, BOUNDS, choices("a b c d e f g h i j k l m n o p"));
        assert!(row.rect.origin.x >= 0.0 && row.rect.right() <= 500.0);
        assert_eq!(row.cell_w, 500.0 / 16.0);
        // Above the headroom: pushed down.
        let high = Rect::new(0.0, 10.0, 50.0, 50.0);
        assert_eq!(
            ChoicesRow::open(high, BOUNDS, choices("1")).rect.origin.y,
            0.0
        );
    }

    #[test]
    fn sliding_picks_cell_under_finger_and_clamps_at_ends() {
        let key = Rect::new(100.0, 200.0, 50.0, 50.0);
        let mut row = ChoicesRow::open(key, BOUNDS, choices("3 é è ê"));
        row.track(Point::new(260.0, 170.0));
        assert_eq!(row.highlighted, Some(3));
        row.track(Point::new(160.0, 220.0));
        assert_eq!(row.highlighted, Some(1));
        row.track(Point::new(20.0, 170.0));
        assert_eq!(row.highlighted, Some(0));
        row.track(Point::new(490.0, 170.0));
        assert_eq!(row.highlighted, Some(3));
        // Mirrored: the cell left of the key is the second choice.
        let p = Rect::new(450.0, 200.0, 50.0, 50.0);
        let mut row = ChoicesRow::open(p, BOUNDS, choices("0 a b"));
        row.track(Point::new(420.0, 170.0));
        assert_eq!(row.highlighted, Some(1));
    }

    #[test]
    fn sliding_far_below_cancels() {
        let key = Rect::new(100.0, 200.0, 50.0, 50.0);
        let mut row = ChoicesRow::open(key, BOUNDS, choices("3 é"));
        // Anywhere on the key, at its bottom edge and in the gap just below
        // it, the finger still picks a cell.
        for y in [205.0, 240.0, 250.0, 262.0, 275.0] {
            row.track(Point::new(120.0, y));
            assert_eq!(row.highlighted, Some(0), "y = {y}");
        }
        // Clearly below the key (more than half a key under it): cancel.
        row.track(Point::new(120.0, 276.0));
        assert_eq!(row.highlighted, None);
        row.track(Point::new(120.0, 320.0));
        assert_eq!(row.highlighted, None);
        // Coming back up picks a cell again.
        row.track(Point::new(160.0, 170.0));
        assert_eq!(row.highlighted, Some(1));
    }
}
