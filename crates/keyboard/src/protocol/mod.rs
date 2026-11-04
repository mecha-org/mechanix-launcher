//! Wayland protocol clients the keyboard uses: [`input_method`] commits text
//! through `zwp_input_method_v2`, [`virtual_keyboard`] emits keysyms through
//! `zwp_virtual_keyboard_v1`, and [`fractional_scale`] learns the output's
//! exact scale through `wp_fractional_scale_v1`.

pub(crate) mod fractional_scale;
pub(crate) mod input_method;
pub(crate) mod virtual_keyboard;
