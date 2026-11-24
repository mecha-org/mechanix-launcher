//! The app drawer: every installed app in one searchable, scrolling list
//! that launches on tap.

mod drawer;
mod launch;
mod placeholder;
mod rank;
mod row;
mod search_bar;

use std::path::PathBuf;
use std::rc::Rc;

use mecha_wayland::prelude::*;
use screen::Screen;

use apps::{DesktopApp, IconDirs, LaunchId};
use drawer::DrawerBuilder;

/// The drawer's two faces, Regular and Light Geist Mono, for the host's
/// `FontBook`.
pub use assets::fonts::{GEIST_MONO, GEIST_MONO_LIGHT};
pub use placeholder::hide_launch_placeholder;

/// Starts an app, for its launch.
type Launcher = Rc<dyn Fn(&mut App, &DesktopApp, LaunchId)>;

/// Brings a running app's window to the front.
type Activator = Rc<dyn Fn(&mut App, ToplevelId)>;

/// The drawer screen. Its window must have the drawer's `Atlas` and
/// `FontBook` (Regular and Light Geist Mono) in place before it is built.
pub struct AppDrawer {
    data_dirs: Vec<PathBuf>,
    icon_dirs: Rc<IconDirs>,
    launcher: Launcher,
    activator: Activator,
}

impl AppDrawer {
    /// Apps and icons from the XDG data dirs and the active GTK icon theme;
    /// launches each app as a systemd user service of its own.
    pub fn new() -> AppDrawer {
        let (data_dirs, icon_dirs) = apps::host_sources();
        AppDrawer::with_sources(data_dirs, icon_dirs)
    }

    /// Apps and icons from `data_dirs` only, with no active icon theme
    /// (hicolor and pixmaps); launches as [`AppDrawer::new`]. For tests and fixtures
    /// that must not depend on the host's installed apps or theme.
    pub fn from_data_dirs(data_dirs: Vec<PathBuf>) -> AppDrawer {
        let icon_dirs = IconDirs::new(&data_dirs, None);
        AppDrawer::with_sources(data_dirs, icon_dirs)
    }

    pub(crate) fn with_sources(data_dirs: Vec<PathBuf>, icon_dirs: IconDirs) -> AppDrawer {
        AppDrawer {
            data_dirs,
            icon_dirs: Rc::new(icon_dirs),
            launcher: Rc::new(launch::start),
            activator: Rc::new(drawer::activate_toplevel),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn launcher(
        mut self,
        f: impl Fn(&mut App, &DesktopApp, LaunchId) + 'static,
    ) -> AppDrawer {
        self.launcher = Rc::new(f);
        self
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn activator(mut self, f: impl Fn(&mut App, ToplevelId) + 'static) -> AppDrawer {
        self.activator = Rc::new(f);
        self
    }
}

impl Default for AppDrawer {
    fn default() -> Self {
        AppDrawer::new()
    }
}

impl Screen for AppDrawer {
    fn build(&self, app: &mut App, parent: NodeId) -> NodeId {
        drawer::install(app, self.activator.clone(), self.launcher.clone());
        app.spawn(
            parent,
            DrawerBuilder {
                data_dirs: self.data_dirs.clone(),
                icon_dirs: self.icon_dirs.clone(),
            },
        )
        .id()
    }
}
