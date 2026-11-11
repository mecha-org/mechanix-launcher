//! The search line at the top of the drawer: a search icon, the field and a
//! clear button over a 2px line that spans the full width.

use mecha_wayland::mechanix_widgets::{self as mw, FontContextExt};
use mecha_wayland::prelude::*;

/// From the top of the drawer, under the status bar, to the top of the
/// field's text.
const ABOVE_TEXT: f32 = 20.0;
/// Over the field's row, which centres the text in it.
const TOP: f32 = ABOVE_TEXT - (FIELD_HEIGHT - TEXT_PX as f32) / 2.0;
const BOTTOM: f32 = 8.0;
/// The field's row, without the line under it.
const FIELD_HEIGHT: f32 = 54.0;
const LINE: f32 = 2.0;
const GLYPH: u32 = 22;
const CLEAR_GLYPH: u32 = 18;
/// The clear button's touch target (its width; it spans the field's height).
const CLEAR: f32 = 44.0;
const TEXT_PX: u16 = 20;

/// The parts of the search bar the drawer drives after spawning it.
#[derive(Clone, Copy)]
pub(crate) struct SearchBar {
    pub input: Handle<Input>,
    pub clear: Handle<Div>,
    glyph: Handle<Icon>,
    placeholder: Handle<Div>,
    line: Handle<Div>,
}

impl SearchBar {
    /// Spawns the bar, idle and empty, as the last child of `parent`.
    pub(crate) fn spawn<W: Widget>(s: &mut Spawner<'_, W>, parent: impl Into<NodeId>) -> Self {
        let (search, cross) = {
            let mut atlas = s.resource_mut::<Atlas>();
            (
                atlas.insert_svg(assets::icons::SEARCH, GLYPH),
                atlas.insert_svg(assets::icons::CLEAR, CLEAR_GLYPH),
            )
        };
        let search = search.expect("embedded search glyph is a valid SVG");
        let cross = cross.expect("embedded clear glyph is a valid SVG");

        let root = s.spawn(
            parent,
            div().style(
                LayoutStyle::default()
                    .column()
                    .width(percent(100.0))
                    .shrink(0.0)
                    .padding(Insets::new(px(TOP), px(0.0), px(BOTTOM), px(0.0))),
            ),
        );
        let field = s.spawn(
            root,
            div().style(
                LayoutStyle::default()
                    .row()
                    .align_items(Align::Center)
                    .gap(px(12.0))
                    .size(percent(100.0), px(FIELD_HEIGHT))
                    .padding(Insets::new(px(0.0), px(12.0), px(0.0), px(24.0))),
            ),
        );
        let glyph = s.spawn(
            field,
            icon(search)
                .color(s.color(ColorRole::OnSurface))
                .style(LayoutStyle::default().shrink(0.0)),
        );

        // The placeholder sits behind the field, shown while it is empty.
        let slot = s.spawn(
            field,
            div().style(
                LayoutStyle::default()
                    .relative()
                    .row()
                    .align_items(Align::Center)
                    .grow(1.0)
                    .min_width(px(0.0))
                    .height(percent(100.0)),
            ),
        );
        let placeholder = s.spawn(
            slot,
            div().style(
                LayoutStyle::default()
                    .absolute()
                    .inset(Insets::all(px(0.0)))
                    .row()
                    .align_items(Align::Center),
            ),
        );
        s.spawn(
            placeholder,
            mw::text("Search")
                .size(f32::from(TEXT_PX))
                .color(ColorRole::OnSurfaceVariant),
        );
        let font = s.font(FontWeight::Regular);
        let caret = s.color(ColorRole::Primary);
        let text = s.color(ColorRole::OnSurface);
        let input = s.spawn(
            slot,
            input(font)
                .size(TEXT_PX)
                .color(text)
                .caret(caret)
                .border(0.0, Color::TRANSPARENT)
                // Text never runs under the clear button.
                .style(LayoutStyle::default().grow(1.0).padding(Insets::new(
                    px(0.0),
                    px(CLEAR),
                    px(0.0),
                    px(0.0),
                ))),
        );
        // The whole slot is the field's press target.
        *s.component_mut::<LayoutStyle>(input).unwrap() = LayoutStyle::default()
            .row()
            .align_items(Align::Center)
            .size(percent(100.0), percent(100.0));

        // Over the input's right end, so a press on it still hits the input
        // and text-input does not blur the field.
        let clear = s.spawn(
            slot,
            div().style(
                LayoutStyle::default()
                    .absolute()
                    .inset(Insets::new(px(0.0), px(0.0), px(0.0), auto()))
                    .width(px(CLEAR))
                    .center()
                    .hidden(),
            ),
        );
        s.spawn(
            clear,
            icon(cross).color(s.color(ColorRole::OnSecondaryContainer)),
        );

        let line = s.spawn(
            root,
            div()
                .background(s.color(ColorRole::SecondaryFixedDim))
                .style(LayoutStyle::default().size(percent(100.0), px(LINE))),
        );

        SearchBar {
            input,
            clear,
            glyph,
            placeholder,
            line,
        }
    }

    /// `active` (focused or holding text) turns the line and icon `Primary`;
    /// text hides the placeholder and shows the clear button.
    pub(crate) fn show<W: Widget>(&self, ctx: &mut Context<'_, W>, active: bool, has_text: bool) {
        let (glyph, line) = if active {
            (ColorRole::Primary, ColorRole::Primary)
        } else {
            (ColorRole::OnSurface, ColorRole::SecondaryFixedDim)
        };
        let (glyph, line) = (ctx.color(glyph), ctx.color(line));
        if let Some(mut c) = ctx.at(self.glyph) {
            c.set_color(glyph);
        }
        if let Some(mut c) = ctx.at(self.line) {
            c.set_background(line);
        }
        let shown = |on: bool| if on { Display::Flex } else { Display::Hidden };
        if let Some(mut c) = ctx.at(self.clear) {
            c.set_display(shown(has_text));
        }
        if let Some(mut c) = ctx.at(self.placeholder) {
            c.set_display(shown(!has_text));
        }
    }
}
