//! Routes a tapped key's [`KeyAction`]: view switches and modifier latches go
//! to the view/latch controllers; every other action is a keystroke emitted
//! through the input method (preferred) or the virtual keyboard (fallback),
//! consuming any latched modifier. Control keys ([`repeats`]) are held down
//! for as long as the finger is, so the client's key repeat runs. Each
//! keystroke also updates the [`TypingContext`], handing finished words to the
//! active profile's recorder.

use mecha_wayland::prelude::*;
use xkbcommon::xkb::Keysym;

use crate::latch::{LatchedState, consume_latch, toggle_latch};
use crate::layout::{KeyAction, Modifier};
use crate::predict::{Prediction, TypingContext};
use crate::protocol::input_method::{self, InputMethodState};
use crate::protocol::virtual_keyboard::{self, VirtualKeyboardState};
use crate::view::switch_view;
use crate::visibility::KeyboardVisibilityExt;
use crate::widget::keyboard::Keyboard;
use crate::zone::toggle_exclusive_zone;

/// Route a tapped key's action. View switches and modifier latches mutate the
/// Keyboard widget state; every other action is a keystroke the virtual
/// keyboard emits (consuming any latched modifier).
pub(crate) fn dispatch(ctx: &mut Context<'_, Keyboard>, action: &KeyAction) {
    match action {
        KeyAction::SetView(name) => {
            switch_view(ctx, name);
        }
        KeyAction::ToggleView { lock, unlock } => {
            let target = if ctx.me().current_view_name() == lock {
                unlock
            } else {
                lock
            };
            switch_view(ctx, target);
        }
        KeyAction::LatchModifier(m) => {
            toggle_latch(ctx, *m);
        }
        KeyAction::EmitKeysym(ks) => {
            let latched_mask = latched_mask(ctx);
            // Printable keysyms commit via IM2; control keysyms fall back to
            // the virtual-keyboard-v1 keysym transport. A latched modifier
            // forces the keycode path — IM2 commits plain text and would drop
            // the modifier.
            let text = virtual_keyboard::keysym_text(*ks);
            if latched_mask == 0
                && let Some(text) = &text
                && im_commit_text(ctx, text)
            {
                tracing::info!(input = %text, "input method");
            } else {
                vk_emit_keysym(ctx, *ks, latched_mask);
            }
            consume_latch(ctx);
            match text {
                Some(text) if latched_mask == 0 => note_text(ctx, &text),
                Some(_) => ctx.resource_mut::<TypingContext>().clear(),
                None => note_control(ctx, *ks),
            }
        }
        KeyAction::EmitText(text) => {
            let latched_mask = latched_mask(ctx);
            if latched_mask == 0 && im_commit_text(ctx, text) {
                tracing::info!("Input method: {text}");
            } else {
                for ch in text.chars() {
                    vk_emit_keysym(ctx, Keysym::from_char(ch), latched_mask);
                }
            }
            consume_latch(ctx);
            if latched_mask == 0 {
                note_text(ctx, text);
            } else {
                ctx.resource_mut::<TypingContext>().clear();
            }
        }
        KeyAction::HideKeyboard => {
            ctx.set_keyboard_visibility(false);
        }
        KeyAction::ToggleExclusiveZone => {
            toggle_exclusive_zone(ctx);
        }
        KeyAction::Unhandled(name) => {
            tracing::info!(action = %name, "tapped key with no wired action");
        }
    }
}

/// Whether a key with this action is held down while pressed (so the focused
/// client repeats it) rather than tapped on click: non-printable keysyms such
/// as BackSpace, Delete and the arrows. Printable keys stay tap-only.
pub(crate) fn repeats(action: &KeyAction) -> bool {
    matches!(action, KeyAction::EmitKeysym(ks) if virtual_keyboard::keysym_text(*ks).is_none())
}

/// Press a [`repeats`] key's keysym through the virtual keyboard and leave it
/// down until [`release`], consuming any latched modifier. The latch is
/// cleared now but its mask stays applied until the key is released.
pub(crate) fn hold(ctx: &mut Context<'_, Keyboard>, ks: Keysym) {
    let latched_mask = latched_mask(ctx);
    {
        let (mut vk, mut wl) = ctx.fetch::<(ResMut<VirtualKeyboardState>, ResMut<Wayland>)>();
        virtual_keyboard::press_keysym(&mut vk, &mut wl, ks, latched_mask);
    }
    consume_latch(ctx);
    note_control(ctx, ks);
}

/// Release the key [`hold`] pressed, if any.
pub(crate) fn release(ctx: &mut Context<'_, Keyboard>) {
    let (mut vk, mut wl) = ctx.fetch::<(ResMut<VirtualKeyboardState>, ResMut<Wayland>)>();
    virtual_keyboard::release_held(&mut vk, &mut wl);
}

/// The focused field's content purpose and hints, which pick the prediction
/// profile.
pub(crate) fn field_kind(
    ctx: &Context<'_, Keyboard>,
) -> (ZwpTextInputV3ContentPurpose, ZwpTextInputV3ContentHint) {
    let field = &ctx.resource::<InputMethodState>().current;
    (field.purpose, field.hint)
}

/// Finish the word being typed, handing it to the recorder.
pub(crate) fn end_word(ctx: &mut Context<'_, Keyboard>) {
    let word = ctx.resource_mut::<TypingContext>().end_word();
    record(ctx, word);
}

/// Text was committed: extend the word, recording any it finished.
fn note_text(ctx: &mut Context<'_, Keyboard>, text: &str) {
    let words = ctx.resource_mut::<TypingContext>().committed(text);
    record(ctx, words);
}

/// A non-printable keysym was sent (tapped, or the start of a hold): BackSpace
/// deletes one character, Return and Tab finish the word, anything else (an
/// arrow, Delete, Home, ...) moves the cursor somewhere unknown, so the word
/// is dropped.
fn note_control(ctx: &mut Context<'_, Keyboard>, ks: Keysym) {
    match ks {
        Keysym::BackSpace => ctx.resource_mut::<TypingContext>().backspace(),
        Keysym::Return | Keysym::Tab => end_word(ctx),
        _ => ctx.resource_mut::<TypingContext>().clear(),
    }
}

/// Hand finished words to the recorder, if the focused field's profile allows.
fn record(ctx: &mut Context<'_, Keyboard>, words: impl IntoIterator<Item = String>) {
    let (purpose, hint) = field_kind(ctx);
    let mut prediction = ctx.resource_mut::<Prediction>();
    if !prediction.active(purpose, hint).0.may_record {
        return;
    }
    for word in words {
        prediction.recorder.record(&word);
    }
}

/// The combined modifier mask of every currently-latched modifier, to OR into
/// a keystroke. Reads from the [`LatchedState`] resource.
fn latched_mask(ctx: &Context<'_, Keyboard>) -> u32 {
    let latched = ctx.resource::<LatchedState>();
    let masks = &ctx.resource::<VirtualKeyboardState>().mod_masks;
    Modifier::ALL
        .into_iter()
        .filter(|&m| latched.get(m))
        .filter_map(|m| masks.get(&m).copied())
        .fold(0, |acc, mask| acc | mask)
}

/// Commit `text` through the input method when it's bound and active.
fn im_commit_text(ctx: &mut Context<'_, Keyboard>, text: &str) -> bool {
    let (st, mut wl) = ctx.fetch::<(Res<InputMethodState>, ResMut<Wayland>)>();
    input_method::commit_text(st, &mut wl, text)
}

/// Send one keysym as a keycode down+up through the virtual keyboard.
fn vk_emit_keysym(ctx: &mut Context<'_, Keyboard>, ks: Keysym, latched_mask: u32) {
    let (mut vk, mut wl) = ctx.fetch::<(ResMut<VirtualKeyboardState>, ResMut<Wayland>)>();
    virtual_keyboard::emit_keysym(&mut vk, &mut wl, ks, latched_mask)
}
