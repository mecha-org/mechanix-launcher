//! Wayland protocol clients the keyboard uses: [`input_method`] commits text
//! through `zwp_input_method_v2` and [`virtual_keyboard`] emits keysyms through
//! `zwp_virtual_keyboard_v1`.

pub(crate) mod input_method;
pub(crate) mod virtual_keyboard;
