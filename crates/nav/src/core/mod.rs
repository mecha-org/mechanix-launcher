//! Pure navigation core: config, gesture samples, triggers, motions,
//! containers and the stack that drives them.
//! No Wayland, windows or nodes; only `Point` and `Size` from mecha-wayland.

pub mod config;
pub mod container;
pub mod gesture;
pub mod input;
pub mod motion;
pub mod stack;
pub mod trigger;

pub use config::NavConfig;
pub use container::{ContainerId, ContainerSpec, container};
pub use gesture::{Detect, Gesture, Phase};
pub use input::Input;
pub use motion::{FADE, Fade, Final, INSTANT, Motion, Snap, Visual};
pub use stack::{ContainerState, ScreenRow, Stack};
pub use trigger::{
    Direction, ExternalEvent, Gated, OnExternal, Span, Swipe, Timing, Trigger, gated, swipe,
};

/// Names a touch area (an edge strip) that gestures start in.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct AreaId(pub &'static str);

pub const BOTTOM_EDGE: AreaId = AreaId("bottom-edge");
pub const TOP_EDGE: AreaId = AreaId("top-edge");
/// The area of triggers that fire on an [`ExternalEvent`]; no window uses it.
pub const EXTERNAL: AreaId = AreaId("external");
