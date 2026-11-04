//! Shift as state: tap for one capital, double tap or hold for caps lock.
//! Letter keys are relabelled and uppercased from this state; there is no
//! separate "upper" view.

use std::time::Duration;

use mecha_wayland::prelude::*;

use crate::layout::KeyAction;
use crate::modifiers::latch::repaint_function_key;
use crate::touch::single_char;
use crate::widget::keyboard::Keyboard;

/// Two taps closer than this lock shift.
pub(crate) const DOUBLE_TAP_MS: u32 = 300;
/// How long ⇧ is held to lock shift.
pub(crate) const HOLD_LOCK: Duration = Duration::from_millis(1200);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum ShiftState {
    #[default]
    Off,
    /// Capitalises the next letter only.
    OneShot,
    /// Caps lock.
    Locked,
}

#[derive(Default)]
pub(crate) struct Shift {
    state: ShiftState,
    last_tap_ms: Option<u32>,
    /// The press now down was held long enough to lock; its release is not a tap.
    hold_fired: bool,
}
impl Resource for Shift {}

impl Shift {
    pub(crate) fn state(&self) -> ShiftState {
        self.state
    }

    /// A tap on ⇧ at `now_ms`: Off to one-shot; a second tap within
    /// [`DOUBLE_TAP_MS`] locks; a slower one, or a tap while locked, turns off.
    pub(crate) fn tap(&mut self, now_ms: u32) -> ShiftState {
        self.state = match self.state {
            ShiftState::Off => {
                self.last_tap_ms = Some(now_ms);
                ShiftState::OneShot
            }
            ShiftState::OneShot => {
                let quick = self
                    .last_tap_ms
                    .is_some_and(|t| now_ms.wrapping_sub(t) <= DOUBLE_TAP_MS);
                self.last_tap_ms = None;
                if quick {
                    ShiftState::Locked
                } else {
                    ShiftState::Off
                }
            }
            ShiftState::Locked => {
                self.last_tap_ms = None;
                ShiftState::Off
            }
        };
        self.state
    }

    /// ⇧ went down.
    pub(crate) fn press(&mut self) {
        self.hold_fired = false;
    }

    /// ⇧ was held long enough: lock.
    pub(crate) fn hold(&mut self) -> ShiftState {
        self.hold_fired = true;
        self.last_tap_ms = None;
        self.state = ShiftState::Locked;
        self.state
    }

    /// ⇧ came up on its key at `now_ms`: a tap, unless the hold already locked.
    pub(crate) fn release(&mut self, now_ms: u32) -> ShiftState {
        if std::mem::take(&mut self.hold_fired) {
            self.state
        } else {
            self.tap(now_ms)
        }
    }

    /// A letter was typed: a one-shot shift is spent.
    pub(crate) fn typed_letter(&mut self) -> ShiftState {
        if self.state == ShiftState::OneShot {
            self.state = ShiftState::Off;
        }
        self.state
    }

    /// The focused field changed.
    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

/// ⇧ went down: start the hold-to-lock timer.
pub(crate) fn pressed(ctx: &mut Context<'_, Keyboard>) {
    ctx.resource_mut::<Shift>().press();
    let id = ctx.me().shift_timer;
    ctx.start_timer_for(id, HOLD_LOCK);
}

/// ⇧ is no longer down (released or slid off): stop the timer.
pub(crate) fn unpressed(ctx: &mut Context<'_, Keyboard>) {
    let id = ctx.me().shift_timer;
    ctx.cancel_timer(id);
}

/// The hold timer fired.
pub(crate) fn held(ctx: &mut Context<'_, Keyboard>) {
    ctx.resource_mut::<Shift>().hold();
    refresh(ctx);
}

/// ⇧ was tapped (released on its key) at `now_ms`.
pub(crate) fn released(ctx: &mut Context<'_, Keyboard>, now_ms: u32) {
    ctx.resource_mut::<Shift>().release(now_ms);
    refresh(ctx);
}

/// A letter was typed: spend a one-shot shift.
pub(crate) fn typed_letter(ctx: &mut Context<'_, Keyboard>) {
    let before = ctx.resource::<Shift>().state();
    if ctx.resource_mut::<Shift>().typed_letter() != before {
        refresh(ctx);
    }
}

/// The lowercase letter a key types, if it follows shift: a key that already
/// types a capital keeps its label and output.
pub(crate) fn shiftable_letter(action: &KeyAction) -> Option<char> {
    match action {
        KeyAction::EmitKeysym(_) | KeyAction::EmitText(_) => {
            single_char(action).filter(char::is_ascii_lowercase)
        }
        _ => None,
    }
}

/// The letter `c` types in `state`.
pub(crate) fn cased(c: char, state: ShiftState) -> char {
    if state == ShiftState::Off {
        c
    } else {
        c.to_ascii_uppercase()
    }
}

/// Show the current shift state: relabel every letter key in every view and
/// repaint the ⇧ keys.
pub(crate) fn refresh(ctx: &mut Context<'_, Keyboard>) {
    let state = ctx.resource::<Shift>().state();
    let letters: Vec<(Handle<_>, char)> = ctx
        .me()
        .keys
        .iter()
        .flatten()
        .filter_map(|k| shiftable_letter(&k.action).map(|c| (k.node, c)))
        .collect();
    for (node, c) in letters {
        let label = ctx.at(node).unwrap().me().label;
        if let crate::widget::key::Label::Text(h) = label {
            ctx.at(h).unwrap().set_text(cased(c, state).to_string());
        }
    }
    let (keys, radius) = (ctx.me().shift_keys.clone(), ctx.me().radius);
    for k in keys {
        repaint_function_key(ctx, k, state != ShiftState::Off, radius);
        if let Some(bar) = ctx.at(k).unwrap().me().lock_bar {
            let display = if state == ShiftState::Locked {
                Display::Flex
            } else {
                Display::Hidden
            };
            ctx.at(bar).unwrap().set_display(display);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_from_off_is_one_shot() {
        let mut s = Shift::default();
        assert_eq!(s.tap(0), ShiftState::OneShot);
        assert_eq!(s.state(), ShiftState::OneShot);
    }

    #[test]
    fn letter_after_one_shot_returns_to_off() {
        let mut s = Shift::default();
        s.tap(0);
        assert_eq!(s.typed_letter(), ShiftState::Off);
    }

    #[test]
    fn double_tap_locks() {
        let mut s = Shift::default();
        s.tap(0);
        assert_eq!(s.tap(250), ShiftState::Locked);
    }

    #[test]
    fn slow_second_tap_turns_off() {
        let mut s = Shift::default();
        s.tap(0);
        assert_eq!(s.tap(400), ShiftState::Off);
    }

    #[test]
    fn locked_survives_letters() {
        let mut s = Shift::default();
        s.tap(0);
        s.tap(100);
        assert_eq!(s.typed_letter(), ShiftState::Locked);
    }

    #[test]
    fn tap_while_locked_turns_off() {
        let mut s = Shift::default();
        s.tap(0);
        s.tap(100);
        assert_eq!(s.tap(1000), ShiftState::Off);
    }

    #[test]
    fn hold_locks() {
        let mut s = Shift::default();
        assert_eq!(s.hold(), ShiftState::Locked);
    }

    #[test]
    fn release_after_hold_is_not_a_tap() {
        let mut s = Shift::default();
        s.press();
        s.hold();
        assert_eq!(s.release(1300), ShiftState::Locked);
        // The next press-release is an ordinary tap again.
        s.press();
        assert_eq!(s.release(2000), ShiftState::Off);
    }

    #[test]
    fn release_without_hold_taps() {
        let mut s = Shift::default();
        s.press();
        assert_eq!(s.release(50), ShiftState::OneShot);
    }

    #[test]
    fn hold_from_one_shot_then_release_stays_locked() {
        let mut s = Shift::default();
        s.press();
        s.release(0);
        s.press();
        s.hold();
        assert_eq!(s.release(1300), ShiftState::Locked);
    }

    #[test]
    fn reset_clears() {
        let mut s = Shift::default();
        s.tap(0);
        s.tap(100);
        s.reset();
        assert_eq!(s.state(), ShiftState::Off);
    }
}
