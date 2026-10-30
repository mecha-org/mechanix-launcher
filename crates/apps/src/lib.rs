//! Installed apps (freedesktop desktop entries and their icons) and the
//! registry of running apps.

mod entries;
mod icons;

pub use entries::{DesktopApp, data_dirs, exec_argv, scan};
pub use icons::{IconCache, IconDirs, IconFile, RowIcon, active_theme};
