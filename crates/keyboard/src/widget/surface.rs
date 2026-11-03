//! The keyboard's layer surface: a `zwlr_layer_surface` anchored to the bottom
//! edge that hosts the keyboard tree.
//!
//! The surface is unmapped while the keyboard is hidden, so it takes no
//! space and catches no touches; visibility flips — from input-method
//! `activate`/`deactivate`, a corner swipe, the hide key or going home — map
//! and unmap it. The compositor keeps an unmapped layer's exclusive zone, so
//! a hide clears the zone first and a show puts it back.

use mecha_wayland::prelude::*;

use crate::layout::Keymap;
use crate::layout::tokens::Fonts;
use crate::visibility::{ApplyKeyboardVisibility, KeyboardVisibilityExt};
use crate::widget::keyboard::KeyboardBuilder;
use crate::zone::{ExclusiveZone, apply_visibility};

/// The keyboard's window and height, for the controls that change its
/// surface (visibility, exclusive zone). The height counts the gap under the
/// keys.
#[derive(Debug, Clone, Copy)]
pub(crate) struct KeyboardSurface {
    pub(crate) window: NodeId,
    pub(crate) height: f32,
}
impl Resource for KeyboardSurface {}

/// Spawns a `zwlr_layer_surface` anchored to the bottom edge of the output.
///
/// When no fixed width is configured it is stretched full-width (left + right
/// anchors); otherwise it is anchored to the bottom only at the configured
/// width, `bottom_gap` taller than the keys so the host's bottom-edge swipe
/// strip has room under them. The exclusive zone follows [`ExclusiveZone`]:
/// `-1` (overlap other content without reserving space) while off or
/// suspended, the whole surface height while on (the default), and `-1`
/// whenever hidden. `KeyboardInteractivity::None`
/// keeps the OSK from stealing keyboard focus from the focused app; pointer
/// taps on the layer surface are still delivered.
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
        let height = style.height + b.bottom_gap;
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
            window().layout(
                LayoutStyle::default()
                    .column()
                    .justify(Justify::Start)
                    .size(width, px(height)),
            ),
            (role,),
        );
        let window = win.id();

        s.spawn(
            win,
            KeyboardBuilder {
                fonts: b.fonts,
                keymap: b.keymap,
                bottom_gap: b.bottom_gap,
            },
        );

        // Map the surface while shown, unmap it while hidden, zone and all.
        let surface = KeyboardSurface { window, height };
        s.on::<ApplyKeyboardVisibility>(me, move |ctx, _| {
            let visible = ctx.keyboard_visible();
            let zone = *ctx.resource::<ExclusiveZone>();
            let (mut surfaces, mut wl) = ctx.fetch::<(ResMut<Surfaces>, ResMut<Wayland>)>();
            apply_visibility(zone, visible, surface, &mut surfaces, &mut wl);
        });

        KeyboardUI { surface }
    }
}
