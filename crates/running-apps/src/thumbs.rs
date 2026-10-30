//! Where previews go: the compositor's `phosh_private` thumbnails, or a
//! test's recorder.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use mecha_wayland::prelude::*;

/// Names one live preview to the sink that made it.
pub(crate) type ThumbKey = u64;

/// A preview's placement: top-left in window-local logical pixels, the
/// window's size multiplied by `scale`, drawn at `alpha`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Place {
    pub x: i32,
    pub y: i32,
    pub scale: f32,
    pub alpha: f32,
}

/// Makes, places and removes previews of windows on a nav window.
pub(crate) trait ThumbSink {
    /// A preview of `toplevel` on `window`'s surface; `None` when previews
    /// are unavailable. It shows nothing until its first [`set`](Self::set).
    fn create(&self, app: &mut App, window: NodeId, toplevel: ToplevelId) -> Option<ThumbKey>;
    fn set(&self, app: &mut App, thumb: ThumbKey, place: Place);
    fn destroy(&self, app: &mut App, thumb: ThumbKey);
    /// Make `window`'s surface commit this frame even if nothing on it
    /// redraws: placements only apply on a commit.
    fn commit(&self, app: &mut App, window: NodeId);
}

/// Previews through `phosh_private`, when the compositor offers it.
#[derive(Default)]
pub(crate) struct WaylandThumbs {
    live: RefCell<HashMap<ThumbKey, Thumbnail>>,
    next: Cell<ThumbKey>,
}

impl ThumbSink for WaylandThumbs {
    fn create(&self, app: &mut App, window: NodeId, toplevel: ToplevelId) -> Option<ThumbKey> {
        if !(app.has_resource::<Toplevels>()
            && app.has_resource::<Wayland>()
            && app.has_resource::<Surfaces>())
        {
            return None;
        }
        let (toplevels, mut wl, surfaces) =
            app.query::<(Res<Toplevels>, ResMut<Wayland>, Res<Surfaces>)>();
        if !toplevels.thumbnails_available() {
            return None;
        }
        let layer = surfaces.layer_surface_of(window)?;
        let thumb = toplevels.add_thumbnail(&mut wl, toplevel, layer)?;
        let key = self.next.get();
        self.next.set(key + 1);
        self.live.borrow_mut().insert(key, thumb);
        Some(key)
    }

    fn set(&self, app: &mut App, thumb: ThumbKey, p: Place) {
        if let Some(t) = self.live.borrow().get(&thumb) {
            t.set(&mut app.resource_mut::<Wayland>(), p.x, p.y, p.scale, p.alpha);
        }
    }

    fn destroy(&self, app: &mut App, thumb: ThumbKey) {
        if let Some(t) = self.live.borrow_mut().remove(&thumb) {
            t.destroy(&mut app.resource_mut::<Wayland>());
        }
    }

    fn commit(&self, app: &mut App, window: NodeId) {
        if app.has_resource::<Surfaces>() {
            app.resource_mut::<Surfaces>().request_commit(window);
        }
    }
}
