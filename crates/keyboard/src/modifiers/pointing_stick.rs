//! The pointing stick's mode: caret (its directions hold plain arrows) or
//! select (Shift+arrows, extending the selection). A tap on the pointing
//! stick toggles it; using any other key, focusing another field and hiding
//! the keyboard reset it to caret. The pointing stick key shows select mode
//! in its latched colours.

use mecha_wayland::prelude::*;

use crate::modifiers::latch::repaint_function_key;
use crate::widget::keyboard::Keyboard;

/// Whether the pointing stick is in select mode.
#[derive(Default)]
pub(crate) struct PointingStickMode {
    select: bool,
}
impl Resource for PointingStickMode {}

impl PointingStickMode {
    /// Whether the directions select (hold Shift) rather than move the caret.
    pub(crate) fn select(&self) -> bool {
        self.select
    }

    /// Flip the mode; returns whether it is now select.
    fn toggle(&mut self) -> bool {
        self.select = !self.select;
        self.select
    }

    /// Back to caret mode; returns whether that changed anything.
    fn reset(&mut self) -> bool {
        std::mem::take(&mut self.select)
    }
}

/// The pointing stick was tapped: flip the mode and repaint its keys.
pub(crate) fn toggle(ctx: &mut Context<'_, Keyboard>) {
    let select = ctx.resource_mut::<PointingStickMode>().toggle();
    repaint(ctx);
    tracing::info!(select, "pointing stick mode");
}

/// Return to caret mode (another key was used, the field changed or the
/// keyboard hid), repainting the pointing stick keys if that changed it.
pub(crate) fn reset(ctx: &mut Context<'_, Keyboard>) {
    if ctx.resource_mut::<PointingStickMode>().reset() {
        repaint(ctx);
        tracing::info!(select = false, "pointing stick mode");
    }
}

/// Paint every pointing stick key, across all views, latched while in select
/// mode.
fn repaint(ctx: &mut Context<'_, Keyboard>) {
    let lit = ctx.resource::<PointingStickMode>().select();
    let radius = ctx.me().radius;
    for k in ctx.me().pointing_stick_keys.clone() {
        repaint_function_key(ctx, k, lit, radius);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_flips_and_reset_returns_to_caret() {
        let mut m = PointingStickMode::default();
        assert!(!m.select());
        assert!(!m.reset(), "resetting caret mode changes nothing");
        assert!(m.toggle());
        assert!(m.select());
        assert!(!m.toggle());
        m.toggle();
        assert!(m.reset());
        assert!(!m.select());
    }
}
