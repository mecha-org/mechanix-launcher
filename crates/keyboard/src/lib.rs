//! An on-screen keyboard built with themed widgets on the mecha-wayland UI
//! core. Install it into a host app with [`KeyboardModule`].
//!
//! The keymap (parsed from `resources/layout.yaml`) drives which keys appear and
//! what they do: page keys switch between the focused field family's views,
//! Shift is state (tap, double tap or hold for caps lock), and Ctrl and Alt
//! latch for one key. Text keys commit through
//! `zwp_input_method_v2` when a text input is focused, falling back to
//! `zwp_virtual_keyboard_v1` keysym transport.

#![recursion_limit = "1024"]

mod feedbackd;
mod field;
mod input;
mod layout;
mod lent;
mod modifiers;
mod module;
mod predict;
mod protocol;
mod scale;
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
