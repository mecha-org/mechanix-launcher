//! The suggestion bar across the top of the keys: suggestions left to
//! right, a thin vertical line between them, and a thin line under the bar
//! separating it from the keys. It is the keyboard container's first child,
//! absolutely positioned over the container's top padding, draw-only: the
//! input layer hit-tests it ([`SuggestionBar::cell_at`]) and drives it with
//! [`BarFeedback`].

use mecha_wayland::prelude::*;

/// The bar is this many key rows tall.
pub(crate) const BAR_ROW_SCALE: f32 = 0.8;
/// Suggestions draw at this share of the key font size.
const FONT_SCALE: f32 = 0.9;
/// Width of the lines under and between suggestions, logical px.
const LINE: f32 = 1.0;
/// Height of the line between suggestions, as a share of the bar's.
const DIVIDER_HEIGHT: f32 = 0.4;

const LINE_ROLE: ColorRole = ColorRole::OutlineVariant;
const INK_ROLE: ColorRole = ColorRole::OnSurface;
const PRESSED_ROLE: ColorRole = ColorRole::SurfaceContainerHigh;

/// The input layer's (and the keyboard's) word to the bar.
#[derive(Clone, Debug)]
pub(crate) enum BarFeedback {
    /// Show these labels, in order, none pressed.
    Show(Vec<String>),
    /// Light cell `i` as pressed, or none.
    Press(Option<usize>),
}
impl Event for BarFeedback {}

pub(crate) struct SuggestionBarBuilder {
    pub(crate) font: FontId,
    pub(crate) font_size: u16,
    /// Height, logical px.
    pub(crate) height: f32,
    /// Side padding of each cell, logical px.
    pub(crate) padding: f32,
}

impl Build for SuggestionBarBuilder {
    type Widget = SuggestionBar;
}

/// The bar. Its cells are spawned per [`BarFeedback::Show`].
pub(crate) struct SuggestionBar {
    font: FontId,
    font_size: u16,
    /// Side padding of each cell (a rescale updates it).
    pub(crate) padding: f32,
    /// One per suggestion, in order.
    cells: Vec<Handle<Div>>,
    /// The lines between cells.
    dividers: Vec<Handle<Div>>,
    pressed: Option<usize>,
}

impl SuggestionBar {
    /// The layout style for a bar `height` tall.
    pub(crate) fn style(height: f32) -> LayoutStyle {
        LayoutStyle::default()
            .row()
            .absolute()
            .align_items(Align::Center)
            .size(percent(100.0), px(height))
            .inset(Insets::new(px(0.0), px(0.0), auto(), px(0.0)))
    }

    /// The cell index under `point`, given each cell's laid-out rectangle,
    /// if any.
    pub(crate) fn cell_at(cells: &[Rect], point: Point) -> Option<usize> {
        cells.iter().position(|r| r.contains(point))
    }

    /// Each cell's handle, in order.
    pub(crate) fn cells(&self) -> &[Handle<Div>] {
        &self.cells
    }
}

impl Widget for SuggestionBar {
    type Builder = SuggestionBarBuilder;
    fn build(b: SuggestionBarBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = Self::style(b.height).hidden();
        let line = s.color(LINE_ROLE);
        *s.component_mut::<Paint>(me).unwrap() = Paint::Quad(underline(line));
        s.on_theme(me, |ctx| {
            let line = ctx.color(LINE_ROLE);
            ctx.set_paint(Paint::Quad(underline(line)));
            restyle_cells(ctx);
        });
        s.on::<BarFeedback>(me, |ctx, e| match e {
            BarFeedback::Show(labels) => show(ctx, labels),
            BarFeedback::Press(i) => press(ctx, *i),
        });
        SuggestionBar {
            font: b.font,
            font_size: (f32::from(b.font_size) * FONT_SCALE).round() as u16,
            padding: b.padding,
            cells: Vec::new(),
            dividers: Vec::new(),
            pressed: None,
        }
    }
}

/// A transparent quad with only a bottom line.
fn underline(color: Color) -> Quad {
    Quad::new(Color::TRANSPARENT)
        .border_widths(Insets::new(0.0, 0.0, LINE, 0.0))
        .border_color(color)
}

/// Replace the cells with one per label.
fn show(ctx: &mut Context<'_, SuggestionBar>, labels: &[String]) {
    for cell in std::mem::take(&mut ctx.me().cells) {
        ctx.remove(cell);
    }
    for divider in std::mem::take(&mut ctx.me().dividers) {
        ctx.remove(divider);
    }
    ctx.me().pressed = None;
    let me = ctx.handle();
    let (font, font_size, padding) = {
        let bar = ctx.me();
        (bar.font, bar.font_size, bar.padding)
    };
    let ink = ctx.color(INK_ROLE);
    let line = ctx.color(LINE_ROLE);
    for (i, label) in labels.iter().enumerate() {
        if i > 0 {
            let divider = ctx.spawn(
                me,
                div().background(line).style(
                    LayoutStyle::default()
                        .size(px(LINE), percent(DIVIDER_HEIGHT * 100.0))
                        .shrink(0.0),
                ),
            );
            ctx.me().dividers.push(divider);
        }
        // Cells take their text's width, shrinking (with an ellipsis) only
        // when the labels do not fit.
        let cell = ctx.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .row()
                    .align_items(Align::Center)
                    .height(percent(100.0))
                    .min_width(px(0.0))
                    .padding(Insets::new(px(0.0), px(padding), px(0.0), px(padding))),
            ),
        );
        ctx.spawn(
            cell,
            text(font, label.as_str())
                .color(ink)
                .size(font_size)
                .overflow(TextOverflow::Ellipsis)
                .opaque(false),
        );
        ctx.me().cells.push(cell);
    }
}

/// Light cell `i` as pressed (and unlight the one before).
fn press(ctx: &mut Context<'_, SuggestionBar>, i: Option<usize>) {
    ctx.me().pressed = i;
    restyle_cells(ctx);
}

/// Paint every cell for the pressed state and the current theme.
fn restyle_cells(ctx: &mut Context<'_, SuggestionBar>) {
    let fill = ctx.color(PRESSED_ROLE);
    let line = ctx.color(LINE_ROLE);
    let (cells, dividers, pressed) = {
        let bar = ctx.me();
        (bar.cells.clone(), bar.dividers.clone(), bar.pressed)
    };
    for (i, cell) in cells.into_iter().enumerate() {
        let paint = if pressed == Some(i) {
            Paint::Quad(Quad::new(fill))
        } else {
            Paint::None
        };
        ctx.at(cell).unwrap().set_paint(paint);
    }
    for divider in dividers {
        ctx.at(divider)
            .unwrap()
            .set_paint(Paint::Quad(Quad::new(line)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cell_at_finds_the_cell_under_the_point() {
        let cells = [
            Rect::new(0.0, 0.0, 80.0, 40.0),
            Rect::new(81.0, 0.0, 60.0, 40.0),
        ];
        assert_eq!(
            SuggestionBar::cell_at(&cells, Point::new(10.0, 20.0)),
            Some(0)
        );
        assert_eq!(
            SuggestionBar::cell_at(&cells, Point::new(100.0, 20.0)),
            Some(1)
        );
        // Past the last cell: blank bar, nothing.
        assert_eq!(
            SuggestionBar::cell_at(&cells, Point::new(300.0, 20.0)),
            None
        );
    }
}
