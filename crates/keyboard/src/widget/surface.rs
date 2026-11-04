//! The keyboard's layer surface: a `zwlr_layer_surface` anchored to the bottom
//! edge that hosts the keyboard tree.
//!
//! The surface is unmapped while the keyboard is hidden, so it takes no
//! space and catches no touches; visibility flips — from input-method
//! `activate`/`deactivate`, a corner swipe, the hide key or going home — map
//! and unmap it. The compositor keeps an unmapped layer's exclusive zone, so
//! a hide clears the zone first and a show puts it back.

use mecha_wayland::prelude::*;

use crate::layout::tokens::Fonts;
use crate::layout::{KeyboardStyle, Keymap, View};
use crate::view::BASE_VIEW;
use crate::visibility::{ApplyKeyboardVisibility, KeyboardVisibilityExt};
use crate::widget::keyboard::{Keyboard, KeyboardBuilder};
use crate::widget::popup::{GAP, HEIGHT_SCALE};
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
    /// The surface's width when the layout fixes it, else `None` (full output
    /// width, which an input rect covers with a huge span).
    pub(crate) width: Option<f32>,
}

impl KeyboardSurface {
    /// The part of the surface that takes input: everything below the
    /// headroom strip.
    pub(crate) fn input_rect(self) -> Rect {
        Rect::new(
            0.0,
            self.headroom,
            self.width.unwrap_or(INPUT_SPAN),
            self.height - self.headroom,
        )
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
/// [`GAP`]).
pub(crate) fn popup_headroom(style: &KeyboardStyle) -> f32 {
    (HEIGHT_SCALE * key_row(style) + GAP).ceil()
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
pub(crate) fn view_height(style: &KeyboardStyle, view: &View) -> f32 {
    let weights: Vec<f32> = view.rows.iter().map(|r| row_weight(r.height)).collect();
    let sum: f32 = weights.iter().sum();
    if (sum - 4.0).abs() < 1e-4 {
        return style.height;
    }
    2.0 * style.padding
        + sum * key_row(style)
        + style.row_gap * weights.len().saturating_sub(1) as f32
}

/// Spawns a `zwlr_layer_surface` anchored to the bottom edge of the output.
///
/// When no fixed width is configured it is stretched full-width (left + right
/// anchors); otherwise it is anchored to the bottom only at the configured
/// width, `bottom_gap` taller than the keys so the host's bottom-edge swipe
/// strip has room under them. The surface also has a transparent headroom
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
        let style = b.keymap.keyboard;
        let popup_room = popup_headroom(&style);
        // One surface for the tallest view; shorter views leave the rest of it
        // transparent above the keys.
        let heights: Vec<f32> = b
            .keymap
            .views
            .iter()
            .map(|v| view_height(&style, v))
            .collect();
        let tallest = heights.iter().copied().fold(style.height, f32::max);
        let height = popup_room + tallest + b.bottom_gap;
        let initial = b
            .keymap
            .views
            .iter()
            .position(|v| v.name == BASE_VIEW)
            .unwrap_or(0);
        let headroom =
            height - heights.get(initial).copied().unwrap_or(style.height) - b.bottom_gap;
        let (anchor, width) = match style.width {
            Some(w) => (Anchor::BOTTOM, px(w)),
            _ => (Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT, auto()),
        };
        let role = Role::Layer(LayerRole {
            layer: Layer::Top,
            anchor,
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
                        .size(width, px(height))
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
            width: style.width,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn surface(width: Option<f32>) -> KeyboardSurface {
        KeyboardSurface {
            window: App::new().root(),
            height: 352.0,
            headroom: 100.0,
            bottom_gap: 0.0,
            width,
        }
    }

    #[test]
    fn input_rect_skips_the_headroom() {
        let r = surface(Some(800.0)).input_rect();
        assert_eq!((r.origin.x, r.origin.y), (0.0, 100.0));
        assert_eq!((r.size.width, r.size.height), (800.0, 252.0));
        assert_eq!(r.bottom(), 352.0);
    }

    #[test]
    fn input_rect_spans_the_output_without_a_fixed_width() {
        assert_eq!(surface(None).input_rect().size.width, INPUT_SPAN);
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
            width: None,
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
        let h = popup_headroom(&style);
        assert!(h >= HEIGHT_SCALE * row + GAP, "{h} vs row {row}");
        assert!(h < HEIGHT_SCALE * row + GAP + 1.0);
    }
}
