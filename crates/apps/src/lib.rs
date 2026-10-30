//! Installed apps (freedesktop desktop entries and their icons) and the
//! registry of running apps.

mod entries;
mod icons;
mod registry;

use std::path::{Path, PathBuf};

pub use entries::{DesktopApp, data_dirs, exec_argv, scan};
pub use icons::{IconCache, IconDirs, IconFile, RowIcon, active_theme};
pub use registry::{
    AppKey, Registry, RegistryChanged, RegistryModule, RunningApp, group, match_entry,
};

/// The XDG data dirs from the environment, and the icon dirs for them with
/// the active GTK icon theme: where the launcher finds apps and icons.
pub fn host_sources() -> (Vec<PathBuf>, IconDirs) {
    let var = |k: &str| std::env::var(k).ok();
    let data_dirs = data_dirs(var);
    let config = var("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|h| Path::new(&h).join(".config")));
    let theme = config.as_deref().and_then(active_theme);
    let icon_dirs = IconDirs::new(&data_dirs, theme.as_deref());
    (data_dirs, icon_dirs)
}
