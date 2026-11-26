//! The suggestion bar across the top of the keys: equal slots left to
//! right, the best suggestion in the middle one, each word centred in its
//! slot, a thin vertical line between two shown words, and a thin line under the bar
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

// Outline, not OutlineVariant: the theme sets the variant to the keyboard's own
// background (surface-container-low) in both schemes, which hides the lines.
const LINE_ROLE: ColorRole = ColorRole::Outline;
const INK_ROLE: ColorRole = ColorRole::OnSurface;
const PRESSED_ROLE: ColorRole = ColorRole::SurfaceContainerHigh;

/// The input layer's (and the keyboard's) word to the bar.
#[derive(Clone, Debug)]
pub(crate) enum BarFeedback {
    /// Show these slots, left to right (`None` an empty slot), none
    /// pressed.
    Show(Vec<Option<String>>),
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

/// The bar. Its cells (one per slot, sharing the width equally) persist: a
/// [`BarFeedback::Show`] relabels them in place, adding or removing only the
/// difference in count, so a word that stays in its slot is not redrawn.
pub(crate) struct SuggestionBar {
    font: FontId,
    font_size: u16,
    /// Side padding of each cell (a rescale updates it).
    pub(crate) padding: f32,
    /// One per slot, left to right.
    cells: Vec<Handle<Div>>,
    /// Each cell's label.
    texts: Vec<Handle<Text>>,
    /// Whether each cell shows a word (an empty one cannot be pressed).
    filled: Vec<bool>,
    /// The lines between cells, `dividers[i]` after `cells[i]`.
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

    /// The cell index under `point`, given each cell's laid-out rectangle
    /// and whether it shows a word, if it falls on one that does.
    pub(crate) fn cell_at(cells: &[Rect], filled: &[bool], point: Point) -> Option<usize> {
        cells
            .iter()
            .position(|r| r.contains(point))
            .filter(|&i| filled.get(i).copied().unwrap_or(false))
    }

    /// Whether each cell shows a word, in order.
    pub(crate) fn filled(&self) -> &[bool] {
        &self.filled
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
            texts: Vec::new(),
            filled: Vec::new(),
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

/// Show `labels`, one per cell (`None` leaves it empty): relabel the cells
/// there are (unchanged labels are left alone), spawn any more needed, and
/// remove any extra. A pressed cell stays lit: the finger is still on it.
fn show(ctx: &mut Context<'_, SuggestionBar>, labels: &[Option<String>]) {
    let texts = ctx.me().texts.clone();
    for (text, label) in texts.iter().zip(labels) {
        ctx.at(*text)
            .unwrap()
            .set_text(label.as_deref().unwrap_or(""));
    }
    while ctx.me().cells.len() > labels.len() {
        let bar = ctx.me();
        let cell = bar.cells.pop().unwrap();
        bar.texts.pop();
        // The line before the cell goes with it; the first cell has none.
        let divider = bar.dividers.pop();
        ctx.remove(cell);
        if let Some(divider) = divider {
            ctx.remove(divider);
        }
    }
    let have = ctx.me().cells.len();
    for label in labels.iter().skip(have) {
        spawn_cell(ctx, label.as_deref().unwrap_or(""));
    }
    ctx.me().filled = labels.iter().map(Option::is_some).collect();
    restyle_cells(ctx);
}

/// Append a cell labelled `label`, after a divider unless it is the first.
fn spawn_cell(ctx: &mut Context<'_, SuggestionBar>, label: &str) {
    let me = ctx.handle();
    let (font, font_size, padding, first) = {
        let bar = ctx.me();
        (bar.font, bar.font_size, bar.padding, bar.cells.is_empty())
    };
    if !first {
        let divider = ctx.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .size(px(LINE), percent(DIVIDER_HEIGHT * 100.0))
                    .shrink(0.0),
            ),
        );
        ctx.me().dividers.push(divider);
    }
    // Cells share the bar's width equally, whatever their words, so a slot
    // stays put; a word too long for its slot ends in an ellipsis.
    let cell = ctx.spawn(
        me,
        div().style(
            LayoutStyle::default()
                .row()
                .align_items(Align::Center)
                .justify(Justify::Center)
                .height(percent(100.0))
                .flex_basis(px(0.0))
                .grow(1.0)
                .shrink(1.0)
                .min_width(px(0.0))
                .padding(Insets::new(px(0.0), px(padding), px(0.0), px(padding))),
        ),
    );
    let ink = ctx.color(INK_ROLE);
    let text = ctx.spawn(
        cell,
        text(font, label)
            .color(ink)
            .size(font_size)
            .overflow(TextOverflow::Ellipsis)
            .opaque(false),
    );
    let bar = ctx.me();
    bar.cells.push(cell);
    bar.texts.push(text);
}

/// Light cell `i` as pressed (and unlight the one before).
fn press(ctx: &mut Context<'_, SuggestionBar>, i: Option<usize>) {
    ctx.me().pressed = i;
    restyle_cells(ctx);
}

/// Paint every cell, label and divider for the pressed state and the
/// current theme. A divider shows only between two shown words.
fn restyle_cells(ctx: &mut Context<'_, SuggestionBar>) {
    let fill = ctx.color(PRESSED_ROLE);
    let line = ctx.color(LINE_ROLE);
    let ink = ctx.color(INK_ROLE);
    let (cells, texts, filled, dividers, pressed) = {
        let bar = ctx.me();
        (
            bar.cells.clone(),
            bar.texts.clone(),
            bar.filled.clone(),
            bar.dividers.clone(),
            bar.pressed,
        )
    };
    for text in texts {
        ctx.at(text).unwrap().set_color(ink);
    }
    for (i, cell) in cells.into_iter().enumerate() {
        let paint = if pressed == Some(i) {
            Paint::Quad(Quad::new(fill))
        } else {
            Paint::None
        };
        ctx.at(cell).unwrap().set_paint(paint);
    }
    for (i, divider) in dividers.into_iter().enumerate() {
        let between_words =
            filled.get(i).copied().unwrap_or(false) && filled.get(i + 1).copied().unwrap_or(false);
        let paint = if between_words {
            Paint::Quad(Quad::new(line))
        } else {
            Paint::None
        };
        ctx.at(divider).unwrap().set_paint(paint);
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
        let filled = [true, true];
        assert_eq!(
            SuggestionBar::cell_at(&cells, &filled, Point::new(10.0, 20.0)),
            Some(0)
        );
        assert_eq!(
            SuggestionBar::cell_at(&cells, &filled, Point::new(100.0, 20.0)),
            Some(1)
        );
        // Past the last cell: blank bar, nothing.
        assert_eq!(
            SuggestionBar::cell_at(&cells, &filled, Point::new(300.0, 20.0)),
            None
        );
        // An empty slot is nothing either.
        assert_eq!(
            SuggestionBar::cell_at(&cells, &[false, true], Point::new(10.0, 20.0)),
            None
        );
    }
}
