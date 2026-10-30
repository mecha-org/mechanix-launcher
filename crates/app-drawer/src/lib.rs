//! The app drawer: every installed app in one searchable, scrolling list
//! that launches on tap.

mod drawer;
mod launch;
mod rank;
mod row;
mod search_bar;

use std::cell::Cell;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use mecha_wayland::prelude::*;
use screen::Screen;

use drawer::DrawerBuilder;
use apps::{IconDirs, active_theme, data_dirs};

/// The drawer's two faces, Regular and Light Geist Mono, for the host's
/// `FontBook`.
pub use assets::fonts::{GEIST_MONO, GEIST_MONO_LIGHT};

/// Runs an app's argv.
type Launcher = Rc<dyn Fn(&[String]) -> io::Result<()>>;

/// The drawer screen. Its window must have the drawer's `Atlas` and
/// `FontBook` (Regular and Light Geist Mono) in place before it is built.
pub struct AppDrawer {
    data_dirs: Vec<PathBuf>,
    icon_dirs: Rc<IconDirs>,
    launcher: Launcher,
    at_top: Rc<Cell<bool>>,
}

impl AppDrawer {
    /// Apps and icons from the XDG data dirs and the active GTK icon theme;
    /// launches by spawning the entry's command.
    pub fn new() -> AppDrawer {
        let var = |k: &str| std::env::var(k).ok();
        let data_dirs = data_dirs(var);
        let config = var("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| var("HOME").map(|h| Path::new(&h).join(".config")));
        let theme = config.as_deref().and_then(active_theme);
        let icon_dirs = IconDirs::new(&data_dirs, theme.as_deref());
        AppDrawer::with_sources(data_dirs, icon_dirs)
    }

    /// Apps and icons from `data_dirs` only, with no active icon theme
    /// (hicolor and pixmaps); launches by spawning. For tests and fixtures
    /// that must not depend on the host's installed apps or theme.
    pub fn from_data_dirs(data_dirs: Vec<PathBuf>) -> AppDrawer {
        let icon_dirs = IconDirs::new(&data_dirs, None);
        AppDrawer::with_sources(data_dirs, icon_dirs)
    }

    pub(crate) fn with_sources(data_dirs: Vec<PathBuf>, icon_dirs: IconDirs) -> AppDrawer {
        AppDrawer {
            data_dirs,
            icon_dirs: Rc::new(icon_dirs),
            launcher: Rc::new(launch::spawn),
            at_top: Rc::new(Cell::new(true)),
        }
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn launcher(
        mut self,
        f: impl Fn(&[String]) -> io::Result<()> + 'static,
    ) -> AppDrawer {
        self.launcher = Rc::new(f);
        self
    }

    /// Whether the list rests at its top: the gate for the nav's close swipe.
    pub fn at_top(&self) -> Rc<Cell<bool>> {
        self.at_top.clone()
    }
}

impl Default for AppDrawer {
    fn default() -> Self {
        AppDrawer::new()
    }
}

impl Screen for AppDrawer {
    fn build(&self, app: &mut App, parent: NodeId) -> NodeId {
        drawer::install(app);
        app.spawn(
            parent,
            DrawerBuilder {
                data_dirs: self.data_dirs.clone(),
                icon_dirs: self.icon_dirs.clone(),
                launcher: self.launcher.clone(),
                at_top: self.at_top.clone(),
            },
        )
        .id()
    }
}
