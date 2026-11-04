//! The keyboard's layer surface: a `zwlr_layer_surface` anchored to the bottom
//! edge that hosts the keyboard tree.
//!
//! The surface is unmapped while the keyboard is hidden, so it takes no
//! space and catches no touches; visibility flips — from input-method
//! `activate`/`deactivate`, a corner swipe, a downward flick on the keys or
//! going home — map and unmap it. The compositor keeps an unmapped layer's exclusive zone, so
//! a hide clears the zone first and a show puts it back.

use mecha_wayland::prelude::*;

use crate::layout::tokens::Fonts;
use crate::layout::{KeyboardStyle, Keymap, View};
use crate::scale::KeyboardGeometry;
use crate::view::BASE_VIEW;
use crate::visibility::{ApplyKeyboardVisibility, KeyboardVisibilityExt};
use crate::widget::key::Key;
use crate::widget::keyboard::{Keyboard, KeyboardBuilder};
use crate::widget::more_keys::MoreKeys;
#[cfg(test)]
use crate::widget::popup::GAP;
use crate::widget::popup::{HEIGHT_SCALE, KeyPopup};
use crate::zone::{ExclusiveZone, apply_visibility, send_zone};

/// The keyboard's window and size, for the controls that change its surface
/// (visibility, exclusive zone). The height counts the transparent headroom
/// strip above the keys and the gap under them. It is fixed for the tallest
/// view; a shorter shown view leaves the extra height in the headroom.
#[derive(Debug, Clone, Copy)]
pub(crate) struct KeyboardSurface {
    pub(crate) window: NodeId,
    pub(crate) height: f32,
    /// Height of the transparent, click-through strip on top: the pop-up
    /// room plus whatever the shown view is shorter than the tallest.
    pub(crate) headroom: f32,
    /// Opaque gap under the keys.
    pub(crate) bottom_gap: f32,
}

impl KeyboardSurface {
    /// The part of the surface that takes input: everything below the
    /// headroom strip, across the whole output.
    pub(crate) fn input_rect(self) -> Rect {
        Rect::new(0.0, self.headroom, INPUT_SPAN, self.height - self.headroom)
    }
}
impl Resource for KeyboardSurface {}

impl KeyboardSurface {
    /// This surface with a `view_h`-tall view shown: the headroom is whatever
    /// the view and the gap under it leave of the surface.
    pub(crate) fn showing(self, view_h: f32) -> Self {
        Self {
            headroom: self.height - view_h - self.bottom_gap,
            ..self
        }
    }
}

/// Whether the surface's input region has reached the compositor since the
/// last show. `set_input_region` does nothing while the window's entry does
/// not exist yet (a show in the spawn tick), so [`ensure_input_region`]
/// retries each tick until it takes.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct InputRegion {
    pub(crate) applied: bool,
}
impl Resource for InputRegion {}

/// Set the surface's input region (everything below the headroom strip) if a
/// show left it unset and the window now exists.
pub(crate) fn ensure_input_region(app: &mut App, _: &PostTick) {
    if !app.has_resource::<Surfaces>() || !app.has_resource::<KeyboardSurface>() {
        return;
    }
    if app.resource::<InputRegion>().applied || !app.keyboard_visible() {
        return;
    }
    let surface = *app.resource::<KeyboardSurface>();
    let (mut surfaces, mut wl) = app.query::<(ResMut<Surfaces>, ResMut<Wayland>)>();
    if surfaces.is_mapped(surface.window) {
        surfaces.set_input_region(surface.window, Some(&[surface.input_rect()]), &mut wl);
        drop((surfaces, wl));
        app.resource_mut::<InputRegion>().applied = true;
    }
}

/// The shown view changed: the surface's headroom (and so the input region
/// and the exclusive zone) now follow the new view's height. While the
/// keyboard is shown both are re-sent at once.
pub(crate) fn view_shown(ctx: &mut Context<'_, Keyboard>) {
    let me = ctx.me();
    let height = me.views[me.current_view].height;
    let surface = ctx.resource::<KeyboardSurface>().showing(height);
    *ctx.resource_mut::<KeyboardSurface>() = surface;
    if !ctx.keyboard_visible() {
        return;
    }
    let zone = *ctx.resource::<ExclusiveZone>();
    let (mut surfaces, mut wl) = ctx.fetch::<(ResMut<Surfaces>, ResMut<Wayland>)>();
    send_zone(zone, true, surface, &mut surfaces, &mut wl);
    if surfaces.is_mapped(surface.window) {
        surfaces.set_input_region(surface.window, Some(&[surface.input_rect()]), &mut wl);
    }
}

/// Width of the input region when the surface spans the whole output: wider
/// than any output, clipped to the surface by the compositor.
const INPUT_SPAN: f32 = 16384.0;

/// Height of the transparent strip above the keys: room for a pop-up over the
/// top row ([`HEIGHT_SCALE`] key heights, as `popup.rs` draws it, plus its
/// `gap`, [`GAP`](crate::widget::popup::GAP) at the current scale).
pub(crate) fn popup_headroom(style: &KeyboardStyle, gap: f32) -> f32 {
    (HEIGHT_SCALE * key_row(style) + gap).ceil()
}

/// A row weight that makes sense: finite and positive, else `1.0` (warned).
pub(crate) fn row_weight(height: f32) -> f32 {
    if height.is_finite() && height > 0.0 {
        height
    } else {
        tracing::warn!(height, "invalid row height; using 1.0");
        1.0
    }
}

/// Height of one key row (weight `1.0`) at the style's base height.
fn key_row(style: &KeyboardStyle) -> f32 {
    (style.height - 2.0 * style.padding - 3.0 * style.row_gap) / 4.0
}

/// The height a view takes: `style.height` when its row weights sum to 4,
/// else padding, the weighted key rows and the gaps between rows.
#[cfg(test)]
pub(crate) fn view_height(style: &KeyboardStyle, view: &View) -> f32 {
    view_height_of(style, &row_weights(view))
}

/// Each row's weight in `view`, as [`row_weight`] makes sense of it.
pub(crate) fn row_weights(view: &View) -> Vec<f32> {
    view.rows.iter().map(|r| row_weight(r.height)).collect()
}

/// [`view_height`] from the view's row weights.
pub(crate) fn view_height_of(style: &KeyboardStyle, weights: &[f32]) -> f32 {
    let sum: f32 = weights.iter().sum();
    if (sum - 4.0).abs() < 1e-4 {
        return style.height;
    }
    2.0 * style.padding
        + sum * key_row(style)
        + style.row_gap * weights.len().saturating_sub(1) as f32
}

/// The surface's height for a keyboard `style` (at the current output
/// width) whose views are `heights` tall: room for the pop-up on top, the
/// tallest view and the `bottom_gap`. The compositor sizes a layer surface in
/// whole logical px, so the height is rounded up; its fraction joins the
/// transparent strip above the keys. The width is always the output's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Metrics {
    /// The pop-up's room above the tallest view.
    pub(crate) popup_room: f32,
    pub(crate) height: f32,
}

pub(crate) fn metrics(
    style: &KeyboardStyle,
    popup_gap: f32,
    heights: &[f32],
    bottom_gap: f32,
) -> Metrics {
    let popup_room = popup_headroom(style, popup_gap);
    let tallest = heights.iter().copied().fold(style.height, f32::max);
    Metrics {
        popup_room,
        height: (popup_room + tallest + bottom_gap).ceil(),
    }
}

/// Spawns a `zwlr_layer_surface` anchored to the bottom edge of the output,
/// stretched full-width (left + right anchors, no width), `bottom_gap` taller
/// than the keys so the host's bottom-edge swipe strip has room under them. The surface also has a transparent headroom
/// strip above the keys for pop-ups; the gap under the keys is opaque black.
/// The exclusive zone follows [`ExclusiveZone`]: `-1` (overlap other content
/// without reserving space) while off or suspended, the keyboard part of the
/// surface (its height minus the headroom strip, for the shown view) while on (the default), and
/// `-1` whenever hidden. `KeyboardInteractivity::None` keeps the OSK from
/// stealing keyboard focus from the focused app; pointer taps on the layer
/// surface are still delivered.
pub(crate) struct KeyboardUI {
    pub(crate) surface: KeyboardSurface,
}

pub(crate) struct KeyboardUIBuilder {
    pub(crate) root: NodeId,
    pub(crate) fonts: Fonts,
    pub(crate) keymap: Keymap,
    pub(crate) bottom_gap: f32,
}

impl Build for KeyboardUIBuilder {
    type Widget = KeyboardUI;
}

impl Widget for KeyboardUI {
    type Builder = KeyboardUIBuilder;
    fn build(b: KeyboardUIBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        let geometry = *s.resource::<KeyboardGeometry>();
        let style = geometry.style();
        // One surface for the tallest view; shorter views leave the rest of it
        // transparent above the keys.
        let heights: Vec<f32> = b
            .keymap
            .views
            .iter()
            .map(|v| view_height_of(&style, &row_weights(v)))
            .collect();
        let m = metrics(&style, geometry.popup_gap(), &heights, b.bottom_gap);
        let (popup_room, height) = (m.popup_room, m.height);
        let initial = b
            .keymap
            .views
            .iter()
            .position(|v| v.name == BASE_VIEW)
            .unwrap_or(0);
        let headroom =
            height - heights.get(initial).copied().unwrap_or(style.height) - b.bottom_gap;
        let role = Role::Layer(LayerRole {
            layer: Layer::Top,
            anchor: Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            // No zone yet: the keyboard starts hidden, and its first commit
            // reaches the compositor even so. A show sets it.
            exclusive_zone: -1,
            namespace: "keyboard".into(),
            keyboard_interactivity: KeyboardInteractivity::None,
        });

        let win = s.spawn_with(
            b.root,
            window()
                .layout(
                    LayoutStyle::default()
                        .column()
                        // The keys sit on the bottom; a shorter view leaves
                        // its slack above them.
                        .justify(Justify::End)
                        .size(auto(), px(height))
                        .padding(Insets::new(
                            px(popup_room),
                            px(0.0),
                            px(b.bottom_gap),
                            px(0.0),
                        )),
                )
                // Transparent, so only the keys' container shows; the strip
                // above it is see-through and click-through.
                .clear(Color::TRANSPARENT),
            (role,),
        );
        let window = win.id();

        s.spawn(
            win,
            KeyboardBuilder {
                fonts: b.fonts,
                keymap: b.keymap,
            },
        );

        // The gap under the keys is opaque: the window itself clears
        // transparent, for the headroom strip only.
        s.spawn(
            win,
            div().background(Color::BLACK).style(
                LayoutStyle::default()
                    .absolute()
                    .size(percent(100.0), px(b.bottom_gap))
                    .inset(Insets::new(auto(), px(0.0), px(0.0), px(0.0))),
            ),
        );

        // Map the surface while shown, unmap it while hidden, zone and all.
        let surface = KeyboardSurface {
            window,
            height,
            headroom,
            bottom_gap: b.bottom_gap,
        };
        s.on::<ApplyKeyboardVisibility>(me, move |ctx, _| {
            let visible = ctx.keyboard_visible();
            let zone = *ctx.resource::<ExclusiveZone>();
            // The resource follows the shown view; the copy captured here
            // would be the initial view's.
            let surface = *ctx.resource::<KeyboardSurface>();
            let applied = {
                let (mut surfaces, mut wl) = ctx.fetch::<(ResMut<Surfaces>, ResMut<Wayland>)>();
                apply_visibility(zone, visible, surface, &mut surfaces, &mut wl)
            };
            ctx.resource_mut::<InputRegion>().applied = applied;
        });

        KeyboardUI { surface }
    }
}

/// Re-lay the keyboard out for the current [`KeyboardGeometry`] after the
/// output width changed: the container's padding and gap, each view's height
/// and gaps, the key, pop-up and choices-row corners, the pop-up's room, and
/// the surface (its size, then, if shown, its exclusive zone and input
/// region). Fonts keep their size.
pub(crate) fn relayout(app: &mut App) {
    let geometry = *app.resource::<KeyboardGeometry>();
    let style = geometry.style();
    let popup_gap = geometry.popup_gap();
    let headroom = popup_headroom(&style, popup_gap);

    let Some(kb) = app.widgets::<Keyboard>().map(|(id, _)| id).next() else {
        return;
    };
    let (views, current, popup, choices_row, old_radius) = {
        let me = app.widget_mut::<Keyboard>(kb).unwrap();
        me.headroom = headroom;
        me.popup_gap = popup_gap;
        let old_radius = std::mem::replace(&mut me.radius, style.radius);
        for v in &mut me.views {
            v.height = view_height_of(&style, &v.weights);
        }
        let views: Vec<_> = me
            .views
            .iter()
            .map(|v| (v.node.id(), v.height, v.rows.clone()))
            .collect();
        (views, me.current_view, me.popup, me.choices_row, old_radius)
    };

    restyle(app, kb, |s| {
        s.gap(px(style.row_gap)).padding_all(px(style.padding))
    });
    for (node, height, rows) in &views {
        restyle(app, *node, |s| {
            s.height(px(height - 2.0 * style.padding))
                .gap(px(style.row_gap))
        });
        for row in rows {
            restyle(app, *row, |s| s.gap(px(style.gap)));
        }
    }

    if style.radius != old_radius {
        let keys: Vec<NodeId> = app
            .widgets::<Key>()
            .map(|(id, key)| {
                key.radius = style.radius;
                id
            })
            .collect();
        for id in keys {
            if let Some(mut paint) = app.component_mut::<Paint>(id)
                && let Paint::Quad(q) = &mut *paint
            {
                *q = q.radius(style.radius);
            }
        }
        if let Some(c) = app.widget_mut::<MoreKeys>(choices_row) {
            c.radius = style.radius;
        }
    }
    if let Some(p) = app.widget_mut::<KeyPopup>(popup) {
        p.radius = style.radius;
        p.headroom = headroom;
        p.gap = popup_gap;
    }

    // The surface.
    let heights: Vec<f32> = views.iter().map(|v| v.1).collect();
    let old = *app.resource::<KeyboardSurface>();
    let m = metrics(&style, popup_gap, &heights, old.bottom_gap);
    let surface = KeyboardSurface {
        height: m.height,
        ..old
    }
    .showing(heights.get(current).copied().unwrap_or(style.height));
    *app.resource_mut::<KeyboardSurface>() = surface;
    restyle(app, surface.window, |s| {
        s.height(px(m.height)).padding(Insets::new(
            px(m.popup_room),
            px(0.0),
            px(old.bottom_gap),
            px(0.0),
        ))
    });
    if !app.has_resource::<Surfaces>() {
        return;
    }
    let visible = app.keyboard_visible();
    let zone = *app.resource::<ExclusiveZone>();
    let (mut surfaces, mut wl) = app.query::<(ResMut<Surfaces>, ResMut<Wayland>)>();
    // Width 0: the compositor keeps it the output's.
    surfaces.request_layer_size(surface.window, 0, m.height as u32, &mut wl);
    if visible {
        send_zone(zone, true, surface, &mut surfaces, &mut wl);
        if surfaces.is_mapped(surface.window) {
            surfaces.set_input_region(surface.window, Some(&[surface.input_rect()]), &mut wl);
        }
    }
}

/// Replace `node`'s layout style with `f` of it.
fn restyle(app: &mut App, node: impl Into<NodeId>, f: impl FnOnce(LayoutStyle) -> LayoutStyle) {
    if let Some(mut style) = app.component_mut::<LayoutStyle>(node) {
        *style = f(style.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surface() -> KeyboardSurface {
        KeyboardSurface {
            window: App::new().root(),
            height: 352.0,
            headroom: 100.0,
            bottom_gap: 0.0,
        }
    }

    #[test]
    fn input_rect_skips_the_headroom() {
        let r = surface().input_rect();
        assert_eq!((r.origin.x, r.origin.y), (0.0, 100.0));
        assert_eq!(r.size.height, 252.0);
        assert_eq!(r.bottom(), 352.0);
    }

    #[test]
    fn input_rect_spans_the_output() {
        assert_eq!(surface().input_rect().size.width, INPUT_SPAN);
    }

    fn view_of(weights: &[f32]) -> View {
        View {
            name: "v".into(),
            family: crate::layout::Family::Text,
            page: crate::layout::Page::Letters,
            rows: weights
                .iter()
                .map(|&height| crate::layout::Row {
                    cells: vec![],
                    height,
                })
                .collect(),
        }
    }

    #[test]
    fn four_equal_rows_are_the_configured_height() {
        let style = crate::layout::load_keymap().keyboard;
        assert_eq!(view_height(&style, &view_of(&[1.0; 4])), style.height);
        assert_eq!(style.height, 252.0);
    }

    #[test]
    fn terminal_strip_adds_three_quarters_row_and_a_gap() {
        let style = crate::layout::load_keymap().keyboard;
        let h = view_height(&style, &view_of(&[0.75, 1.0, 1.0, 1.0, 1.0]));
        assert_eq!(h, 252.0 + 0.75 * 50.0 + 12.0);
    }

    #[test]
    fn invalid_row_heights_count_as_one() {
        let style = crate::layout::load_keymap().keyboard;
        let h = view_height(&style, &view_of(&[f32::NAN, 0.0, -2.0, 1.0]));
        assert_eq!(h, style.height);
    }

    #[test]
    fn a_shorter_view_leaves_its_slack_in_the_headroom() {
        let s = KeyboardSurface {
            window: App::new().root(),
            height: 100.0 + 301.5 + 20.0,
            headroom: 100.0,
            bottom_gap: 20.0,
        };
        let short = s.showing(252.0);
        assert_eq!(short.headroom, 100.0 + 49.5);
        assert_eq!(short.input_rect().size.height, 252.0 + 20.0);
        assert_eq!(s.showing(301.5).headroom, 100.0);
    }

    #[test]
    fn popup_headroom_fits_a_top_row_bubble() {
        let style = crate::layout::load_keymap().keyboard;
        let row = (style.height - 2.0 * style.padding - 3.0 * style.row_gap) / 4.0;
        let h = popup_headroom(&style, GAP);
        assert!(h >= HEIGHT_SCALE * row + GAP, "{h} vs row {row}");
        assert!(h < HEIGHT_SCALE * row + GAP + 1.0);
    }

    use crate::scale::KeyboardGeometry;
    use crate::zone::{ExclusiveZone, reserved_height};

    /// The shipped keyboard on the Comet's 1080 px panel at output `scale`.
    fn at_scale(scale: f32) -> KeyboardGeometry {
        let mut g = KeyboardGeometry::new(crate::layout::load_keymap().keyboard);
        g.output_width = Some(1080.0 / scale);
        g
    }

    const LETTERS: [f32; 4] = [1.0; 4];
    const TERMINAL: [f32; 5] = [0.75, 1.0, 1.0, 1.0, 1.0];

    #[test]
    fn design_scale_geometry_is_unchanged() {
        let g = at_scale(2.0);
        let style = g.style();
        assert_eq!(view_height_of(&style, &LETTERS), 252.0);
        assert_eq!(view_height_of(&style, &TERMINAL), 301.5);
        assert_eq!(popup_headroom(&style, g.popup_gap()), 69.0);
        let m = metrics(&style, g.popup_gap(), &[252.0, 301.5], 20.0);
        assert_eq!(
            m,
            Metrics {
                popup_room: 69.0,
                height: 391.0,
            }
        );
        let zone = ExclusiveZone::default();
        assert_eq!(
            zone.zone(reserved_height(m.height, m.height - 252.0 - 20.0)),
            272
        );
    }

    #[test]
    fn scale_3_2_keeps_the_physical_size() {
        // 337.5 logical px wide: factor 0.625.
        let g = at_scale(3.2);
        let style = g.style();
        let letters = view_height_of(&style, &LETTERS);
        let terminal = view_height_of(&style, &TERMINAL);
        // Same physical px as at scale 2: 252 * 2 and 301.5 * 2.
        assert_eq!(letters * 3.2, 504.0);
        assert!((terminal * 3.2 - 603.0).abs() < 1e-3, "{terminal}");
        // 1.3 rows of 31.25 plus a 2.5 gap, rounded up.
        assert_eq!(popup_headroom(&style, g.popup_gap()), 44.0);
        let m = metrics(&style, g.popup_gap(), &[letters, terminal], 20.0);
        assert_eq!(m.popup_room, 44.0);
        assert_eq!(m.height, (44.0f32 + terminal + 20.0).ceil());
        let s = KeyboardSurface {
            window: App::new().root(),
            height: m.height,
            headroom: 0.0,
            bottom_gap: 20.0,
        };
        let zone = ExclusiveZone::default();
        let shown = s.showing(letters);
        assert_eq!(shown.input_rect().size.height, letters + 20.0);
        assert_eq!(
            zone.zone(reserved_height(shown.height, shown.headroom)),
            177
        );
        let shown = s.showing(terminal);
        assert_eq!(
            zone.zone(reserved_height(shown.height, shown.headroom)),
            208
        );
    }

    #[test]
    fn nothing_4a_keeps_the_design_aspect_ratio() {
        // 1224 px at scale 3: 408 logical px wide.
        let mut g = KeyboardGeometry::new(crate::layout::load_keymap().keyboard);
        g.output_width = Some(408.0);
        let style = g.style();
        let letters = view_height_of(&style, &LETTERS);
        assert!((letters - 252.0 * 408.0 / 540.0).abs() < 1e-3, "{letters}");
        assert!((408.0 / letters - 540.0 / 252.0).abs() < 1e-4);
        let terminal = view_height_of(&style, &TERMINAL);
        assert!(
            (terminal - 301.5 * 408.0 / 540.0).abs() < 1e-3,
            "{terminal}"
        );
        let m = metrics(&style, g.popup_gap(), &[letters, terminal], 20.0);
        assert_eq!(m.popup_room, popup_headroom(&style, g.popup_gap()));
        assert_eq!(m.height, (m.popup_room + terminal + 20.0).ceil());
    }
}
