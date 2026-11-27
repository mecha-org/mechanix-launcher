//! The pointing stick's direction pad: four key-sized cells around the held
//! pointing stick, each showing its arrow, the held direction's cell in the
//! `latched` key style. It is a keyboard container child painted above the
//! keys, absolutely positioned, draw-only, and driven by the input layer's
//! [`PointingStickFeedback`].

use mecha_wayland::prelude::*;

use crate::layout::icons::Icons;
use crate::layout::style::{KeyLook, KeyState};
use crate::touch::pointing_stick::Direction;
use crate::widget::key::{KeyKind, key_roles};

/// The input layer's word to the direction pad.
#[derive(Clone, Debug)]
pub(crate) enum PointingStickFeedback {
    /// Show the pad at `rect` (relative to the keyboard container), each
    /// direction's cell at `cells[direction.index()]` (relative to `rect`),
    /// `lit` lit.
    Open {
        rect: Rect,
        cells: [Rect; 4],
        lit: Option<Direction>,
    },
    /// Light this direction's cell instead, or none.
    Light(Option<Direction>),
    /// Hide the pad.
    Close,
}
impl Event for PointingStickFeedback {}

/// Build args for the [`PointingStickOverlay`]: the corner radius and the
/// colours (cells in the look's normal state, the held direction's in its
/// latched state).
pub(crate) struct PointingStickOverlayBuilder {
    pub(crate) radius: f32,
    pub(crate) look: KeyLook,
}

impl Build for PointingStickOverlayBuilder {
    type Widget = PointingStickOverlay;
}

/// The (background, foreground) roles of a cell, lit or not.
#[derive(Clone, Copy)]
struct Colors {
    cell: (ColorRole, ColorRole),
    lit: (ColorRole, ColorRole),
}

/// One direction's cell and its arrow (none if the icon is missing).
#[derive(Clone, Copy)]
struct Cell {
    node: Handle<Div>,
    arrow: Option<Handle<Icon>>,
}

/// The direction pad. Hidden until the pointing stick is held.
pub(crate) struct PointingStickOverlay {
    /// By [`Direction::index`].
    cells: [Cell; 4],
    lit: Option<Direction>,
    colors: Colors,
    /// Corner radius at the current scale (a rescale updates it).
    pub(crate) radius: f32,
}

impl Widget for PointingStickOverlay {
    type Builder = PointingStickOverlayBuilder;
    fn build(b: PointingStickOverlayBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default().absolute().hidden();
        let colors = Colors {
            // The pointing stick key's own base colour.
            cell: key_roles(b.look, KeyKind::Function, KeyState::Normal),
            lit: key_roles(b.look, KeyKind::Normal, KeyState::Latched),
        };
        let ink = s.color(colors.cell.1);
        let cells = Direction::ALL.map(|d| {
            let node = s.spawn(me, div().style(LayoutStyle::default().absolute().center()));
            let arrow = match s.resource::<Icons>().get(d.icon()) {
                Some(sprite) => Some(s.spawn(node, icon(sprite).color(ink))),
                None => {
                    tracing::warn!(icon = d.icon(), "pointing stick arrow icon missing");
                    None
                }
            };
            Cell { node, arrow }
        });

        s.on::<PointingStickFeedback>(me, |ctx, e| match e {
            PointingStickFeedback::Open { rect, cells, lit } => open(ctx, *rect, cells, *lit),
            PointingStickFeedback::Light(lit) => light(ctx, *lit),
            PointingStickFeedback::Close => {
                ctx.me().lit = None;
                ctx.set_display(Display::Hidden);
            }
        });
        s.on_theme(me, |ctx| {
            let lit = ctx.me().lit;
            light(ctx, lit);
        });

        PointingStickOverlay {
            cells,
            lit: None,
            colors,
            radius: b.radius,
        }
    }
}

/// Lay the pad out at `rect`, its cells at `cells`, and paint it.
fn open(
    ctx: &mut Context<'_, PointingStickOverlay>,
    rect: Rect,
    cells: &[Rect; 4],
    lit: Option<Direction>,
) {
    ctx.set_style(
        LayoutStyle::default()
            .absolute()
            .size(px(rect.size.width), px(rect.size.height))
            .inset(Insets::new(
                px(rect.origin.y),
                auto(),
                auto(),
                px(rect.origin.x),
            )),
    );
    for (cell, r) in ctx
        .me()
        .cells
        .iter()
        .copied()
        .zip(cells)
        .collect::<Vec<_>>()
    {
        ctx.at(cell.node).unwrap().set_style(
            LayoutStyle::default()
                .absolute()
                .center()
                .size(px(r.size.width), px(r.size.height))
                .inset(Insets::new(px(r.origin.y), auto(), auto(), px(r.origin.x))),
        );
    }
    light(ctx, lit);
}

/// Paint every cell, `lit`'s in the lit colours.
fn light(ctx: &mut Context<'_, PointingStickOverlay>, lit: Option<Direction>) {
    ctx.me().lit = lit;
    let (colors, radius, cells) = {
        let me = ctx.me();
        (me.colors, me.radius, me.cells)
    };
    for d in Direction::ALL {
        let (bg, fg) = if lit == Some(d) {
            colors.lit
        } else {
            colors.cell
        };
        let (fill, ink) = (ctx.color(bg), ctx.color(fg));
        let cell = cells[d.index()];
        ctx.at(cell.node)
            .unwrap()
            .set_paint(Paint::Quad(Quad::new(fill).radius(radius)));
        if let Some(arrow) = cell.arrow {
            ctx.at(arrow).unwrap().set_color(ink);
        }
    }
}
