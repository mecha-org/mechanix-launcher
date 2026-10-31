//! [`KeyboardModule`] — install the on-screen keyboard into a host app.

use mecha_wayland::prelude::*;

use crate::latch::LatchedState;
use crate::layout;
use crate::layout::icons::Icons;
use crate::layout::tokens::Fonts;
use crate::protocol::input_method::InputMethodModule;
use crate::protocol::virtual_keyboard::VirtualKeyboardModule;
use crate::view::on_text_input_focused;
use crate::widget::surface::{KeyboardUI, KeyboardUIBuilder};
use crate::zone::ExclusiveZone;

/// Installs the on-screen keyboard: binds `zwp_virtual_keyboard_v1` and
/// `zwp_input_method_v2`, loads the keymap, fonts and icons, and spawns the
/// keyboard's own bottom-anchored layer surface, unmapped until shown. Unlike
/// the screens, the keyboard owns its window (it maps and unmaps the surface
/// as it shows and hides), so it is not placed by the nav; the host shows it
/// with [`show_keyboard`](crate::show_keyboard), e.g. from a nav action.
///
/// Expects the host to have installed the [`WaylandModule`] (with [`WlSeat`]
/// and [`ZwlrLayerShellV1`] bound), the `WindowModule`, `InteractivityModule`
/// and a theme, and to have inserted the [`Atlas`]. The theme is the host's:
/// the keymap's `mode` is ignored.
///
/// Add this after any other `Layer::Top` windows (such as the `NavModule`'s) so
/// the keyboard surface stacks above them.
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyboardModule {
    /// Empty space under the keys, logical px: room for a host's bottom-edge
    /// swipe strip, which should sit in a layer above the keyboard.
    pub bottom_gap: f32,
}

impl Module for KeyboardModule {
    fn install(self, app: &mut App) {
        // Virtual-keyboard-v1 and input-method-v2 modules — bind their globals
        // conditionally from `Globals` and handle their protocol events. These
        // must install after `WaylandModule` and after the seat is bound.
        app.add_module(VirtualKeyboardModule)
            .add_module(InputMethodModule);
        app.system(on_text_input_focused);
        spawn_ui(app, self.bottom_gap);
    }
}

/// The keyboard without its protocols: the keymap, latch, exclusive-zone and
/// visibility state, fonts and icons, and its surface, unmapped. What
/// [`KeyboardModule`] installs after binding them; public so a host can
/// build the keyboard headlessly, as the launcher's atlas budget test does.
#[doc(hidden)]
pub fn spawn_ui(app: &mut App, bottom_gap: f32) {
    // Load the keymap from layout.yaml (env var, local file, or bundled fallback).
    let keymap = layout::load_keymap();

    app.insert_resource(LatchedState::default());
    app.insert_resource(ExclusiveZone::default());
    // Keyboard surface visibility — starts hidden; IM2 `activate`, a
    // corner swipe and the hide key flip it through `KeyboardVisibilityExt`.
    crate::visibility::install(app);

    let fonts = Fonts::load(&mut app.resource_mut::<Atlas>());
    let icons = Icons::load(
        &mut app.resource_mut::<Atlas>(),
        keymap.keyboard.font_size as u32,
    );
    app.insert_resource(icons);

    let root = app.root();
    let ui = app
        .spawn(
            root,
            KeyboardUIBuilder {
                root,
                fonts,
                keymap,
                bottom_gap,
            },
        )
        .id();
    let surface = app
        .widgets::<KeyboardUI>()
        .find(|(id, _)| *id == ui)
        .unwrap()
        .1
        .surface;
    app.insert_resource(surface);

    // Starts hidden: the surface is created unmapped.
    if app.has_resource::<Surfaces>() {
        let (mut surfaces, mut wl) = app.query::<(ResMut<Surfaces>, ResMut<Wayland>)>();
        surfaces.set_mapped(surface.window, false, &mut wl);
    }
}
