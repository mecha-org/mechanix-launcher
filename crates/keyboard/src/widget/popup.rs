//! The key pop-up: a bubble over the pressed key showing its character, so the
//! finger doesn't hide what was hit. It is the keyboard container's last
//! child (painting above the keys), absolutely positioned, draw-only, and
//! driven by the input layer's [`KeyFeedback`].

use mecha_wayland::prelude::*;

use crate::layout::style::KeyState;
use crate::touch::KeyFeedback;
use crate::widget::key::{Key, KeyKind, key_roles};
use crate::widget::keyboard::Keyboard;

/// The bubble's width as a multiple of the key's.
const WIDTH_SCALE: f32 = 1.4;
/// The bubble's height as a multiple of the key's.
const HEIGHT_SCALE: f32 = 1.3;
/// Gap between the bubble's bottom edge and the key's top, in px.
const GAP: f32 = 4.0;

/// Where the bubble for `key` goes: centred over it, its bottom edge `GAP`
/// above the key, then pushed inside `container` (all in window coordinates).
/// A top-row key's bubble ends up overlapping the key itself.
pub(crate) fn popup_rect(key: Rect, container: Rect) -> Rect {
    let w = key.size.width * WIDTH_SCALE;
    let h = key.size.height * HEIGHT_SCALE;
    let x = key.origin.x + (key.size.width - w) / 2.0;
    let y = key.origin.y - GAP - h;
    let x = x.min(container.right() - w).max(container.origin.x);
    let y = y.min(container.bottom() - h).max(container.origin.y);
    Rect::new(x, y, w, h)
}

/// Build args for the [`KeyPopup`]: the keyboard it floats over, the font,
/// the key label size (the bubble draws at twice it) and the corner radius.
pub(crate) struct KeyPopupBuilder {
    pub(crate) keyboard: Handle<Keyboard>,
    pub(crate) font: FontId,
    pub(crate) font_size: u16,
    pub(crate) radius: f32,
}

impl Build for KeyPopupBuilder {
    type Widget = KeyPopup;
}

/// The pressed key's bubble. Hidden until a key with a character is pressed.
pub(crate) struct KeyPopup {
    label: Handle<Text>,
}

impl Widget for KeyPopup {
    type Builder = KeyPopupBuilder;
    fn build(b: KeyPopupBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default().absolute().hidden();
        let label = s.spawn(me, text(b.font, "").size(b.font_size * 2));

        let keyboard = b.keyboard;
        let radius = b.radius;
        s.on::<KeyFeedback>(me, move |ctx, e| match *e {
            KeyFeedback::Pressed { key } => show(ctx, keyboard, key, radius),
            KeyFeedback::Released { .. } => {
                ctx.set_display(Display::Hidden);
            }
        });
        KeyPopup { label }
    }
}

/// Show the bubble over `key`, or hide it if the key has no character.
fn show(ctx: &mut Context<'_, KeyPopup>, keyboard: Handle<Keyboard>, key: NodeId, radius: f32) {
    let Some((string, look)) = ctx
        .widget::<Key>(key)
        .and_then(|k| k.popup_text.clone().map(|t| (t, k.look)))
    else {
        ctx.set_display(Display::Hidden);
        return;
    };
    let key_node = ctx
        .widget::<Keyboard>(keyboard)
        .and_then(|kb| kb.keys[kb.current_view].iter().find(|k| k.node.id() == key))
        .map(|k| k.node);
    let key_rect = key_node.and_then(|h| layout_rect_of(ctx, h));
    let container = layout_rect_of(ctx, keyboard);
    let (Some(key_rect), Some(container)) = (key_rect, container) else {
        ctx.set_display(Display::Hidden);
        return;
    };

    let r = popup_rect(key_rect, container);
    let (x, y) = (r.origin.x - container.origin.x, r.origin.y - container.origin.y);
    ctx.set_style(
        LayoutStyle::default()
            .center()
            .absolute()
            .size(px(r.size.width), px(r.size.height))
            .inset(Insets::new(px(y), auto(), auto(), px(x))),
    );

    // The pressed key's colours: its fill and ink swapped.
    let (bg_role, fg_role) = key_roles(look, KeyKind::Normal, KeyState::Normal);
    let (fill, ink) = (ctx.color(fg_role), ctx.color(bg_role));
    ctx.set_paint(Paint::Quad(Quad::new(fill).radius(radius)));
    let label = ctx.me().label;
    if let Some(mut t) = ctx.at(label) {
        t.set_text(string);
        t.set_color(ink);
    }
}

/// The window-coordinate rectangle a widget's layout gave it.
fn layout_rect<W: Widget>(ctx: Context<'_, W>) -> Option<Rect> {
    ctx.component::<ComputedLayout>().map(|l| l.rect)
}

/// [`layout_rect`] of another widget, by handle.
fn layout_rect_of<W: Widget, V: Widget>(ctx: &mut Context<'_, W>, h: Handle<V>) -> Option<Rect> {
    layout_rect(ctx.at(h)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTAINER: Rect = Rect::new(0.0, 0.0, 1000.0, 400.0);

    #[test]
    fn popup_sits_above_key() {
        let key = Rect::new(200.0, 100.0, 100.0, 50.0);
        let p = popup_rect(key, CONTAINER);
        assert_eq!((p.size.width, p.size.height), (140.0, 65.0));
        assert_eq!(p.origin.x + p.size.width / 2.0, 250.0);
        assert_eq!(p.bottom(), 96.0);
    }

    #[test]
    fn top_row_popup_clamped_inside() {
        let key = Rect::new(200.0, 10.0, 100.0, 50.0);
        let p = popup_rect(key, CONTAINER);
        assert_eq!(p.origin.y, 0.0);
        assert!(p.bottom() > key.origin.y);
    }

    #[test]
    fn edge_popup_pushed_inward() {
        let left = popup_rect(Rect::new(0.0, 100.0, 100.0, 50.0), CONTAINER);
        assert_eq!(left.origin.x, 0.0);
        let right = popup_rect(Rect::new(900.0, 100.0, 100.0, 50.0), CONTAINER);
        assert_eq!(right.right(), 1000.0);
    }
}
