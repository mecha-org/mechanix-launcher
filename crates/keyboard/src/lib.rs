//! An on-screen keyboard built with themed widgets on the mecha-wayland UI
//! core. Install it into a host app with [`KeyboardModule`].
//!
//! The keymap (parsed from `resources/layout.yaml`) drives which keys appear and
//! what they do. View switching (Shift `locking`, `set_view`) and one-shot
//! modifier latching (Ctrl, Alt) are supported. Text keys commit through
//! `zwp_input_method_v2` when a text input is focused, falling back to
//! `zwp_virtual_keyboard_v1` keysym transport.

#![recursion_limit = "1024"]

mod feedbackd;
mod input;
mod latch;
mod layout;
mod lent;
mod module;
mod predict;
mod protocol;
mod touch;
mod view;
mod visibility;
mod widget;
mod zone;

pub use lent::keys;
pub use module::{KeyboardModule, spawn_ui};
pub use visibility::{
    KeyboardVisibilityChanged, hide_keyboard, is_keyboard_visible, show_keyboard,
};
pub use zone::suspend_keyboard_zone;
