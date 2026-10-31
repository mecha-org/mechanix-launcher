//! Wayland-facing half of nav: spawns the windows and container nodes and
//! pushes the stack's state onto them.

mod apply;
mod input;
mod module;
mod tick;

pub use apply::ContainerShown;
pub use module::{NavModule, WindowSpec, nav_window};
