//! Running apps: one card per running app, its icon, name and ✕ over a
//! live preview the compositor draws into a hole in our layer surface.
//!
//! Cards scroll vertically; a card swiped sideways (either way) past
//! `card::SWIPE_AWAY` closes its app, as does its ✕. Tapping a card
//! brings the app to the front and signals [`screen::AppLaunched`].

mod card;
mod thumbs;
mod view;

use std::rc::Rc;

use apps::IconDirs;
use mecha_wayland::prelude::*;
use screen::Screen;

use thumbs::{ThumbSink, WaylandThumbs};

/// Does something to one window: activate it, or ask it to close.
type WindowAction = Rc<dyn Fn(&mut App, ToplevelId)>;

/// The running-apps screen. Its window must be a layer surface for the
/// previews, and the app must have `Registry` for any cards to show.
pub struct RunningApps {
    icon_dirs: Rc<IconDirs>,
    activator: WindowAction,
    closer: WindowAction,
    thumbs: Rc<dyn ThumbSink>,
}

impl RunningApps {
    /// Icons from the XDG data dirs and the active GTK icon theme; windows
    /// and previews through the compositor.
    pub fn new() -> RunningApps {
        RunningApps::with_icon_dirs(apps::host_sources().1)
    }

    pub(crate) fn with_icon_dirs(icon_dirs: IconDirs) -> RunningApps {
        RunningApps {
            icon_dirs: Rc::new(icon_dirs),
            activator: Rc::new(activate_toplevel),
            closer: Rc::new(close_toplevel),
            thumbs: Rc::new(WaylandThumbs::default()),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn activator(mut self, f: impl Fn(&mut App, ToplevelId) + 'static) -> Self {
        self.activator = Rc::new(f);
        self
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn closer(mut self, f: impl Fn(&mut App, ToplevelId) + 'static) -> Self {
        self.closer = Rc::new(f);
        self
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn thumbs(mut self, sink: impl ThumbSink + 'static) -> Self {
        self.thumbs = Rc::new(sink);
        self
    }
}

impl Default for RunningApps {
    fn default() -> Self {
        RunningApps::new()
    }
}

impl Screen for RunningApps {
    fn build(&self, app: &mut App, parent: NodeId) -> NodeId {
        view::install(app, self.activator.clone(), self.closer.clone());
        let view = app
            .spawn(
                parent,
                view::ViewBuilder {
                    icon_dirs: self.icon_dirs.clone(),
                    thumbs: self.thumbs.clone(),
                },
            )
            .id();
        view::spawned(app, view);
        view
    }
}

/// Places the previews each frame. The compositor applies a placement at
/// the layer surface's next commit, so this must run before presentation
/// draws and commits: install after `ScrollModule` (so a frame's scroll is
/// final) and before `PresentationModule`.
pub struct RunningAppsModule;

impl Module for RunningAppsModule {
    fn install(self, app: &mut App) {
        app.system(view::place_thumbnails);
    }
}

/// Production activator: raise the window for the seat, if the compositor
/// offers toplevel management.
fn activate_toplevel(app: &mut App, id: ToplevelId) {
    if !(app.has_resource::<Toplevels>()
        && app.has_resource::<Wayland>()
        && app.has_resource::<WlSeat>())
    {
        return;
    }
    let (toplevels, mut wl, seat) = app.query::<(Res<Toplevels>, ResMut<Wayland>, Res<WlSeat>)>();
    toplevels.activate(&mut wl, *seat, id);
}

/// Production closer: ask the window to close.
fn close_toplevel(app: &mut App, id: ToplevelId) {
    if !(app.has_resource::<Toplevels>() && app.has_resource::<Wayland>()) {
        return;
    }
    let (toplevels, mut wl) = app.query::<(Res<Toplevels>, ResMut<Wayland>)>();
    toplevels.close(&mut wl, id);
}

#[cfg(test)]
mod tests;
