//! The keyboard's exclusive zone: off, the shown keyboard sits over apps; on,
//! it reserves its height so apps shrink above it. On by default, flipped by
//! a `toggle_exclusive_zone` key, which shows the on state in its latched
//! colours. The host suspends it while its home screen is up, and a hidden
//! keyboard has none.

use mecha_wayland::prelude::*;

use crate::latch::repaint_function_key;
use crate::visibility::KeyboardVisibilityExt;
use crate::widget::keyboard::Keyboard;
use crate::widget::surface::KeyboardSurface;

/// Whether the keyboard reserves its height. Starts on, not suspended.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ExclusiveZone {
    /// The user's choice, from the toggle key.
    pub(crate) on: bool,
    /// Held off by the host, e.g. while its home screen is up.
    pub(crate) suspended: bool,
}
impl Resource for ExclusiveZone {}

impl Default for ExclusiveZone {
    fn default() -> Self {
        Self {
            on: true,
            suspended: false,
        }
    }
}

impl ExclusiveZone {
    /// The layer surface's exclusive zone for a shown keyboard `height`
    /// tall: `-1` (overlap everything) when off or suspended.
    pub(crate) fn zone(self, height: f32) -> i32 {
        if self.on && !self.suspended {
            height as i32
        } else {
            -1
        }
    }
}

/// Send the zone the surface should have now: [`ExclusiveZone::zone`] while
/// shown, none while hidden. Sends nothing if it has not changed.
fn send_zone(
    zone: ExclusiveZone,
    visible: bool,
    surface: KeyboardSurface,
    s: &mut Surfaces,
    wl: &mut Wayland,
) {
    let zone = if visible {
        zone.zone(surface.height)
    } else {
        -1
    };
    s.set_exclusive_zone(surface.window, zone, wl);
}

/// Hold the keyboard's exclusive zone off (`true`) or let it follow the
/// toggle again (`false`), for while the host's own screens (home, a panel)
/// cover the apps, so they keep their full height. A shown keyboard updates
/// at once. While held, an app's text input cannot bring the keyboard up;
/// only the host's own focused field can.
/// Needs the [`KeyboardModule`](crate::KeyboardModule) installed.
pub fn suspend_keyboard_zone(app: &mut App, suspended: bool) {
    app.resource_mut::<ExclusiveZone>().suspended = suspended;
    let zone = *app.resource::<ExclusiveZone>();
    let visible = app.keyboard_visible();
    let surface = *app.resource::<KeyboardSurface>();
    if app.has_resource::<Surfaces>() {
        let (mut surfaces, mut wl) = app.query::<(ResMut<Surfaces>, ResMut<Wayland>)>();
        send_zone(zone, visible, surface, &mut surfaces, &mut wl);
    }
    tracing::info!(suspended, "keyboard exclusive zone suspended");
}

/// Map or unmap the keyboard surface for `visible`, with its zone. The zone
/// is cleared while the surface is still mapped, so that commit reaches the
/// compositor (which keeps an unmapped layer's zone) and the apps above grow
/// back. On a show it is sent after the map: a surface that has never drawn
/// is not re-sent its stored zone when it maps.
pub(crate) fn apply_visibility(
    zone: ExclusiveZone,
    visible: bool,
    surface: KeyboardSurface,
    s: &mut Surfaces,
    wl: &mut Wayland,
) {
    if !visible {
        send_zone(zone, false, surface, s, wl);
    }
    s.set_mapped(surface.window, visible, wl);
    if visible {
        send_zone(zone, true, surface, s, wl);
    }
}

/// Flip the zone, send it to the surface and repaint every zone key.
pub(crate) fn toggle_exclusive_zone(ctx: &mut Context<'_, Keyboard>) {
    let on = !ctx.resource::<ExclusiveZone>().on;
    ctx.resource_mut::<ExclusiveZone>().on = on;
    let zone = *ctx.resource::<ExclusiveZone>();
    let visible = ctx.keyboard_visible();
    let surface = *ctx.resource::<KeyboardSurface>();
    {
        let (mut surfaces, mut wl) = ctx.fetch::<(ResMut<Surfaces>, ResMut<Wayland>)>();
        send_zone(zone, visible, surface, &mut surfaces, &mut wl);
    }
    let radius = ctx.me().radius;
    for k in ctx.me().zone_keys.clone() {
        repaint_function_key(ctx, k, on, radius);
    }
    tracing::info!(on, "keyboard exclusive zone");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_overlaps_and_on_reserves_the_height() {
        let on = ExclusiveZone::default();
        let off = ExclusiveZone { on: false, ..on };
        assert_eq!(off.zone(252.0), -1);
        assert_eq!(on.zone(252.0), 252);
    }

    #[test]
    fn starts_reserving() {
        assert_eq!(ExclusiveZone::default().zone(252.0), 252);
    }

    #[test]
    fn suspended_reserves_nothing() {
        let zone = ExclusiveZone {
            suspended: true,
            ..Default::default()
        };
        assert_eq!(zone.zone(252.0), -1);
    }
}
