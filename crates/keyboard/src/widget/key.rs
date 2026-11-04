//! The themed key button: a `div` with a `Paint::Quad` background resolved
//! from a [`ColorRole`] pair, a centred text or icon label, and handlers that
//! re-resolve both colours on theme flip and on the input layer's
//! [`KeyFeedback`]. Keys take no input themselves.

use mecha_wayland::prelude::*;

use crate::latch::LatchedState;
use crate::layout::icons::Icons;
use crate::layout::style::{KeyLook, KeyState};
use crate::layout::{KeyAction, KeyFace};
use crate::touch::{KeyFeedback, popup_text};
use crate::zone::ExclusiveZone;

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
        | KeyAction::ToggleView { .. }
        | KeyAction::HideKeyboard
        | KeyAction::ToggleExclusiveZone
        | KeyAction::Page(_)
        | KeyAction::Shift
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
}

/// Recolour a key's label, whichever kind it is.
pub(crate) fn set_label_color<W: Widget>(ctx: &mut Context<'_, W>, label: Label, fg: Color) {
    match label {
        Label::Text(h) => ctx.at(h).unwrap().set_color(fg),
        Label::Icon(h) => ctx.at(h).unwrap().set_color(fg),
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
    /// What the pop-up shows while this key is pressed; `None` for keys that
    /// don't type a visible character.
    pub(crate) popup_text: Option<String>,
}

impl Widget for Key {
    type Builder = KeyBuilder;
    fn build(b: KeyBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        let kind = kind_of(&b.key.action);
        let look = b.key.look;
        let (bg_role, fg_role) = key_roles(look, kind, KeyState::Normal);
        let radius = b.radius;

        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .center()
            // Outline width is the flex-grow weight; only ratios within a row
            // matter. Kept unscaled so a row's weights never sum below 1, which
            // would leave part of the row's free space unfilled.
            .grow(b.key.width)
            .padding_all(px(2.0));

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
        };

        // Corner hint: top-right, half the label size, not on icon keys.
        let hint = match (&b.key.face, &b.key.hint) {
            (KeyFace::Text(_), Some(h)) => {
                let color = s.color(ColorRole::OnSecondaryContainer);
                let size = (f32::from(b.font_size) * 0.5).round().max(1.0) as u16;
                let handle = s.spawn(me, text(b.hint_font, h).color(color).size(size));
                *s.component_mut::<LayoutStyle>(handle).unwrap() = LayoutStyle::default()
                    .absolute()
                    .inset(Insets::new(px(3.0), px(5.0), auto(), auto()));
                Some(handle)
            }
            _ => None,
        };

        let latch = match b.key.action {
            KeyAction::LatchModifier(m) => Some(m),
            _ => None,
        };
        let zone = matches!(b.key.action, KeyAction::ToggleExclusiveZone);
        // Latch and zone keys show their on state, so a press doesn't flash them.
        let is_latch = latch.is_some() || zone;
        s.on_theme(me, move |ctx| {
            let lit = latch.is_some_and(|m| ctx.resource::<LatchedState>().get(m))
                || (zone && ctx.resource::<ExclusiveZone>().on);
            let state = KeyState::latched_if(lit);
            let (bg_role, fg_role) = key_roles(look, kind, state);
            let bg = ctx.color(bg_role);
            let fg = ctx.color(fg_role);
            ctx.set_paint(Paint::Quad(Quad::new(bg).radius(radius)));
            set_label_color(ctx, label, fg);
            if let Some(h) = hint {
                let c = ctx.color(ColorRole::OnSecondaryContainer);
                ctx.at(h).unwrap().set_color(c);
            }
        });

        if !is_latch {
            s.on::<KeyFeedback>(me, move |ctx, e| {
                let bg = ctx.color(bg_role);
                let fg = ctx.color(fg_role);
                // Pressed swaps the colours; released restores them.
                let (fill, ink) = match e {
                    KeyFeedback::Pressed { .. } => (fg, bg),
                    KeyFeedback::Released { .. } => (bg, fg),
                };
                set_label_color(ctx, label, ink);
                ctx.set_paint(Paint::Quad(Quad::new(fill).radius(radius)));
            });
        }

        Key {
            label,
            look,
            popup_text: popup_text(&b.key),
        }
    }
}
