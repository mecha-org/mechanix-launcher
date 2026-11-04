//! The keyboard widget: spawns one subtree per keymap view (only the current
//! view is shown), hands every touch on it to the input layer
//! ([`touch::attach`](crate::touch::attach)), and handles
//! [`SwitchView`](crate::view::SwitchView) to toggle which view is visible.

use mecha_wayland::prelude::*;

use crate::layout::style::KeyState;
use crate::layout::tokens::Fonts;
use crate::layout::{Cell, Family, KeyAction, Keymap, Modifier, Page};
use crate::predict::TypingContext;
use crate::touch::{self, KeyEntry, TouchState, TouchTracker, letter_views};
use crate::view::{BASE_VIEW, SwitchView, switch_view, view_for};
use crate::widget::key::{Key, KeyBuilder};
use crate::widget::popup::{KeyPopup, KeyPopupBuilder};
use crate::widget::surface::{popup_headroom, row_weight, view_height};

/// One spawned view: its keymap name and the node toggled
/// `Display::Hidden`/`Flex` on switch.
pub(crate) struct ViewNode {
    pub(crate) family: Family,
    pub(crate) page: Page,
    pub(crate) name: String,
    pub(crate) node: Handle<Div>,
    /// Height the view takes when shown, padding included.
    pub(crate) height: f32,
}

/// The on-screen keyboard: every view, the index of the one currently shown,
/// and every latching modifier key (for armed-state repaint on latch toggle).
pub(crate) struct Keyboard {
    /// Every view, in keymap order.
    pub(crate) views: Vec<ViewNode>,
    /// Index into `views` of the view currently shown.
    pub(crate) current_view: usize,
    /// The focused field's layout family, set on focus; page keys stay inside it.
    pub(crate) family: Family,
    /// Every latching modifier key across all views, for armed-state repaint on
    /// latch toggle.
    pub(crate) latch_keys: Vec<(Modifier, Handle<Key>)>,
    /// Every exclusive-zone toggle key across all views, for on-state repaint.
    pub(crate) zone_keys: Vec<Handle<Key>>,
    /// Key corner radius, for repainting latch keys.
    pub(crate) radius: f32,
    /// Every key, per view, in keymap order: what the input layer resolves
    /// touches against.
    pub(crate) keys: Vec<Vec<KeyEntry>>,
    /// Per view: whether any key types a letter.
    pub(crate) letter_views: Vec<bool>,
    /// The contact being tracked across press, slide and release.
    pub(crate) tracker: TouchTracker,
    /// The input layer's other per-contact bookkeeping.
    pub(crate) touch: TouchState,
    /// The pressed key's bubble, painted above the keys.
    pub(crate) popup: Handle<KeyPopup>,
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
        // Keyboard container: a column as wide as the window, as tall as the
        // shown view (the window puts it on the bottom), surface background.
        // Its gap only separates views, and just one view is ever shown.
        let style = b.keymap.keyboard;
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .column()
            .width(percent(100.0))
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
        let mut keys = Vec::with_capacity(b.keymap.views.len());

        for (vi, view) in b.keymap.views.iter().enumerate() {
            let height = view_height(&style, view);
            let view_style = LayoutStyle::default()
                .column()
                .size(percent(100.0), px(height - 2.0 * style.padding))
                .gap(px(style.row_gap));
            let view_style = if vi == initial_view {
                view_style
            } else {
                view_style.hidden()
            };
            let view_div = s.spawn(me, div().style(view_style));
            views.push(ViewNode {
                name: view.name.clone(),
                family: view.family,
                page: view.page,
                node: view_div,
                height,
            });
            let mut view_keys = Vec::new();

            for row in &view.rows {
                // Rows share the view's height by weight.
                let row_div = s.spawn(
                    view_div,
                    div().style(
                        LayoutStyle::default()
                            .row()
                            .width(percent(100.0))
                            .flex_basis(px(0.0))
                            .grow(row_weight(row.height))
                            .gap(px(style.gap)),
                    ),
                );

                for cell in &row.cells {
                    // A spacer is blank width; it spawns no key, so it is
                    // never in `view_keys` and never hit.
                    let key = match cell {
                        Cell::Spacer(w) => {
                            s.spawn(
                                row_div,
                                div().style(LayoutStyle::default().grow(*w).flex_basis(px(0.0))),
                            );
                            continue;
                        }
                        Cell::Key(key) => key,
                    };
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

                    view_keys.push(KeyEntry::new(key_handle, key.action.clone()));
                }
            }
            keys.push(view_keys);
        }

        // Last child, so it paints above every key.
        let popup = s.spawn(
            me,
            KeyPopupBuilder {
                keyboard: me,
                font: b.fonts.id(style.font),
                font_size: style.font_size,
                radius: style.radius,
                headroom: popup_headroom(&style),
            },
        );

        touch::attach(s, me);
        // A focused field asked for this view: the word typed belonged to
        // the previous field, so drop it rather than finish (and record) it,
        // and cancel any touch whose key the switch hid.
        s.on::<SwitchView>(me, |ctx, event| {
            ctx.resource_mut::<TypingContext>().clear();
            let old = ctx.me().current_view;
            ctx.me().family = event.field.family;
            let me = ctx.me();
            if let Some(idx) = view_for(&me.views, event.field.family, Page::Letters) {
                let name = me.views[idx].name.clone();
                switch_view(ctx, &name);
            }
            touch::view_switched(ctx, old);
        });

        Keyboard {
            views,
            current_view: initial_view,
            family: Family::Text,
            latch_keys,
            zone_keys,
            radius: style.radius,
            letter_views: letter_views(&b.keymap),
            keys,
            tracker: TouchTracker::default(),
            touch: TouchState::default(),
            popup,
        }
    }
}
