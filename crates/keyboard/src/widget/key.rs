//! The themed key button: a `div` with a `Paint::Quad` background resolved
//! from a [`ColorRole`] pair, a centred text or icon label, and handlers that
//! re-resolve both colours on theme flip and on the input layer's
//! [`KeyFeedback`]. Keys take no input themselves.

use mecha_wayland::prelude::*;

use crate::layout::emoji::Emoji;
use crate::layout::icons::Icons;
use crate::layout::style::{KeyLook, KeyState};
use crate::layout::{KeyAction, KeyFace};
use crate::modifiers::latch::LatchedState;
use crate::modifiers::pointing_stick::PointingStickMode;
use crate::modifiers::shift::{Shift, ShiftState};
use crate::touch::{KeyFeedback, popup_text};

/// What a key looks like — determines the [`ColorRole`] pair it draws with.
/// Derived from the key's [`KeyAction`] so the visual matches the behaviour.
#[derive(Clone, Copy)]
pub(crate) enum KeyKind {
    /// Regular character key (letters, digits, punctuation).
    Normal,
    /// Non-typing key: modifiers (Ctrl, Alt), view switches (Shift, abc, 123),
    /// and actions not wired yet (prefs, emoji).
    Function,
}

/// Classify a key's action into a visual kind for theme colour selection.
fn kind_of(action: &KeyAction) -> KeyKind {
    match action {
        KeyAction::EmitKeysym(_) | KeyAction::EmitText(_) => KeyKind::Normal,
        KeyAction::LatchModifier(_)
        | KeyAction::SetView(_)
        | KeyAction::Page(_)
        | KeyAction::Family(_)
        | KeyAction::Shift
        | KeyAction::PointingStick
        | KeyAction::Unhandled(_) => KeyKind::Function,
    }
}

/// Resolve a [`KeyKind`] to its (background, foreground) colour roles. A latched
/// modifier uses the `Primary` palette so the armed state reads at a glance.
fn roles_for(kind: KeyKind, state: KeyState) -> (ColorRole, ColorRole) {
    match state {
        KeyState::Latched => (ColorRole::Primary, ColorRole::OnPrimary),
        KeyState::Normal => match kind {
            KeyKind::Normal => (ColorRole::SurfaceContainerHigh, ColorRole::OnSurface),
            KeyKind::Function => (
                ColorRole::SecondaryContainer,
                ColorRole::OnSecondaryContainer,
            ),
        },
    }
}

/// Resolve a key's colour-role pair, letting the key's own style override the
/// kind-based defaults.
pub(crate) fn key_roles(look: KeyLook, kind: KeyKind, state: KeyState) -> (ColorRole, ColorRole) {
    let style = look.style(state);
    let (bg, fg) = roles_for(kind, state);
    (
        style.background.unwrap_or(bg),
        style.foreground.unwrap_or(fg),
    )
}

/// A key's label, whichever kind it is.
#[derive(Clone, Copy)]
pub(crate) enum Label {
    Text(Handle<Text>),
    Icon(Handle<Icon>),
    /// A colour emoji: keeps its own colours whatever the key state.
    Image,
}

/// Recolour a key's label, whichever kind it is.
pub(crate) fn set_label_color<W: Widget>(ctx: &mut Context<'_, W>, label: Label, fg: Color) {
    match label {
        Label::Text(h) => ctx.at(h).unwrap().set_color(fg),
        Label::Icon(h) => ctx.at(h).unwrap().set_color(fg),
        Label::Image => {}
    }
}

/// Build args for a [`Key`]: the resolved font, the label's font size, the
/// resolved key, and the corner radius.
pub(crate) struct KeyBuilder {
    pub(crate) font: FontId,
    pub(crate) font_size: u16,
    /// Font for the corner hint (`geist-mono`).
    pub(crate) hint_font: FontId,
    pub(crate) key: crate::layout::Key,
    pub(crate) radius: f32,
}

impl Build for KeyBuilder {
    type Widget = Key;
}

/// A themed key button. Draw-only: the input layer resolves touches to keys
/// and tells the key when it is pressed and released.
pub(crate) struct Key {
    pub(crate) label: Label,
    pub(crate) look: KeyLook,
    /// The caps-lock underline on a Shift key, shown only while locked.
    pub(crate) lock_bar: Option<Handle<Div>>,
    /// What the pop-up shows while this key is pressed; `None` for keys that
    /// don't type a visible character.
    pub(crate) popup_text: Option<String>,
    /// Corner radius at the current scale (a rescale updates it).
    pub(crate) radius: f32,
}

impl Widget for Key {
    type Builder = KeyBuilder;
    fn build(b: KeyBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        let kind = kind_of(&b.key.action);
        let look = b.key.look;
        let (bg_role, fg_role) = key_roles(look, kind, KeyState::Normal);
        let radius = b.radius;

        let mut layout = LayoutStyle::default()
            .center()
            // Fills its cell, which the keyboard sizes from the outline width
            // (see `cell_style`); a zero basis and minimum keep a wide label
            // from widening it.
            .grow(1.0)
            .flex_basis(px(0.0))
            .min_width(px(0.0));
        if b.key.height < 1.0 {
            // A short key: a fraction of the row, centred vertically.
            layout = layout
                .height(percent(b.key.height * 100.0))
                .align_self(Align::Center);
        }
        *s.component_mut::<LayoutStyle>(me).unwrap() = layout;

        *s.component_mut::<Paint>(me).unwrap() =
            Paint::Quad(Quad::new(s.color(bg_role)).radius(radius));

        let fg = s.color(fg_role);
        let label = match &b.key.face {
            KeyFace::Icon(name) => match s.resource::<Icons>().get(name) {
                Some(sprite) => Label::Icon(s.spawn(me, icon(sprite).color(fg))),
                None => {
                    tracing::warn!("no icon named {name:?} in resources/icons; drawing empty key");
                    Label::Text(s.spawn(me, text(b.font, "").color(fg).size(b.font_size)))
                }
            },
            KeyFace::Text(t) => {
                Label::Text(s.spawn(me, text(b.font, t).color(fg).size(b.font_size)))
            }
            KeyFace::Emoji(e) => match s.resource::<Emoji>().get(e) {
                Some(e) => {
                    let style = LayoutStyle::default().size(px(e.width), px(e.height));
                    s.spawn(me, image(e.sprite).style(style));
                    Label::Image
                }
                None => Label::Text(s.spawn(me, text(b.font, "").color(fg).size(b.font_size))),
            },
        };

        // Corner hint: top-right, half the label size, not on icon keys.
        let hint = match (&b.key.face, &b.key.hint) {
            (KeyFace::Text(_), Some(h)) => {
                let color = s.color(ColorRole::OnSecondaryContainer);
                let size = (f32::from(b.font_size) * 0.5).round().max(1.0) as u16;
                let handle = s.spawn(me, text(b.hint_font, h).color(color).size(size));
                *s.component_mut::<LayoutStyle>(handle).unwrap() = LayoutStyle::default()
                    .absolute()
                    .inset(Insets::new(px(0.0), px(5.0), auto(), auto()));
                Some(handle)
            }
            _ => None,
        };

        // Caps-lock marker: a 2 px bar along the bottom edge, hidden until locked.
        let lock_bar = matches!(b.key.action, KeyAction::Shift).then(|| {
            let color = s.color(ColorRole::OnPrimary);
            s.spawn(
                me,
                div().background(color).style(
                    LayoutStyle::default()
                        .absolute()
                        .inset(Insets::new(auto(), percent(30.0), px(4.0), percent(30.0)))
                        .height(px(2.0))
                        .hidden(),
                ),
            )
        });

        let latch = match b.key.action {
            KeyAction::LatchModifier(m) => Some(m),
            _ => None,
        };
        let shift = matches!(b.key.action, KeyAction::Shift);
        // Latch and shift keys show their on state, so a press doesn't
        // flash them.
        let is_latch = latch.is_some() || shift;
        // The pointing stick shows select mode in its latched colours, and
        // flashes from (and back to) whichever colours its mode has.
        let pointing_stick = matches!(b.key.action, KeyAction::PointingStick);
        let selecting = move |ctx: &Context<'_, Key>| {
            pointing_stick && ctx.resource::<PointingStickMode>().select()
        };
        s.on_theme(me, move |ctx| {
            let lit = latch.is_some_and(|m| ctx.resource::<LatchedState>().get(m))
                || (shift && ctx.resource::<Shift>().state() != ShiftState::Off)
                || selecting(ctx);
            let state = KeyState::latched_if(lit);
            let (bg_role, fg_role) = key_roles(look, kind, state);
            let bg = ctx.color(bg_role);
            let fg = ctx.color(fg_role);
            let radius = ctx.me().radius;
            ctx.set_paint(Paint::Quad(Quad::new(bg).radius(radius)));
            set_label_color(ctx, label, fg);
            if let Some(h) = lock_bar {
                let c = ctx.color(ColorRole::OnPrimary);
                ctx.at(h).unwrap().set_paint(Paint::Quad(Quad::new(c)));
            }
            if let Some(h) = hint {
                let c = ctx.color(ColorRole::OnSecondaryContainer);
                ctx.at(h).unwrap().set_color(c);
            }
        });

        if !is_latch {
            s.on::<KeyFeedback>(me, move |ctx, e| {
                let state = KeyState::latched_if(selecting(ctx));
                let (bg_role, fg_role) = key_roles(look, kind, state);
                let bg = ctx.color(bg_role);
                let fg = ctx.color(fg_role);
                // Pressed swaps the colours; released restores them.
                let (fill, ink) = match e {
                    KeyFeedback::Pressed { .. } => (fg, bg),
                    KeyFeedback::Released { .. } => (bg, fg),
                };
                set_label_color(ctx, label, ink);
                let radius = ctx.me().radius;
                ctx.set_paint(Paint::Quad(Quad::new(fill).radius(radius)));
            });
        }

        Key {
            label,
            look,
            lock_bar,
            popup_text: popup_text(&b.key),
            radius,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::tokens::{Font, Fonts};
    use crate::layout::{Cell, load_keymap};
    use crate::modifiers::latch::LatchedState;
    use crate::widget::keyboard::{cell_style, row_style, view_span};

    struct Leaf;
    impl Build for Leaf {
        type Widget = Leaf;
    }
    impl Widget for Leaf {
        type Builder = Leaf;
        fn build(b: Leaf, _: Handle<Self>, _: &mut Spawner<'_, Self>) -> Self {
            b
        }
    }

    /// The base view's rows laid out as the keyboard lays them out, at the
    /// design width (540, so rows 532 wide between the 4 px side padding)
    /// with an 8 px gap and 50 px rows: each key's (width, height), by token.
    fn base_row_sizes() -> Vec<Vec<(String, f32, f32)>> {
        let keymap = load_keymap();
        let mut app = App::new();
        app.add_module(LayoutModule)
            .add_module(PaintModule)
            .add_module(MechanixTheme::dark());
        let mut atlas = Atlas::new();
        let fonts = Fonts::load(&mut atlas);
        let icons = Icons::load(&mut atlas, 20);
        app.insert_resource(atlas);
        app.insert_resource(icons);
        app.insert_resource(LatchedState::default());
        app.insert_resource(Shift::default());
        app.insert_resource(PointingStickMode::default());

        let view = keymap.views.iter().find(|v| v.name == "base").unwrap();
        let span = view_span(view);
        let mut rows = Vec::new();
        for row in &view.rows {
            let root = app.root();
            let row_node = app.spawn_with(
                root,
                Leaf,
                (LayoutRoot(true), row_style().size(px(532.0), px(50.0))),
            );
            let mut keys = Vec::new();
            for cell in &row.cells {
                let cell_node =
                    app.spawn_with(row_node, Leaf, (cell_style(cell.width(), span, 8.0),));
                let Cell::Key(key) = cell else { continue };
                let h = app.spawn(
                    cell_node,
                    KeyBuilder {
                        font: fonts.id(Font::Geist),
                        font_size: 20,
                        hint_font: fonts.id(Font::GeistMono),
                        key: key.clone(),
                        radius: 0.0,
                    },
                );
                keys.push((key.token.clone(), h));
            }
            rows.push(keys);
        }
        app.tick();
        rows.into_iter()
            .map(|keys| {
                keys.into_iter()
                    .map(|(t, h)| {
                        let r = app.component::<ComputedLayout>(h).unwrap().rect;
                        (t, r.width(), r.height())
                    })
                    .collect()
            })
            .collect()
    }

    fn base_row_widths() -> Vec<Vec<(String, f32)>> {
        base_row_sizes()
            .into_iter()
            .map(|row| row.into_iter().map(|(t, w, _)| (t, w)).collect())
            .collect()
    }

    fn width(row: &[(String, f32)], token: &str) -> f32 {
        row.iter().find(|(t, _)| t == token).unwrap().1
    }

    /// Key widths follow the outline units, whatever the label or the row:
    /// every 10-wide key is as wide as `q`, in any row, and a wider key spans
    /// that many letters plus the gaps between them.
    #[test]
    fn key_widths_follow_outline_units_not_labels() {
        let rows = base_row_widths();
        let letter = width(&rows[0], "q");
        // 10 % of 532, less the 8 px gap.
        assert!((letter - 45.2).abs() <= 1.0, "q is {letter}");
        for (token, w) in rows.iter().flatten() {
            let w = *w;
            match token.as_str() {
                "pointing_stick" => assert!(w < letter, "stick {w}, q {letter}"),
                "shift" | "BackSpace" | "to_symbols" | "Return" => {
                    // 15 wide: one and a half letters plus half a gap.
                    let want = 1.5 * letter + 4.0;
                    assert!((w - want).abs() <= 1.0, "{token} is {w}, want {want}");
                }
                "space" => {
                    let want = 4.0 * letter + 3.0 * 8.0;
                    assert!((w - want).abs() <= 1.0, "space {w}, want {want}");
                }
                _ => assert!((w - letter).abs() <= 1.0, "{token} is {w}, q is {letter}"),
            }
        }
    }

    /// The pointing stick is a square, centred in its row.
    #[test]
    fn pointing_stick_is_square() {
        let rows = base_row_sizes();
        let (_, w, h) = rows[1]
            .iter()
            .find(|(t, _, _)| t == "pointing_stick")
            .cloned()
            .unwrap();
        assert!((w - h).abs() <= 1.0, "stick is {w} x {h}");
    }
}
