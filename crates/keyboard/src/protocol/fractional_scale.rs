//! `wp_fractional_scale_v1`, by hand: the toolkit does not generate it. The
//! keyboard asks for its own surface's preferred scale so it can keep one
//! physical size at a fractional output scale, which the integer
//! `preferred_buffer_scale` (rounded up) cannot tell it.
//!
//! [`ensure_fractional_scale`] creates the object once the keyboard's surface
//! exists; each `preferred_scale` the compositor sends (on creation and on
//! every change) becomes a [`PreferredScale`] signal, which
//! [`on_preferred_scale`] stores and re-lays the keyboard out for.

use std::collections::VecDeque;
use std::os::fd::OwnedFd;

use mecha_wayland::prelude::*;
use mw_wayland::Info;
use mw_wayland::wire::Reader;

use crate::scale::KeyboardGeometry;
use crate::widget::surface::{KeyboardSurface, relayout};

/// `wp_fractional_scale_manager_v1`: hands out a fractional-scale object per
/// surface. Sends no events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WpFractionalScaleManagerV1(ObjectId);

impl Interface for WpFractionalScaleManagerV1 {
    const NAME: &'static str = "wp_fractional_scale_manager_v1";
    const VERSION: u32 = 1;
    const INFO: &'static Info = &Info {
        name: Self::NAME,
        version: 1,
        dispatch: no_events,
    };
    fn id(self) -> ObjectId {
        self.0
    }
    fn from_id(id: ObjectId) -> Self {
        Self(id)
    }
}

impl WpFractionalScaleManagerV1 {
    /// `get_fractional_scale` (opcode 1) for `surface`.
    fn get_fractional_scale(self, wl: &mut Wayland, surface: WlSurface) -> WpFractionalScaleV1 {
        let version = wl.version(self);
        let object: WpFractionalScaleV1 = wl.alloc(version);
        wl.request(self.0, 1, |w| {
            w.new_id(object.0);
            w.object(surface.id());
        });
        object
    }
}

/// `wp_fractional_scale_v1`: one surface's preferred scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct WpFractionalScaleV1(ObjectId);

impl Interface for WpFractionalScaleV1 {
    const NAME: &'static str = "wp_fractional_scale_v1";
    const VERSION: u32 = 1;
    const INFO: &'static Info = &Info {
        name: Self::NAME,
        version: 1,
        dispatch: dispatch_fractional_scale,
    };
    fn id(self) -> ObjectId {
        self.0
    }
    fn from_id(id: ObjectId) -> Self {
        Self(id)
    }
}

/// The compositor's preferred scale for the keyboard's surface.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PreferredScale(pub(crate) f32);
impl Signal for PreferredScale {}

/// `preferred_scale` carries the scale in 120ths.
fn decode_preferred_scale(opcode: u16, body: &[u8]) -> Option<PreferredScale> {
    (opcode == 0)
        .then(|| Reader::new(body).uint())
        .flatten()
        .map(|n| PreferredScale(n as f32 / 120.0))
}

fn dispatch_fractional_scale(
    app: &mut App,
    _: ObjectId,
    opcode: u16,
    body: &[u8],
    _: &mut VecDeque<OwnedFd>,
) {
    if let Some(e) = decode_preferred_scale(opcode, body) {
        app.signal(e);
    }
}

fn no_events(_: &mut App, _: ObjectId, _: u16, _: &[u8], _: &mut VecDeque<OwnedFd>) {}

/// Whether the keyboard's fractional-scale object was created, or the
/// compositor has none to offer.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct FractionalScale {
    pub(crate) settled: bool,
}
impl Resource for FractionalScale {}

/// Create the keyboard surface's fractional-scale object once its surface
/// exists (the tick after the window spawns). Does nothing more after that,
/// or if the compositor does not offer the manager.
pub(crate) fn ensure_fractional_scale(app: &mut App, _: &PostTick) {
    if app.resource::<FractionalScale>().settled || !app.has_resource::<Surfaces>() {
        return;
    }
    let window = app.resource::<KeyboardSurface>().window;
    let Some(surface) = app.resource::<Surfaces>().surface_of(window) else {
        return;
    };
    let global = app
        .resource::<Globals>()
        .find(WpFractionalScaleManagerV1::NAME)
        .cloned();
    app.resource_mut::<FractionalScale>().settled = true;
    let Some(global) = global else {
        tracing::info!("no wp_fractional_scale_manager_v1; sizing from the buffer scale");
        return;
    };
    let (globals, mut wl) = app.query::<(Res<Globals>, ResMut<Wayland>)>();
    let manager: WpFractionalScaleManagerV1 = globals.bind(&global, &mut wl);
    manager.get_fractional_scale(&mut wl, surface);
}

/// A new preferred scale: re-lay the keyboard out if it changes the size.
pub(crate) fn on_preferred_scale(app: &mut App, e: &PreferredScale) {
    let before = app.resource::<KeyboardGeometry>().factor();
    app.resource_mut::<KeyboardGeometry>().fractional = Some(e.0);
    tracing::info!(scale = e.0, "keyboard: preferred fractional scale");
    if app.resource::<KeyboardGeometry>().factor() != before {
        relayout(app);
    }
}

/// The window's buffer scale changed: the fallback when there is no
/// fractional scale.
pub(crate) fn on_buffer_scale(app: &mut App, _: &Emitted<ScaleFactorChanged>) {
    let window = app.resource::<KeyboardSurface>().window;
    let Some(scale) = app.widget::<Window>(window).map(Window::scale) else {
        return;
    };
    let before = app.resource::<KeyboardGeometry>().factor();
    app.resource_mut::<KeyboardGeometry>().buffer = Some(scale);
    if app.resource::<KeyboardGeometry>().factor() != before {
        relayout(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preferred_scale_is_in_120ths() {
        let body = 384u32.to_ne_bytes();
        assert_eq!(decode_preferred_scale(0, &body), Some(PreferredScale(3.2)));
        assert_eq!(
            decode_preferred_scale(0, &240u32.to_ne_bytes()).unwrap().0,
            2.0
        );
        assert_eq!(decode_preferred_scale(1, &body), None);
        assert_eq!(decode_preferred_scale(0, &[]), None);
    }
}
