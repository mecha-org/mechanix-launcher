//! Wires touches on the keyboard container to keys: each contact event is
//! resolved against the current view's laid-out key rectangles (biased by the
//! next-letter prior), run through the [`TouchTracker`](super::TouchTracker),
//! and its outputs become keystrokes ([`dispatch`], [`hold`], [`release`]) and
//! [`KeyFeedback`] for the key widgets.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use mecha_wayland::prelude::*;
use xkbcommon::xkb::Keysym;

use super::KeyFeedback;
use super::resolve::{KeyGeom, ResolveParams, resolve};
use super::tracker::{KeyRef, TouchOut};
use crate::input::{dispatch, field_kind, hold, release, repeats};
use crate::layout::KeyAction;
use crate::predict::{Prediction, TypingContext};
use crate::widget::keyboard::Keyboard;

/// Bookkeeping the handlers share that the tracker doesn't keep.
#[derive(Default)]
struct Glue {
    /// Last event time per contact, for `PressCancelled` (which carries none).
    last_time: HashMap<ContactId, u32>,
    /// The keysym a running hold is pressing.
    held: Option<Keysym>,
}

/// Route every touch on the keyboard container `me` through the input layer.
/// Registered on the container only: the window's `bottom_gap` strip below it
/// gets no handler, so taps there stay dead.
pub(crate) fn attach(s: &mut Spawner<'_, Keyboard>, me: Handle<Keyboard>) {
    let glue = Rc::new(RefCell::new(Glue::default()));

    let g = glue.clone();
    s.on::<Press>(me, move |ctx, e| {
        g.borrow_mut().last_time.insert(e.contact, e.time);
        let key = key_at(ctx, e.position);
        let out = ctx.me().tracker.press(e.contact, key, e.time);
        apply(ctx, &g, out);
    });

    let g = glue.clone();
    s.on::<PressMoved>(me, move |ctx, e| {
        g.borrow_mut().last_time.insert(e.contact, e.time);
        let key = key_at(ctx, e.position);
        let out = ctx.me().tracker.moved(e.contact, key, e.time);
        apply(ctx, &g, out);
    });

    let g = glue.clone();
    s.on::<Release>(me, move |ctx, e| {
        g.borrow_mut().last_time.remove(&e.contact);
        let out = ctx.me().tracker.release(e.contact, e.time);
        apply(ctx, &g, out);
    });

    let g = glue;
    s.on::<PressCancelled>(me, move |ctx, e| {
        let time = g.borrow_mut().last_time.remove(&e.contact).unwrap_or(0);
        let out = ctx.me().tracker.cancel(e.contact, time);
        apply(ctx, &g, out);
    });
}

/// The key of the current view a touch at `p` means, if any.
fn key_at(ctx: &mut Context<'_, Keyboard>, p: Point) -> Option<KeyRef> {
    let view = ctx.me().current_view;
    let keys: Vec<(Handle<_>, Option<char>)> = ctx.me().keys[view]
        .iter()
        .map(|k| (k.node, k.letter))
        .collect();
    let geoms: Vec<KeyGeom> = keys
        .into_iter()
        .map(|(node, letter)| {
            let rect = ctx
                .at(node)
                .and_then(|c| c.component::<ComputedLayout>().map(|l| l.rect))
                .unwrap_or_default();
            KeyGeom { rect, letter }
        })
        .collect();

    let (purpose, hint) = field_kind(ctx);
    let (profile, predictor) = ctx.resource::<Prediction>().active(purpose, hint);
    let prior = predictor.prior(&ctx.resource::<TypingContext>().prefix());
    let params = ResolveParams::for_keys(&geoms, profile.strength);

    let index = resolve(&geoms, p, &prior, &params)?;
    let repeats = repeats(&ctx.me().keys[view][index].action);
    Some(KeyRef { index, repeats })
}

/// Act on the tracker's outputs, in order. Indices refer to the current view.
fn apply(ctx: &mut Context<'_, Keyboard>, glue: &RefCell<Glue>, out: Vec<TouchOut>) {
    let view = ctx.me().current_view;
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
                    glue.borrow_mut().held = Some(ks);
                    hold(ctx, ks);
                }
            }
            TouchOut::StopHold { held_ms } => {
                release(ctx);
                if glue.borrow_mut().held.take() == Some(Keysym::BackSpace) {
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

/// Tell the UI about key `i` of `view`. To reach another widget (the pop-up),
/// add its node to the targets.
fn feedback(
    ctx: &mut Context<'_, Keyboard>,
    view: usize,
    i: usize,
    make: impl FnOnce(NodeId) -> KeyFeedback,
) {
    let Some(node) = ctx.me().keys[view].get(i).map(|k| k.node.id()) else {
        return;
    };
    ctx.emit(make(node), [node]);
}
