//! Keep awake: a tap keeps the screen on for the user, or stops. While on,
//! the screen never blanks and the system never suspends for being idle.

use mecha_wayland::mechanix_widgets::prelude::{Text, TextContextExt, TextVariant, text};
use mecha_wayland::prelude::*;
use services::{KeepAwake, SetKeepAwake};

use super::{Borders, DOT_RADIUS, build_tile, label_spacer, tile_quad};

const ICON: &[u8] = include_bytes!("../../assets/coffee.svg");

pub struct KeepAwakeTile {
    name: Handle<Text>,
    dot: Handle<Div>,
    borders: Borders,
}

pub struct KeepAwakeTileBuilder {
    /// Which sides get a border, e.g. [`borders::ALL`](super::borders::ALL).
    pub borders: Borders,
}

impl Build for KeepAwakeTileBuilder {
    type Widget = KeepAwakeTile;
}

/// [`KeepAwake`] changed: every tile shows it.
struct Changed;
impl Event for Changed {}

pub(crate) fn on_changed(app: &mut App, _: &OnChanged<KeepAwake>) {
    let tiles: Vec<NodeId> = app.widgets::<KeepAwakeTile>().map(|(id, _)| id).collect();
    app.emit(Changed, tiles);
}

/// The user keeps the screen awake.
fn user(ctx: &Context<'_, KeepAwakeTile>) -> bool {
    ctx.resource::<KeepAwake>().user
}

impl Widget for KeepAwakeTile {
    type Builder = KeepAwakeTileBuilder;

    fn build(b: Self::Builder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        build_tile(me, s, ICON, b.borders);
        let spacer = label_spacer(me, s);
        s.component_mut::<LayoutStyle>(spacer).unwrap().display = Display::Flex;
        let name = s.spawn(
            me,
            text("Auto")
                .variant(TextVariant::LabelLarge)
                .max_lines(1)
                .style(LayoutStyle::default().shrink(0.0))
                .color(ColorRole::OnSurfaceVariant),
        );
        let dot = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .size(px(DOT_RADIUS), px(DOT_RADIUS))
                    .absolute()
                    .inset(Insets::new(px(8.0), px(10.0), auto(), auto())),
            ),
        );
        s.on::<Clicked>(me, |ctx, _| {
            let on = user(ctx);
            ctx.signal(SetKeepAwake(!on));
        });
        s.on::<Changed>(me, |ctx, _| show(ctx));
        s.on_theme(me, show);
        KeepAwakeTile {
            name,
            dot,
            borders: b.borders,
        }
    }
}

/// Look like the Bluetooth tile: filled with a dot while on.
fn show(ctx: &mut Context<'_, KeepAwakeTile>) {
    let (on, borders) = (user(ctx), ctx.me().borders);
    let bg = if on {
        ctx.color(ColorRole::Surface)
    } else {
        Color::TRANSPARENT
    };
    let border = ctx.color(ColorRole::SecondaryFixedDim);
    ctx.set_paint(Paint::Quad(tile_quad(bg, border, borders)));
    let dot = if on {
        Paint::Quad(Quad::new(ctx.color(ColorRole::Primary)).radius(DOT_RADIUS))
    } else {
        Paint::None
    };
    let (name, d) = (ctx.me().name, ctx.me().dot);
    if let Some(mut t) = ctx.at(name) {
        t.set_text(if on { "Stay on" } else { "Auto" });
    }
    if let Some(mut d) = ctx.at(d) {
        d.set_paint(dot);
    }
}
