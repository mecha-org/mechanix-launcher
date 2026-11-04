//! The keyboard keeps one physical size at any output scale. Its geometry in
//! `layout.yaml` (width, height, padding, gaps, corner radius) and the pop-up
//! gap are logical px at [`DESIGN_SCALE`]; at an output scale `s` they are
//! multiplied by `DESIGN_SCALE / s`, so the keys cover the same physical
//! pixels whatever the scale. Font sizes are not: they stay logical and grow
//! with the scale like every other client's text.
//!
//! The scale comes from `wp_fractional_scale_v1` (exact, e.g. 3.2) when the
//! compositor offers it, else from the window's integer buffer scale. Until
//! either is known the keyboard is laid out at the design scale, so the first
//! frame does not jump; a later or changed scale re-lays it out
//! ([`crate::widget::surface::relayout`]).

use mecha_wayland::prelude::*;

use crate::layout::KeyboardStyle;
use crate::widget::popup::GAP;

/// The output scale `layout.yaml`'s keyboard geometry is written for.
pub(crate) const DESIGN_SCALE: f32 = 2.0;

/// What keyboard geometry is multiplied by at output `scale`: `1.0` at the
/// design scale and while the scale is unknown (`None`, or not a positive
/// number).
pub(crate) fn geometry_factor(scale: Option<f32>) -> f32 {
    match scale {
        Some(s) if s.is_finite() && s > 0.0 => DESIGN_SCALE / s,
        _ => 1.0,
    }
}

/// `style` with its geometry multiplied by `factor`; colours, font and font
/// size unchanged.
pub(crate) fn scaled(style: &KeyboardStyle, factor: f32) -> KeyboardStyle {
    KeyboardStyle {
        radius: style.radius * factor,
        gap: style.gap * factor,
        row_gap: style.row_gap * factor,
        padding: style.padding * factor,
        width: style.width.map(|w| w * factor),
        height: style.height * factor,
        ..*style
    }
}

/// The keyboard's design style and the output scale it is shown at.
#[derive(Debug, Clone, Copy)]
pub(crate) struct KeyboardGeometry {
    /// `layout.yaml`'s style, at [`DESIGN_SCALE`].
    design: KeyboardStyle,
    /// From `wp_fractional_scale_v1`, once it said.
    pub(crate) fractional: Option<f32>,
    /// The window's integer buffer scale, once the WSI reported one.
    pub(crate) buffer: Option<f32>,
}
impl Resource for KeyboardGeometry {}

impl KeyboardGeometry {
    pub(crate) fn new(design: KeyboardStyle) -> Self {
        Self {
            design,
            fractional: None,
            buffer: None,
        }
    }

    /// The best scale known: the exact fractional one, else the buffer one.
    pub(crate) fn scale(&self) -> Option<f32> {
        self.fractional.or(self.buffer)
    }

    pub(crate) fn factor(&self) -> f32 {
        geometry_factor(self.scale())
    }

    /// The keyboard style at the current scale.
    pub(crate) fn style(&self) -> KeyboardStyle {
        scaled(&self.design, self.factor())
    }

    /// Gap between a pop-up (or choices row) and its key at the current scale.
    pub(crate) fn popup_gap(&self) -> f32 {
        GAP * self.factor()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::load_keymap;

    #[test]
    fn design_scale_and_unknown_keep_the_geometry() {
        assert_eq!(geometry_factor(Some(2.0)), 1.0);
        assert_eq!(geometry_factor(None), 1.0);
        assert_eq!(geometry_factor(Some(0.0)), 1.0);
        assert_eq!(geometry_factor(Some(f32::NAN)), 1.0);
    }

    #[test]
    fn higher_scale_shrinks_the_logical_geometry() {
        assert_eq!(geometry_factor(Some(3.2)), 0.625);
        assert_eq!(geometry_factor(Some(4.0)), 0.5);
        assert_eq!(geometry_factor(Some(1.0)), 2.0);
    }

    #[test]
    fn scaled_keeps_fonts_and_colours() {
        let design = load_keymap().keyboard;
        let s = scaled(&design, 0.625);
        assert_eq!(s.height, 157.5);
        assert_eq!(s.width, Some(337.5));
        assert_eq!((s.padding, s.gap, s.row_gap), (5.0, 5.0, 7.5));
        assert_eq!(s.radius, design.radius * 0.625);
        assert_eq!(s.font_size, design.font_size);
        assert_eq!(s.font, design.font);
        assert_eq!(s.background_color, design.background_color);
    }

    #[test]
    fn same_physical_height_at_any_scale() {
        let design = load_keymap().keyboard;
        for scale in [1.0, 2.0, 2.5, 3.2, 4.0] {
            let s = scaled(&design, geometry_factor(Some(scale)));
            assert!((s.height * scale - 504.0).abs() < 1e-3, "{scale}");
        }
    }

    #[test]
    fn fractional_scale_wins_over_the_buffer_scale() {
        let mut g = KeyboardGeometry::new(load_keymap().keyboard);
        assert_eq!(g.factor(), 1.0);
        g.buffer = Some(4.0);
        assert_eq!(g.factor(), 0.5);
        g.fractional = Some(3.2);
        assert_eq!(g.factor(), 0.625);
        assert_eq!(g.popup_gap(), 2.5);
        assert_eq!(g.style().height, 157.5);
    }
}
