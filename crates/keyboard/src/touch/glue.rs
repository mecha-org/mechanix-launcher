//! Wires touches on the keyboard container to keys: each contact event is
//! resolved against the current view's laid-out key rectangles (biased by the
//! next-letter prior), run through the [`TouchTracker`], and its outputs
//! become keystrokes ([`dispatch`], [`hold`], [`release`]) and
//! [`KeyFeedback`] for the key widgets.
//!
//! The tracker's key indices always refer to the current view: when the view
//! changes under an active contact, that contact is cancelled against the old
//! view (its highlight and hold end; nothing is typed). A press that lands
//! before its view is laid out waits in a [`Deferral`] until it is.

use std::collections::HashMap;

use mecha_wayland::prelude::*;
use xkbcommon::xkb::Keysym;

use super::KeyFeedback;
use super::defer::{Contact, Deferral, PendingPress, laid_out};
use super::resolve::{KeyGeom, ResolveParams, resolve};
use super::tracker::{KeyRef, TouchOut, TouchTracker};
use crate::input::{dispatch, field_kind, hold, release, repeats};
use crate::layout::KeyAction;
use crate::predict::{FLAT, LetterPrior, Prediction, TypingContext};
use crate::widget::keyboard::Keyboard;

/// Bookkeeping the input layer keeps besides the tracker.
pub(crate) struct TouchState {
    /// Last event time per contact, for `PressCancelled` (which carries none).
    last_time: HashMap<ContactId, u32>,
    /// The keysym a running hold is pressing.
    held: Option<Keysym>,
    /// The next-letter prior for the current contact, taken at its press:
    /// the word can't change while a finger is down.
    prior: LetterPrior,
    /// The active profile's prediction strength, taken with `prior`.
    strength: f32,
    /// Whether the active profile shows the key pop-up, taken with `prior`.
    popup: bool,
    /// A press waiting for its view's layout.
    deferral: Deferral,
}

impl Default for TouchState {
    fn default() -> Self {
        Self {
            last_time: HashMap::new(),
            held: None,
            prior: FLAT,
            strength: 0.0,
            popup: false,
            deferral: Deferral::default(),
        }
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
        if let Some(geoms) = gate(ctx, Contact::Press(p)) {
            press(ctx, &geoms, p);
        }
    });

    s.on::<PressMoved>(me, |ctx, e| {
        ctx.me().touch.last_time.insert(e.contact, e.time);
        if let Some(geoms) = gate(ctx, Contact::Moved(e.contact)) {
            let key = key_at(ctx, &geoms, e.position);
            let out = ctx.me().tracker.moved(e.contact, key, e.time);
            run(ctx, out, e.time);
        }
    });

    s.on::<Release>(me, |ctx, e| {
        ctx.me().touch.last_time.remove(&e.contact);
        if gate(ctx, Contact::Release(e.contact)).is_some() {
            let out = ctx.me().tracker.release(e.contact, e.time);
            run(ctx, out, e.time);
        }
    });

    s.on::<PressCancelled>(me, |ctx, e| {
        let time = ctx.me().touch.last_time.remove(&e.contact).unwrap_or(0);
        if gate(ctx, Contact::Cancel(e.contact)).is_some() {
            let out = ctx.me().tracker.cancel(e.contact, time);
            run(ctx, out, time);
        }
    });
}

/// Pass `event` through the deferral: replays a press that was waiting for
/// the layout, then returns the current view's key geometry if the event
/// itself should be acted on.
fn gate(ctx: &mut Context<'_, Keyboard>, event: Contact) -> Option<Vec<KeyGeom>> {
    let geoms = key_geoms(ctx);
    let ready = laid_out(&geoms);
    let gate = ctx.me().touch.deferral.gate(event, ready);
    if let Some(p) = gate.replay {
        press(ctx, &geoms, p);
    }
    gate.forward.then_some(geoms)
}

/// A press the view is laid out for: take the prior, resolve and track it.
fn press(ctx: &mut Context<'_, Keyboard>, geoms: &[KeyGeom], p: PendingPress) {
    let (prior, strength, popup) = predict(ctx);
    let touch = &mut ctx.me().touch;
    touch.prior = prior;
    touch.strength = strength;
    touch.popup = popup;
    let key = key_at(ctx, geoms, p.position);
    let out = ctx.me().tracker.press(p.contact, key, p.time);
    run(ctx, out, p.time);
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
    let (purpose, hint) = field_kind(ctx);
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

/// The key of the current view (whose keys are `geoms`) a touch at `p`
/// means, if any, using the prior taken at the contact's press.
fn key_at(ctx: &mut Context<'_, Keyboard>, geoms: &[KeyGeom], p: Point) -> Option<KeyRef> {
    let view = ctx.me().current_view;
    let touch = &ctx.me().touch;
    let params = ResolveParams::for_keys(geoms, touch.strength);
    let index = resolve(geoms, p, &touch.prior, &params)?;
    let repeats = repeats(&ctx.me().keys[view][index].action);
    Some(KeyRef { index, repeats })
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
