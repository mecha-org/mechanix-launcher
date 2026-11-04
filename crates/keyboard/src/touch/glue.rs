//! Wires touches on the keyboard container to keys: each contact event is
//! resolved against the current view's laid-out key rectangles (biased by the
//! next-letter prior), run through the [`TouchTracker`], and its outputs
//! become keystrokes ([`dispatch`], [`hold`], [`release`]) and
//! [`KeyFeedback`] for the key widgets.
//!
//! The tracker's key indices always refer to the current view: when the view
//! changes under an active contact, that contact is cancelled against the old
//! view (its highlight and hold end; nothing is typed). The order of
//! operations per event (deferral, finishing an earlier finger, resolving)
//! lives in [`flow`]; this file binds it to the widgets.

use std::collections::HashMap;

use mecha_wayland::prelude::*;
use xkbcommon::xkb::Keysym;

use super::KeyFeedback;
use super::defer::PendingPress;
use super::flow::{self, Board, Flow};
use super::resolve::KeyGeom;
use super::tracker::{TouchOut, TouchTracker};
use crate::field::purpose_hint;
use crate::input::{dispatch, hold, release, repeats};
use crate::layout::KeyAction;
use crate::predict::{LetterPrior, Prediction, TypingContext};
use crate::widget::keyboard::Keyboard;

/// Bookkeeping the input layer keeps besides the tracker.
#[derive(Default)]
pub(crate) struct TouchState {
    /// Last event time per contact, for `PressCancelled` (which carries none).
    last_time: HashMap<ContactId, u32>,
    /// The keysym a running hold is pressing.
    held: Option<Keysym>,
    /// Whether the active profile shows the key pop-up, taken at the press.
    popup: bool,
    /// Deferral and per-contact prior.
    flow: Flow,
}

/// The keyboard as the [`flow`] sees it.
struct Kb<'c, 'a>(&'c mut Context<'a, Keyboard>);

impl Board for Kb<'_, '_> {
    fn geoms(&mut self) -> Vec<KeyGeom> {
        key_geoms(self.0)
    }
    fn predict(&mut self) -> (LetterPrior, f32) {
        let (prior, strength, popup) = predict(self.0);
        self.0.me().touch.popup = popup;
        (prior, strength)
    }
    fn repeats(&mut self, index: usize) -> bool {
        let me = self.0.me();
        repeats(&me.keys[me.current_view][index].action)
    }
    fn tracker(&mut self) -> &mut TouchTracker {
        &mut self.0.me().tracker
    }
    fn flow(&mut self) -> &mut Flow {
        &mut self.0.me().touch.flow
    }
    fn act(&mut self, out: Vec<TouchOut>, time: u32) {
        run(self.0, out, time);
    }
}

/// Route every touch on the keyboard container `me` through the input layer.
/// Registered on the container only: the window's `bottom_gap` strip below it
/// gets no handler, so taps there stay dead.
pub(crate) fn attach(s: &mut Spawner<'_, Keyboard>, me: Handle<Keyboard>) {
    s.on::<Press>(me, |ctx, e| {
        ctx.me().touch.last_time.insert(e.contact, e.time);
        let p = PendingPress {
            contact: e.contact,
            position: e.position,
            time: e.time,
        };
        flow::press(&mut Kb(ctx), p);
    });

    s.on::<PressMoved>(me, |ctx, e| {
        ctx.me().touch.last_time.insert(e.contact, e.time);
        flow::moved(&mut Kb(ctx), e.contact, e.position, e.time);
    });

    s.on::<Release>(me, |ctx, e| {
        ctx.me().touch.last_time.remove(&e.contact);
        // Commits the last resolved (highlighted) key, not a fresh resolve
        // at the release position: by design.
        flow::release(&mut Kb(ctx), e.contact, e.time);
    });

    s.on::<PressCancelled>(me, |ctx, e| {
        let time = ctx.me().touch.last_time.remove(&e.contact).unwrap_or(0);
        flow::cancel(&mut Kb(ctx), e.contact, time);
    });
}

/// The view changed from `old` outside a touch event (a focused field asked
/// for another view): cancel any active contact against `old`.
pub(crate) fn view_switched(ctx: &mut Context<'_, Keyboard>, old: usize) {
    let me = ctx.me();
    let time = me
        .tracker
        .contact()
        .and_then(|c| me.touch.last_time.get(&c).copied())
        .unwrap_or(0);
    settle(ctx, old, time);
}

/// The prior over the next letter, the prediction strength and whether the
/// pop-up is on, for the focused field and the word typed so far.
fn predict(ctx: &Context<'_, Keyboard>) -> (LetterPrior, f32, bool) {
    let (purpose, hint) = purpose_hint(ctx);
    let (profile, predictor) = ctx.resource::<Prediction>().active(purpose, hint);
    let prior = predictor.prior(&ctx.resource::<TypingContext>().prefix());
    (prior, profile.strength, profile.popup)
}

/// The current view's keys, with their laid-out rectangles.
fn key_geoms(ctx: &mut Context<'_, Keyboard>) -> Vec<KeyGeom> {
    let view = ctx.me().current_view;
    let keys: Vec<(Handle<_>, Option<char>)> = ctx.me().keys[view]
        .iter()
        .map(|k| (k.node, k.letter))
        .collect();
    keys.into_iter()
        .map(|(node, letter)| {
            let rect = ctx
                .at(node)
                .and_then(|c| c.component::<ComputedLayout>().map(|l| l.rect))
                .unwrap_or_default();
            KeyGeom { rect, letter }
        })
        .collect()
}

/// Act on a batch of tracker outputs against the current view, then cancel
/// the active contact if acting switched the view.
fn run(ctx: &mut Context<'_, Keyboard>, out: Vec<TouchOut>, time: u32) {
    let view = ctx.me().current_view;
    apply(ctx, view, out);
    settle(ctx, view, time);
}

/// If the view is no longer `view`, cancel the active contact, whose key
/// indices refer to `view`.
fn settle(ctx: &mut Context<'_, Keyboard>, view: usize, time: u32) {
    let me = ctx.me();
    let now = me.current_view;
    let out = finish_stale(&mut me.tracker, view, now, time);
    apply(ctx, view, out);
}

/// The outputs that end `tracker`'s active contact if its view `then` is no
/// longer the current view `now`; none if the view is unchanged.
fn finish_stale(tracker: &mut TouchTracker, then: usize, now: usize, time: u32) -> Vec<TouchOut> {
    if then == now {
        return Vec::new();
    }
    tracker.cancel_active(time)
}

/// Act on the tracker's outputs, in order; indices refer to `view`.
fn apply(ctx: &mut Context<'_, Keyboard>, view: usize, out: Vec<TouchOut>) {
    for o in out {
        match o {
            TouchOut::Pressed(i) => feedback(ctx, view, i, |key| KeyFeedback::Pressed { key }),
            TouchOut::Unpressed(i) => feedback(ctx, view, i, |key| KeyFeedback::Released { key }),
            TouchOut::Commit(i) => {
                if let Some(action) = entry_action(ctx, view, i) {
                    dispatch(ctx, &action);
                }
            }
            TouchOut::Hold(i) => {
                if let Some(KeyAction::EmitKeysym(ks)) = entry_action(ctx, view, i) {
                    ctx.me().touch.held = Some(ks);
                    hold(ctx, ks);
                }
            }
            TouchOut::StopHold { held_ms } => {
                release(ctx);
                if ctx.me().touch.held.take() == Some(Keysym::BackSpace) {
                    ctx.resource_mut::<TypingContext>().backspace_held(held_ms);
                }
            }
        }
    }
}

/// The action of key `i` in `view`, if it exists.
fn entry_action(ctx: &mut Context<'_, Keyboard>, view: usize, i: usize) -> Option<KeyAction> {
    ctx.me().keys[view].get(i).map(|k| k.action.clone())
}

/// Tell the UI about key `i` of `view`: the key itself and, while the
/// contact's profile has the pop-up on, the pop-up.
fn feedback(
    ctx: &mut Context<'_, Keyboard>,
    view: usize,
    i: usize,
    make: impl FnOnce(NodeId) -> KeyFeedback,
) {
    let Some(node) = ctx.me().keys[view].get(i).map(|k| k.node.id()) else {
        return;
    };
    let me = ctx.me();
    if me.touch.popup {
        let popup = me.popup.id();
        ctx.emit(make(node), [node, popup]);
    } else {
        ctx.emit(make(node), [node]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::touch::tracker::KeyRef;

    const A: ContactId = ContactId::Touch(1);

    #[test]
    fn view_change_cancels_active_contact() {
        let mut t = TouchTracker::default();
        t.press(
            A,
            Some(KeyRef {
                index: 4,
                repeats: false,
            }),
            0,
        );
        assert_eq!(finish_stale(&mut t, 0, 0, 10), vec![]);
        assert_eq!(finish_stale(&mut t, 0, 1, 10), vec![TouchOut::Unpressed(4)]);
        assert_eq!(t.release(A, 20), vec![]);
    }

    #[test]
    fn view_change_without_contact_does_nothing() {
        let mut t = TouchTracker::default();
        assert_eq!(finish_stale(&mut t, 0, 1, 10), vec![]);
    }
}
