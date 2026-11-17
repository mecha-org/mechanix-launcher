use crate::style::size::OPEN_OFFSET;
use crate::style::swipe::{DISMISS, EXPAND, REVEAL_MAX, REVEAL_MIN, SLOP};
use mecha_wayland::prelude::{ContactId, Point};

#[derive(Default)]
pub struct CardDrag {
    offset: f32,
    gesture: Gesture,
}

#[derive(Default)]
enum Gesture {
    #[default]
    Idle,
    Pressing(PressState),
    Swiping(PressState, Swiping),
    Swiped,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Swiping {
    Left,
    Right,
    Down,
    Up,
}

fn direction(dx: f32, dy: f32) -> Option<Swiping> {
    if dx.abs() <= SLOP && dy.abs() <= SLOP {
        return None;
    }
    let swiping = if dx.abs() > dy.abs() {
        if dx < 0.0 {
            Swiping::Left
        } else {
            Swiping::Right
        }
    } else if dy > 0.0 {
        Swiping::Down
    } else {
        Swiping::Up
    };
    Some(swiping)
}

struct PressState {
    contact: ContactId,
    origin: Point,
    card_offset: f32,
}

#[derive(Debug, PartialEq)]
pub enum Moved {
    Ignore,
    Claim(Swiping),
    Slide(f32),
}

#[derive(Debug, PartialEq)]
pub enum Settle {
    Ignore,
    Rest(f32),
    Dismiss,
    Expand,
    Collapse,
}

impl CardDrag {
    pub fn offset(&self) -> f32 {
        self.offset
    }

    pub fn set_offset(&mut self, offset: f32) {
        self.offset = offset;
    }

    pub fn press(&mut self, contact: ContactId, pos: Point) {
        if matches!(self.gesture, Gesture::Pressing(_) | Gesture::Swiping(..)) {
            return;
        }
        self.gesture = Gesture::Pressing(PressState {
            contact,
            origin: pos,
            card_offset: self.offset,
        });
    }

    pub fn update(&mut self, contact: ContactId, pos: Point, width: f32) -> Moved {
        match &self.gesture {
            Gesture::Pressing(p) if p.contact == contact => {
                let (dx, dy) = (pos.x - p.origin.x, pos.y - p.origin.y);
                match direction(dx, dy) {
                    Some(s) => Moved::Claim(s),
                    None => Moved::Ignore,
                }
            }
            Gesture::Swiping(p, Swiping::Left | Swiping::Right) if p.contact == contact => {
                let dx = pos.x - p.origin.x;
                self.offset = (p.card_offset + dx).clamp(-width, width);
                Moved::Slide(self.offset)
            }
            _ => Moved::Ignore,
        }
    }

    pub fn start_swipe(&mut self, swiping: Swiping) {
        self.gesture = match std::mem::take(&mut self.gesture) {
            Gesture::Pressing(p) => Gesture::Swiping(p, swiping),
            other => other,
        };
    }

    pub fn release(&mut self, contact: ContactId, pos: Point, width: f32) -> Settle {
        match &self.gesture {
            Gesture::Pressing(p) if p.contact == contact => {
                self.gesture = Gesture::Idle;
                Settle::Ignore
            }
            Gesture::Swiping(p, swiping) if p.contact == contact => {
                let (swiping, dy) = (*swiping, pos.y - p.origin.y);
                self.gesture = Gesture::Swiped;
                match swiping {
                    Swiping::Left | Swiping::Right => self.settle(width),
                    Swiping::Down if dy >= EXPAND => Settle::Expand,
                    Swiping::Up if -dy >= EXPAND => Settle::Collapse,
                    Swiping::Down | Swiping::Up => Settle::Ignore,
                }
            }
            _ => Settle::Ignore,
        }
    }

    fn settle(&mut self, width: f32) -> Settle {
        let pulled = if width > 0.0 {
            self.offset / width
        } else {
            0.0
        };
        if pulled >= DISMISS {
            return Settle::Dismiss;
        }
        self.offset = if -pulled >= REVEAL_MIN {
            -OPEN_OFFSET.min(width * REVEAL_MAX)
        } else {
            0.0
        };
        Settle::Rest(self.offset)
    }

    pub fn cancel(&mut self) -> f32 {
        if let Gesture::Pressing(p) | Gesture::Swiping(p, _) = &self.gesture {
            self.offset = p.card_offset;
        }
        self.gesture = Gesture::Idle;
        self.offset
    }

    pub fn ended_in_swipe(&self) -> bool {
        matches!(self.gesture, Gesture::Swiped)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FINGER: ContactId = ContactId::Touch(0);
    const WIDTH: f32 = 500.0;

    fn pressed() -> CardDrag {
        let mut drag = CardDrag::default();
        drag.press(FINGER, Point::new(100.0, 100.0));
        drag
    }

    fn moved(drag: &mut CardDrag, dx: f32, dy: f32) -> Moved {
        drag.update(FINGER, Point::new(100.0 + dx, 100.0 + dy), WIDTH)
    }

    #[test]
    fn direction_is_none_inside_slop() {
        assert_eq!(direction(SLOP, -SLOP), None);
        assert_eq!(direction(0.0, 0.0), None);
    }

    #[test]
    fn direction_follows_the_larger_axis() {
        assert_eq!(direction(-30.0, 10.0), Some(Swiping::Left));
        assert_eq!(direction(30.0, -10.0), Some(Swiping::Right));
        assert_eq!(direction(10.0, 30.0), Some(Swiping::Down));
        assert_eq!(direction(-10.0, -30.0), Some(Swiping::Up));
    }

    #[test]
    fn a_horizontal_move_claims_with_its_direction() {
        let mut drag = pressed();
        assert_eq!(moved(&mut drag, -5.0, 0.0), Moved::Ignore, "inside slop");
        assert_eq!(moved(&mut drag, -30.0, 5.0), Moved::Claim(Swiping::Left));

        let mut drag = pressed();
        assert_eq!(moved(&mut drag, 30.0, 5.0), Moved::Claim(Swiping::Right));
    }

    #[test]
    fn a_vertical_move_claims_with_its_direction() {
        let mut drag = pressed();
        assert_eq!(moved(&mut drag, 5.0, 30.0), Moved::Claim(Swiping::Down));
        let mut drag = pressed();
        assert_eq!(moved(&mut drag, 5.0, -30.0), Moved::Claim(Swiping::Up));
    }

    fn released_at(swiping: Swiping, dy: f32) -> Settle {
        let mut drag = pressed();
        drag.start_swipe(swiping);
        drag.release(FINGER, Point::new(100.0, 100.0 + dy), WIDTH)
    }

    #[test]
    fn a_down_swipe_past_expand_expands() {
        assert_eq!(released_at(Swiping::Down, EXPAND), Settle::Expand);
        assert_eq!(released_at(Swiping::Down, EXPAND - 1.0), Settle::Ignore);
    }

    #[test]
    fn an_up_swipe_past_expand_collapses() {
        assert_eq!(released_at(Swiping::Up, -EXPAND), Settle::Collapse);
        assert_eq!(released_at(Swiping::Up, -EXPAND + 1.0), Settle::Ignore);
    }

    #[test]
    fn a_vertical_swipe_never_slides_the_card() {
        let mut drag = pressed();
        drag.start_swipe(Swiping::Down);
        assert_eq!(moved(&mut drag, 80.0, 60.0), Moved::Ignore);
        assert_eq!(drag.offset(), 0.0);
    }

    #[test]
    fn a_horizontal_swipe_slides_with_the_finger_and_stays_horizontal() {
        let mut drag = pressed();
        let Moved::Claim(swiping) = moved(&mut drag, -30.0, 0.0) else {
            panic!("expected a claim");
        };
        drag.start_swipe(swiping);
        assert_eq!(moved(&mut drag, -60.0, 0.0), Moved::Slide(-60.0));
        assert_eq!(
            moved(&mut drag, -60.0, 200.0),
            Moved::Slide(-60.0),
            "drifting down keeps it a horizontal swipe"
        );
    }

    #[test]
    fn releasing_a_left_swipe_past_reveal_opens_the_card() {
        let mut drag = pressed();
        drag.start_swipe(Swiping::Left);
        moved(&mut drag, -WIDTH * REVEAL_MIN - 1.0, 0.0);
        assert_eq!(
            drag.release(FINGER, Point::new(100.0, 100.0), WIDTH),
            Settle::Rest(-OPEN_OFFSET.min(WIDTH * REVEAL_MAX))
        );
        assert!(drag.ended_in_swipe());
    }

    #[test]
    fn releasing_a_right_swipe_past_dismiss_dismisses() {
        let mut drag = pressed();
        drag.start_swipe(Swiping::Right);
        moved(&mut drag, WIDTH * DISMISS + 1.0, 0.0);
        assert_eq!(
            drag.release(FINGER, Point::new(100.0, 100.0), WIDTH),
            Settle::Dismiss
        );
    }

    #[test]
    fn a_press_released_without_moving_is_a_tap() {
        let mut drag = pressed();
        assert_eq!(
            drag.release(FINGER, Point::new(100.0, 100.0), WIDTH),
            Settle::Ignore
        );
        assert!(!drag.ended_in_swipe());
    }
}
