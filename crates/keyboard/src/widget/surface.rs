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
use crate::layout::{KeyboardStyle, Keymap};
use crate::visibility::{ApplyKeyboardVisibility, KeyboardVisibilityExt};
use crate::widget::keyboard::KeyboardBuilder;
use crate::widget::popup::{GAP, HEIGHT_SCALE};
use crate::zone::{ExclusiveZone, apply_visibility};

/// The keyboard's window and size, for the controls that change its surface
/// (visibility, exclusive zone). The height counts the transparent headroom
/// strip above the keys and the gap under them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct KeyboardSurface {
    pub(crate) window: NodeId,
    pub(crate) height: f32,
    /// Height of the transparent strip on top, where key pop-ups may rise.
    pub(crate) headroom: f32,
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

/// Width of the input region when the surface spans the whole output: wider
/// than any output, clipped to the surface by the compositor.
const INPUT_SPAN: f32 = 16384.0;

/// Height of the transparent strip above the keys: room for a pop-up over the
/// top row ([`HEIGHT_SCALE`] key heights, as `popup.rs` draws it, plus its
/// [`GAP`]).
pub(crate) fn popup_headroom(style: &KeyboardStyle) -> f32 {
    let row = (style.height - 2.0 * style.padding - 3.0 * style.row_gap) / 4.0;
    (HEIGHT_SCALE * row + GAP).ceil()
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
/// surface (its height minus the headroom strip) while on (the default), and
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
        let headroom = popup_headroom(&style);
        let height = headroom + style.height + b.bottom_gap;
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
            // The usual OSK namespace: compositors show it over fullscreen apps.
            namespace: "osk".into(),
            keyboard_interactivity: KeyboardInteractivity::None,
        });

        let win = s.spawn_with(
            b.root,
            window()
                .layout(
                    LayoutStyle::default()
                        .column()
                        .justify(Justify::Start)
                        .size(width, px(height))
                        .padding(Insets::new(
                            px(headroom),
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
            width: style.width,
        };
        s.on::<ApplyKeyboardVisibility>(me, move |ctx, _| {
            let visible = ctx.keyboard_visible();
            let zone = *ctx.resource::<ExclusiveZone>();
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

    #[test]
    fn popup_headroom_fits_a_top_row_bubble() {
        let style = crate::layout::load_keymap().keyboard;
        let row = (style.height - 2.0 * style.padding - 3.0 * style.row_gap) / 4.0;
        let h = popup_headroom(&style);
        assert!(h >= HEIGHT_SCALE * row + GAP, "{h} vs row {row}");
        assert!(h < HEIGHT_SCALE * row + GAP + 1.0);
    }
}
