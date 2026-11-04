//! The keyboard always spans the output and keeps its design aspect ratio.
//! Its geometry in `layout.yaml` (height, padding, gaps, corner radius) and
//! the pop-up gap are logical px for an output [`design_width`] wide
//! (`keyboard.width`); on an output `w` logical px wide they are multiplied by
//! `w / design_width`, so every proportion, and the width:height ratio, is
//! kept. Font sizes are not: they stay logical, like every other client's
//! text.
//!
//! The output's width is the keyboard surface's own: anchored left and right
//! with no width, the compositor configures it to the output's full logical
//! width ([`on_resized`]). Only while those anchors hold, though: hiding a
//! drawn surface unmaps it, which resets the compositor's layer state
//! (anchors included), so a configure that a commit on the hidden surface
//! draws (a zone change, say) carries some other width. Those are ignored;
//! the show re-sends the anchors and its configure brings the real width. Until the first configure the keyboard is laid out
//! at the design width (factor 1); a later or changed width (rotation, a new
//! output scale) re-lays it out ([`crate::widget::surface::relayout`]).
//!
//! [`design_width`]: KeyboardGeometry::design_width

use mecha_wayland::prelude::*;

use crate::layout::KeyboardStyle;
use crate::visibility::KeyboardVisibilityExt;
use crate::widget::popup::GAP;
use crate::widget::surface::{KeyboardSurface, relayout};

/// The design width when `layout.yaml` sets none, logical px.
pub(crate) const DEFAULT_DESIGN_WIDTH: f32 = 540.0;

/// What keyboard geometry is multiplied by on an output `output_width`
/// logical px wide, for a keyboard designed `design_width` wide: `1.0` while
/// the width is unknown (`None`) or either is not a positive number.
pub(crate) fn geometry_factor(output_width: Option<f32>, design_width: f32) -> f32 {
    match output_width {
        Some(w) if w.is_finite() && w > 0.0 && design_width.is_finite() && design_width > 0.0 => {
            w / design_width
        }
        _ => 1.0,
    }
}

/// `style` with its geometry multiplied by `factor`; width (the design
/// width), colours, font and font size unchanged.
pub(crate) fn scaled(style: &KeyboardStyle, factor: f32) -> KeyboardStyle {
    KeyboardStyle {
        radius: style.radius * factor,
        gap: style.gap * factor,
        row_gap: style.row_gap * factor,
        padding: style.padding * factor,
        height: style.height * factor,
        ..*style
    }
}

/// The keyboard's design style and the width of the output it spans.
#[derive(Debug, Clone, Copy)]
pub(crate) struct KeyboardGeometry {
    /// `layout.yaml`'s style, for an output [`Self::design_width`] wide.
    design: KeyboardStyle,
    /// The output's logical width, once the compositor configured the surface.
    pub(crate) output_width: Option<f32>,
    /// Whether the keyboard was ever shown. Once it was, a hide resets the
    /// surface's anchors at the compositor until the next show.
    pub(crate) ever_shown: bool,
}
impl Resource for KeyboardGeometry {}

impl KeyboardGeometry {
    pub(crate) fn new(design: KeyboardStyle) -> Self {
        Self {
            design,
            output_width: None,
            ever_shown: false,
        }
    }

    /// The output width `layout.yaml`'s geometry is written for.
    pub(crate) fn design_width(&self) -> f32 {
        self.design.width.unwrap_or(DEFAULT_DESIGN_WIDTH)
    }

    pub(crate) fn factor(&self) -> f32 {
        geometry_factor(self.output_width, self.design_width())
    }

    /// The keyboard style at the current output width.
    pub(crate) fn style(&self) -> KeyboardStyle {
        scaled(&self.design, self.factor())
    }

    /// Whether the compositor holds the surface's bottom|left|right anchors
    /// now: shown, or never shown (created anchored, not yet reset by a hide).
    /// Only then does a configure carry the output's width, and only then may
    /// the surface ask for width 0.
    pub(crate) fn anchored(&self, visible: bool) -> bool {
        visible || !self.ever_shown
    }

    /// Gap between a pop-up (or choices row) and its key at the current
    /// output width.
    pub(crate) fn popup_gap(&self) -> f32 {
        GAP * self.factor()
    }
}

/// The keyboard's surface was configured: its width is the output's. Re-lay
/// the keyboard out if that changes the factor.
pub(crate) fn on_resized(app: &mut App, e: &Emitted<Resized>) {
    let window = app.resource::<KeyboardSurface>().window;
    if !e.targets.contains(&window) {
        return;
    }
    let visible = app.keyboard_visible();
    if !app.resource::<KeyboardGeometry>().anchored(visible) {
        tracing::debug!(
            width = e.event.size.width,
            "keyboard: ignoring a configure of the hidden, unanchored surface"
        );
        return;
    }
    let before = app.resource::<KeyboardGeometry>().factor();
    app.resource_mut::<KeyboardGeometry>().output_width = Some(e.event.size.width);
    let geometry = *app.resource::<KeyboardGeometry>();
    if geometry.factor() != before {
        tracing::info!(
            output_width = e.event.size.width,
            factor = geometry.factor(),
            height = geometry.style().height,
            "keyboard: scaled to the output width"
        );
        relayout(app);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::load_keymap;

    fn at_width(w: f32) -> KeyboardGeometry {
        let mut g = KeyboardGeometry::new(load_keymap().keyboard);
        g.output_width = Some(w);
        g
    }

    #[test]
    fn design_width_and_unknown_keep_the_geometry() {
        assert_eq!(geometry_factor(Some(540.0), 540.0), 1.0);
        assert_eq!(geometry_factor(None, 540.0), 1.0);
        assert_eq!(geometry_factor(Some(0.0), 540.0), 1.0);
        assert_eq!(geometry_factor(Some(f32::NAN), 540.0), 1.0);
        assert_eq!(geometry_factor(Some(540.0), 0.0), 1.0);
        let g = KeyboardGeometry::new(load_keymap().keyboard);
        assert_eq!(g.design_width(), 540.0);
        assert_eq!(g.factor(), 1.0);
        assert_eq!(g.style().height, 252.0);
    }

    #[test]
    fn comet_at_scale_2_is_the_design() {
        // 1080 px panel at scale 2.
        let g = at_width(1080.0 / 2.0);
        assert_eq!(g.factor(), 1.0);
        assert_eq!(g.style().height, 252.0);
        assert_eq!(g.popup_gap(), GAP);
    }

    #[test]
    fn comet_at_scale_3_2_shrinks_the_logical_geometry() {
        // 1080 px at scale 3.2: 337.5 logical px.
        let g = at_width(1080.0 / 3.2);
        assert_eq!(g.factor(), 0.625);
        assert_eq!(g.popup_gap(), 2.5);
        let s = g.style();
        assert_eq!(s.height, 157.5);
        assert_eq!((s.padding, s.gap, s.row_gap), (5.0, 5.0, 7.5));
    }

    #[test]
    fn nothing_4a_keeps_the_aspect_ratio() {
        // 1224 px at scale 3: 408 logical px.
        let g = at_width(1224.0 / 3.0);
        assert_eq!(g.factor(), 408.0 / 540.0);
        let h = g.style().height;
        assert!((h - 252.0 * 408.0 / 540.0).abs() < 1e-3, "{h}");
        assert!((h - 190.4).abs() < 0.05, "{h}");
        assert!((408.0 / h - 540.0 / 252.0).abs() < 1e-4);
    }

    #[test]
    fn scaled_keeps_width_fonts_and_colours() {
        let design = load_keymap().keyboard;
        let s = scaled(&design, 0.625);
        assert_eq!(s.width, design.width);
        assert_eq!(s.radius, design.radius * 0.625);
        assert_eq!(s.font_size, design.font_size);
        assert_eq!(s.font, design.font);
        assert_eq!(s.background_color, design.background_color);
    }

    /// An app with the keyboard's geometry and surface resources (its window
    /// the root, no keyboard tree), shown or hidden, and another node.
    fn keyboard_app(visible: bool, ever_shown: bool) -> (App, NodeId, NodeId) {
        let mut app = App::new();
        app.add_module(LayoutModule).add_module(PaintModule);
        let window = app.root();
        let other = app.spawn(window, div()).id();
        let mut g = KeyboardGeometry::new(load_keymap().keyboard);
        g.ever_shown = ever_shown;
        app.insert_resource(g);
        app.insert_resource(KeyboardSurface {
            window,
            height: 391.0,
            headroom: 69.0,
            bottom_gap: 20.0,
        });
        crate::visibility::install(&mut app);
        app.resource_mut::<crate::visibility::KeyboardVisibility>()
            .visible = visible;
        (app, window, other)
    }

    fn resized(app: &mut App, target: NodeId, width: f32) {
        let e = Emitted {
            event: Resized {
                size: Size::new(width, 100.0),
            },
            targets: target.into(),
        };
        on_resized(app, &e);
    }

    #[test]
    fn the_keyboards_configure_sets_the_width() {
        let (mut app, window, _) = keyboard_app(false, false);
        resized(&mut app, window, 408.0);
        assert_eq!(app.resource::<KeyboardGeometry>().output_width, Some(408.0));
        let (mut app, window, _) = keyboard_app(true, true);
        resized(&mut app, window, 337.5);
        assert_eq!(app.resource::<KeyboardGeometry>().factor(), 0.625);
    }

    #[test]
    fn another_windows_resize_does_not_change_the_factor() {
        let (mut app, window, other) = keyboard_app(true, true);
        resized(&mut app, window, 408.0);
        resized(&mut app, other, 204.0);
        let g = *app.resource::<KeyboardGeometry>();
        assert_eq!(g.output_width, Some(408.0));
        assert_eq!(g.factor(), 408.0 / 540.0);
    }

    #[test]
    fn a_configure_while_hidden_after_a_show_is_ignored() {
        // A hide reset the anchors: the compositor's width is not the output's.
        let (mut app, window, _) = keyboard_app(false, true);
        app.resource_mut::<KeyboardGeometry>().output_width = Some(408.0);
        resized(&mut app, window, 204.0);
        assert_eq!(app.resource::<KeyboardGeometry>().output_width, Some(408.0));
        let g = *app.resource::<KeyboardGeometry>();
        assert!(!g.anchored(false));
        assert!(g.anchored(true));
    }
}
