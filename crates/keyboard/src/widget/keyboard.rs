//! The keyboard widget: spawns one subtree per keymap view (only the current
//! view is shown), wires each key's `Clicked` to [`dispatch`](crate::input::dispatch)
//! (or, for repeating control keys, `Press`/`Release` to
//! [`hold`](crate::input::hold)/[`release`](crate::input::release)), and handles [`SwitchView`](crate::view::SwitchView) to toggle which view is
//! visible.

use mecha_wayland::prelude::*;

use crate::input::{dispatch, hold, release, repeats};
use crate::layout::style::KeyState;
use crate::layout::tokens::Fonts;
use crate::layout::{KeyAction, Keymap, Modifier};
use crate::view::{BASE_VIEW, SwitchView, switch_view};
use crate::widget::key::{Key, KeyBuilder};

/// One spawned view: its keymap name and the node toggled
/// `Display::Hidden`/`Flex` on switch.
pub(crate) struct ViewNode {
    pub(crate) name: String,
    pub(crate) node: Handle<Div>,
}

/// The on-screen keyboard: every view, the index of the one currently shown,
/// and every latching modifier key (for armed-state repaint on latch toggle).
pub(crate) struct Keyboard {
    /// Every view, in keymap order.
    pub(crate) views: Vec<ViewNode>,
    /// Index into `views` of the view currently shown.
    pub(crate) current_view: usize,
    /// Every latching modifier key across all views, for armed-state repaint on
    /// latch toggle.
    pub(crate) latch_keys: Vec<(Modifier, Handle<Key>)>,
    /// Every exclusive-zone toggle key across all views, for on-state repaint.
    pub(crate) zone_keys: Vec<Handle<Key>>,
    /// Key corner radius, for repainting latch keys.
    pub(crate) radius: f32,
}

impl Keyboard {
    /// The index of the view with this name, for storing as the current view.
    /// Resolved once per switch so later reads stay O(1) array indexing.
    pub(crate) fn index_of(&self, name: &str) -> Option<usize> {
        self.views.iter().position(|v| v.name == name)
    }

    pub(crate) fn current_view_name(&self) -> &str {
        &self.views[self.current_view].name
    }
}

pub(crate) struct KeyboardBuilder {
    pub(crate) fonts: Fonts,
    pub(crate) keymap: Keymap,
}

impl Build for KeyboardBuilder {
    type Widget = Keyboard;
}

impl Widget for Keyboard {
    type Builder = KeyboardBuilder;
    fn build(b: KeyboardBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        // Keyboard container: column filling the window, surface background.
        // Its gap only separates views, and just one view is ever shown.
        let style = b.keymap.keyboard;
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .column()
            .fill()
            .gap(px(style.row_gap))
            .padding_all(px(style.padding));
        let bg_role = b.keymap.keyboard.background_color;
        *s.component_mut::<Paint>(me).unwrap() = Paint::Quad(Quad::new(s.color(bg_role)));

        s.on_theme(me, move |ctx| {
            let bg = ctx.color(bg_role);
            ctx.set_paint(Paint::Quad(Quad::new(bg)));
        });

        let initial_view = b
            .keymap
            .views
            .iter()
            .position(|v| v.name == BASE_VIEW)
            .unwrap_or(0);

        // ── spawn all views ─────────────────────────────────────────────
        let mut views = Vec::with_capacity(b.keymap.views.len());
        let mut latch_keys = Vec::new();
        let mut zone_keys = Vec::new();

        for (vi, view) in b.keymap.views.iter().enumerate() {
            let view_style = LayoutStyle::default()
                .column()
                .fill()
                .gap(px(style.row_gap));
            let view_style = if vi == initial_view {
                view_style
            } else {
                view_style.hidden()
            };
            let view_div = s.spawn(me, div().style(view_style));
            views.push(ViewNode {
                name: view.name.clone(),
                node: view_div,
            });

            for row in &view.rows {
                let row_div = s.spawn(
                    view_div,
                    div().style(LayoutStyle::default().row().fill().gap(px(style.gap))),
                );

                for key in &row.keys {
                    // The label keeps its normal font while latched.
                    let label_style = key.look.style(KeyState::Normal);
                    let key_handle = s.spawn(
                        row_div,
                        KeyBuilder {
                            font: b.fonts.id(label_style.font.unwrap_or(style.font)),
                            font_size: label_style.font_size.unwrap_or(style.font_size),
                            key: key.clone(),
                            radius: style.radius,
                        },
                    );

                    match key.action {
                        KeyAction::LatchModifier(m) => latch_keys.push((m, key_handle)),
                        KeyAction::ToggleExclusiveZone => zone_keys.push(key_handle),
                        _ => {}
                    }

                    // Control keys stay down while pressed so the client
                    // repeats them; `Exit` also releases, covering a cancelled
                    // touch (which never sends `Release`) and sliding off.
                    if let KeyAction::EmitKeysym(ks) = key.action
                        && repeats(&key.action)
                    {
                        s.on::<Press>(key_handle, move |ctx, _| hold(ctx, ks));
                        s.on::<Release>(key_handle, |ctx, _| release(ctx));
                        s.on::<Exit>(key_handle, |ctx, _| release(ctx));
                        continue;
                    }

                    // Each other key gets its own handler that dispatches its
                    // action through the Keyboard's action logic.
                    let action = key.action.clone();
                    s.on::<Clicked>(key_handle, move |ctx, _| {
                        dispatch(ctx, &action);
                    });
                }
            }
        }

        s.on::<SwitchView>(me, |ctx, event| {
            switch_view(ctx, event.view);
        });

        Keyboard {
            views,
            current_view: initial_view,
            latch_keys,
            zone_keys,
            radius: style.radius,
        }
    }
}
