//! The long-press choices row: a strip of key-sized cells over the held key,
//! the highlighted one in the primary colours. It is the keyboard container's
//! last child (painting above the keys and the pop-up), absolutely
//! positioned, draw-only, and driven by the input layer's [`ChoicesFeedback`].

use mecha_wayland::prelude::*;

use crate::layout::style::{KeyLook, KeyState};
use crate::widget::key::{KeyKind, key_roles};

/// The input layer's word to the choices row. Cells count left to right.
#[derive(Clone, Debug)]
pub(crate) enum ChoicesFeedback {
    /// Show the row at `rect` (relative to the keyboard container), one cell
    /// per label, `highlighted` lit.
    Open {
        rect: Rect,
        cells: Vec<String>,
        highlighted: Option<usize>,
    },
    /// Light cell `0` instead, or none.
    Highlight(Option<usize>),
    /// Hide the row.
    Close,
}
impl Event for ChoicesFeedback {}

/// Build args for the [`MoreKeys`] row: the key font, label size, corner
/// radius and colours (cells in the look's normal state, the highlighted
/// cell in its latched state).
pub(crate) struct MoreKeysBuilder {
    pub(crate) font: FontId,
    pub(crate) font_size: u16,
    pub(crate) radius: f32,
    pub(crate) look: KeyLook,
}

/// The (background, foreground) roles of a cell, lit or not.
#[derive(Clone, Copy)]
struct Colors {
    cell: (ColorRole, ColorRole),
    lit: (ColorRole, ColorRole),
}

impl Build for MoreKeysBuilder {
    type Widget = MoreKeys;
}

/// The choices row. Hidden until a key is long-pressed; its cells are spawned
/// per opening.
pub(crate) struct MoreKeys {
    cells: Vec<(Handle<Div>, Handle<Text>)>,
    highlighted: Option<usize>,
}

impl Widget for MoreKeys {
    type Builder = MoreKeysBuilder;
    fn build(b: MoreKeysBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default().absolute().hidden();
        let (font, font_size, radius) = (b.font, b.font_size, b.radius);
        let colors = Colors {
            cell: key_roles(b.look, KeyKind::Normal, KeyState::Normal),
            lit: key_roles(b.look, KeyKind::Normal, KeyState::Latched),
        };
        s.on::<ChoicesFeedback>(me, move |ctx, e| match e {
            ChoicesFeedback::Open {
                rect,
                cells,
                highlighted,
            } => {
                let look = (font, font_size, radius, colors);
                open(ctx, *rect, cells, *highlighted, look);
            }
            ChoicesFeedback::Highlight(h) => highlight(ctx, *h, radius, colors),
            ChoicesFeedback::Close => close(ctx),
        });
        MoreKeys {
            cells: Vec::new(),
            highlighted: None,
        }
    }
}

/// Lay the row out at `rect` with a cell per label and paint it.
fn open(
    ctx: &mut Context<'_, MoreKeys>,
    rect: Rect,
    labels: &[String],
    highlighted: Option<usize>,
    (font, font_size, radius, colors): (FontId, u16, f32, Colors),
) {
    close(ctx);
    ctx.set_style(
        LayoutStyle::default()
            .row()
            .absolute()
            .size(px(rect.size.width), px(rect.size.height))
            .inset(Insets::new(
                px(rect.origin.y),
                auto(),
                auto(),
                px(rect.origin.x),
            )),
    );
    let fill = ctx.color(colors.cell.0);
    ctx.set_paint(Paint::Quad(Quad::new(fill).radius(radius)));
    let ink = ctx.color(colors.cell.1);
    let cell_w = rect.size.width / labels.len().max(1) as f32;
    let me = ctx.handle();
    let cells = labels
        .iter()
        .map(|label| {
            let cell = ctx.spawn(
                me,
                div().style(
                    LayoutStyle::default()
                        .center()
                        .size(px(cell_w), px(rect.size.height)),
                ),
            );
            let text = ctx.spawn(cell, text(font, label.as_str()).color(ink).size(font_size));
            (cell, text)
        })
        .collect();
    ctx.me().cells = cells;
    highlight(ctx, highlighted, radius, colors);
}

/// Light cell `h` (and unlight the one before).
fn highlight(ctx: &mut Context<'_, MoreKeys>, h: Option<usize>, radius: f32, colors: Colors) {
    let old = std::mem::replace(&mut ctx.me().highlighted, h);
    let (on_fill, on_ink) = (ctx.color(colors.lit.0), ctx.color(colors.lit.1));
    let off_ink = ctx.color(colors.cell.1);
    let cells = ctx.me().cells.clone();
    if let Some((cell, text)) = old.and_then(|i| cells.get(i)) {
        ctx.at(*cell).unwrap().set_paint(Paint::None);
        ctx.at(*text).unwrap().set_color(off_ink);
    }
    if let Some((cell, text)) = h.and_then(|i| cells.get(i)) {
        let quad = Quad::new(on_fill).radius(radius);
        ctx.at(*cell).unwrap().set_paint(Paint::Quad(quad));
        ctx.at(*text).unwrap().set_color(on_ink);
    }
}

/// Hide the row and drop its cells.
fn close(ctx: &mut Context<'_, MoreKeys>) {
    ctx.set_display(Display::Hidden);
    ctx.me().highlighted = None;
    for (cell, _) in std::mem::take(&mut ctx.me().cells) {
        ctx.remove(cell);
    }
}
