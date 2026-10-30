//! Installed apps (freedesktop desktop entries and their icons) and the
//! registry of running apps.

mod entries;
mod icons;
mod registry;

pub use entries::{DesktopApp, data_dirs, exec_argv, scan};
pub use icons::{IconCache, IconDirs, IconFile, RowIcon, active_theme};
pub use registry::{
    AppKey, Registry, RegistryChanged, RegistryModule, RunningApp, group, match_entry,
};
