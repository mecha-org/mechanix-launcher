use crate::style::size::OPEN_OFFSET;
use crate::style::swipe::{DISMISS, REVEAL_MAX, REVEAL_MIN, SLOP};
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
    Swiping(PressState),
    Swiped,
}

struct PressState {
    contact: ContactId,
    origin: Point,
    card_offset: f32,
}

#[derive(Debug, PartialEq)]
pub enum Moved {
    Ignore,
    Claim,
    Slide(f32),
}

#[derive(Debug, PartialEq)]
pub enum Settle {
    Ignore,
    Rest(f32),
    Dismiss,
}

impl CardDrag {
    pub fn offset(&self) -> f32 {
        self.offset
    }

    pub fn set_offset(&mut self, offset: f32) {
        self.offset = offset;
    }

    pub fn press(&mut self, contact: ContactId, pos: Point) {
        if matches!(self.gesture, Gesture::Pressing(_) | Gesture::Swiping(_)) {
            return;
        }
        self.gesture = Gesture::Pressing(PressState {
            contact,
            origin: pos,
            card_offset: self.offset,
        });
    }

    pub fn update(&mut self, contact: ContactId, pos: Point, width: f32) -> Moved {
        let (press, swiping) = match &self.gesture {
            Gesture::Pressing(p) => (p, false),
            Gesture::Swiping(p) => (p, true),
            Gesture::Idle | Gesture::Swiped => return Moved::Ignore,
        };
        if press.contact != contact {
            return Moved::Ignore;
        }

        let (dx, dy) = (pos.x - press.origin.x, pos.y - press.origin.y);

        if !swiping {
            if dx.abs() <= SLOP || dx.abs() <= dy.abs() {
                return Moved::Ignore;
            }
            return Moved::Claim;
        }

        self.offset = (press.card_offset + dx).clamp(-width, 0.0);
        Moved::Slide(self.offset)
    }

    pub fn start_swipe(&mut self) {
        self.gesture = match std::mem::take(&mut self.gesture) {
            Gesture::Pressing(p) => Gesture::Swiping(p),
            other => other,
        };
    }

    pub fn release(&mut self, contact: ContactId, width: f32) -> Settle {
        match &self.gesture {
            Gesture::Pressing(p) if p.contact == contact => {
                self.gesture = Gesture::Idle;
                Settle::Ignore
            }
            Gesture::Swiping(p) if p.contact == contact => {
                self.gesture = Gesture::Swiped;
                self.settle(width)
            }
            _ => Settle::Ignore,
        }
    }

    fn settle(&mut self, width: f32) -> Settle {
        let pulled = if width > 0.0 {
            -self.offset / width
        } else {
            0.0
        };
        if pulled >= DISMISS {
            return Settle::Dismiss;
        }
        self.offset = if pulled >= REVEAL_MIN {
            -OPEN_OFFSET.min(width * REVEAL_MAX)
        } else {
            0.0
        };
        Settle::Rest(self.offset)
    }

    pub fn cancel(&mut self) -> f32 {
        if let Gesture::Pressing(p) | Gesture::Swiping(p) = &self.gesture {
            self.offset = p.card_offset;
        }
        self.gesture = Gesture::Idle;
        self.offset
    }

    pub fn ended_in_swipe(&self) -> bool {
        matches!(self.gesture, Gesture::Swiped)
    }
}
