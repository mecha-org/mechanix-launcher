//! The keyboard surface's shown/hidden state.
//!
//! Driven by input-method `activate`/`deactivate` (applied at `done`), shown
//! by the host with [`show_keyboard`] and hidden by the hide key; all write
//! through
//! [`KeyboardVisibilityExt`]. A [`KeyboardVisibilityChanged`] signal is fanned
//! out as an [`ApplyKeyboardVisibility`] event by a system, which the keyboard
//! surface widget subscribes to.

use mecha_wayland::prelude::*;

/// The keyboard surface's shown/hidden state.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyboardVisibility {
    pub visible: bool,
}
impl Resource for KeyboardVisibility {}

/// `Signal`: the visibility flag changed — fan out [`ApplyKeyboardVisibility`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KeyboardVisibilityChanged;
impl Signal for KeyboardVisibilityChanged {}

/// `Event`: re-apply the current visibility to the surface. Broadcast to the
/// root and every descendant; the keyboard surface widget handles it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ApplyKeyboardVisibility;
impl Event for ApplyKeyboardVisibility {}

/// The single visibility setter: the input-method `done` handler,
/// [`show_keyboard`] and the hide key all go through here. Implemented for `App` (systems) and `Context`
/// (widget handlers) so the same call works at every site.
pub trait KeyboardVisibilityExt {
    /// The current visibility flag.
    fn keyboard_visible(&self) -> bool;
    /// Set the flag and signal the change (no-op if already `visible`).
    fn set_keyboard_visibility(&mut self, visible: bool);
}

impl KeyboardVisibilityExt for App {
    fn keyboard_visible(&self) -> bool {
        self.resource::<KeyboardVisibility>().visible
    }
    fn set_keyboard_visibility(&mut self, visible: bool) {
        if self.resource::<KeyboardVisibility>().visible == visible {
            return;
        }
        self.resource_mut::<KeyboardVisibility>().visible = visible;
        self.signal(KeyboardVisibilityChanged);
        tracing::info!(visible, "keyboard visibility");
    }
}

impl<W: Widget> KeyboardVisibilityExt for Context<'_, W> {
    fn keyboard_visible(&self) -> bool {
        self.resource::<KeyboardVisibility>().visible
    }
    fn set_keyboard_visibility(&mut self, visible: bool) {
        if self.resource::<KeyboardVisibility>().visible == visible {
            return;
        }
        self.resource_mut::<KeyboardVisibility>().visible = visible;
        self.signal(KeyboardVisibilityChanged);
        tracing::info!(visible, "keyboard visibility");
    }
}

/// Show the keyboard, e.g. from a nav corner-swipe action. Needs the
/// [`KeyboardModule`](crate::KeyboardModule) installed.
pub fn show_keyboard(app: &mut App) {
    app.set_keyboard_visibility(true);
}

/// Hide the keyboard, e.g. on the way home, so its exclusive zone goes and
/// the app under it gets its full size back. Needs the
/// [`KeyboardModule`](crate::KeyboardModule) installed.
pub fn hide_keyboard(app: &mut App) {
    app.set_keyboard_visibility(false);
}

/// On [`KeyboardVisibilityChanged`], broadcast [`ApplyKeyboardVisibility`] to
/// the root and every descendant so the surface widget re-applies `Display`.
fn on_keyboard_visibility_changed(app: &mut App, _: &KeyboardVisibilityChanged) {
    let mut targets = vec![app.root()];
    targets.extend(app.tree().descendants(app.root()));
    app.emit(ApplyKeyboardVisibility, targets);
}

/// Insert the visibility resource (starting hidden) and the signal→event bridge.
pub(crate) fn install(app: &mut App) {
    app.insert_resource(KeyboardVisibility { visible: false });
    app.system(on_keyboard_visibility_changed);
}
