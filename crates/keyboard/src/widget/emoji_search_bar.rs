//! The emoji search bar: it takes the suggestion bar's place over an emoji
//! page, and over the letters while an emoji search is typed. Like the
//! suggestion bar it is a plain strip with a thin line under it: a field
//! (an icon and the "Search emoji" prompt, or the query with its caret)
//! and, while searching, a back button on its left and, once something is
//! typed, a clear button on its right. It is absolutely
//! positioned over the keyboard container's top padding, draw-only: the
//! input layer hit-tests it ([`EmojiSearchBar::part_at`]) and drives it
//! with [`SearchBarFeedback`].

use mecha_wayland::prelude::*;

use crate::layout::icons::Icons;

/// The field's icon.
const FIELD_ICON: &str = "system-search-symbolic";
/// The back button's icon.
const BACK_ICON: &str = "go-previous-symbolic";
/// The clear button's icon (the backspace key's).
const CLEAR_ICON: &str = "edit-clear-symbolic";
/// What the field says while nothing is typed.
const PROMPT: &str = "Search emoji";
/// Drawn at the query's caret.
const CARET: char = '|';

/// Text draws at this share of the key font size, as suggestions do.
const FONT_SCALE: f32 = 0.9;
/// Icons are this share of the bar's height.
const ICON_SCALE: f32 = 0.5;
/// Width of the line under the bar and right of the back button, logical px.
const LINE: f32 = 1.0;
/// Height of the line right of the back button, as a share of the bar's.
const DIVIDER_HEIGHT: f32 = 0.4;

const LINE_ROLE: ColorRole = ColorRole::Outline;
const INK_ROLE: ColorRole = ColorRole::OnSurface;
const PROMPT_ROLE: ColorRole = ColorRole::OnSurfaceVariant;
const PRESSED_ROLE: ColorRole = ColorRole::SurfaceContainerHigh;

/// A part of the bar a touch can land on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SearchPart {
    /// The back button (only while searching).
    Back,
    /// The field.
    Field,
    /// The clear button (only while something is typed).
    Clear,
}

/// The keyboard's word to the bar.
#[derive(Clone, Debug)]
pub(crate) enum SearchBarFeedback {
    /// Show the prompt (`typing` false: over an emoji page), or the query
    /// with the caret before char `caret` and the back button.
    Show {
        typing: bool,
        query: String,
        caret: usize,
    },
    /// Light a part as pressed, or none.
    Press(Option<SearchPart>),
}
impl Event for SearchBarFeedback {}

pub(crate) struct EmojiSearchBarBuilder {
    pub(crate) font: FontId,
    pub(crate) font_size: u16,
    /// Height, logical px.
    pub(crate) height: f32,
    /// Side padding of the field and the back button, logical px.
    pub(crate) padding: f32,
}

impl Build for EmojiSearchBarBuilder {
    type Widget = EmojiSearchBar;
}

pub(crate) struct EmojiSearchBar {
    back: Handle<Div>,
    back_icon: Option<Handle<Icon>>,
    divider: Handle<Div>,
    field: Handle<Div>,
    clear: Handle<Div>,
    clear_icon: Option<Handle<Icon>>,
    field_icon: Option<Handle<Icon>>,
    text: Handle<Text>,
    /// Whether the query (not the prompt) shows.
    typing: bool,
    /// Whether anything is typed (the text is the query, not the prompt).
    has_query: bool,
    pressed: Option<SearchPart>,
}

impl EmojiSearchBar {
    /// The layout style for a bar `height` tall, `top` below the top of
    /// the keyboard container.
    pub(crate) fn style(height: f32, top: f32) -> LayoutStyle {
        LayoutStyle::default()
            .row()
            .absolute()
            .align_items(Align::Center)
            .size(percent(100.0), px(height))
            .inset(Insets::new(px(top), px(0.0), auto(), px(0.0)))
    }

    /// The part under `point`, given the back button's, the clear
    /// button's and the field's laid-out rectangles (a button's `None`
    /// while hidden).
    pub(crate) fn part_at(
        back: Option<Rect>,
        clear: Option<Rect>,
        field: Rect,
        point: Point,
    ) -> Option<SearchPart> {
        if back.is_some_and(|r| r.contains(point)) {
            Some(SearchPart::Back)
        } else if clear.is_some_and(|r| r.contains(point)) {
            Some(SearchPart::Clear)
        } else if field.contains(point) {
            Some(SearchPart::Field)
        } else {
            None
        }
    }

    /// The back button's node, if it shows.
    pub(crate) fn back(&self) -> Option<Handle<Div>> {
        self.typing.then_some(self.back)
    }

    /// The clear button's node, if it shows.
    pub(crate) fn clear(&self) -> Option<Handle<Div>> {
        self.has_query.then_some(self.clear)
    }

    /// The field's node.
    pub(crate) fn field(&self) -> Handle<Div> {
        self.field
    }
}

impl Widget for EmojiSearchBar {
    type Builder = EmojiSearchBarBuilder;
    fn build(b: EmojiSearchBarBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        *s.component_mut::<LayoutStyle>(me).unwrap() = Self::style(b.height, 0.0).hidden();
        let line = s.color(LINE_ROLE);
        *s.component_mut::<Paint>(me).unwrap() = Paint::Quad(underline(line));
        s.on_theme(me, |ctx| {
            let line = ctx.color(LINE_ROLE);
            ctx.set_paint(Paint::Quad(underline(line)));
            restyle(ctx);
        });
        s.on::<SearchBarFeedback>(me, |ctx, e| match e {
            SearchBarFeedback::Show {
                typing,
                query,
                caret,
            } => show(ctx, *typing, query, *caret),
            SearchBarFeedback::Press(p) => {
                ctx.me().pressed = *p;
                restyle(ctx);
            }
        });

        let icon_size = (b.height * ICON_SCALE).round();
        let ink = s.color(INK_ROLE);
        let (back_sprite, field_sprite, clear_sprite) = {
            let icons = s.resource::<Icons>();
            (
                icons.get(BACK_ICON),
                icons.get(FIELD_ICON),
                icons.get(CLEAR_ICON),
            )
        };
        let icon_style = LayoutStyle::default()
            .size(px(icon_size), px(icon_size))
            .shrink(0.0);

        // The back and clear buttons: square cells, hidden until needed.
        let button_style = LayoutStyle::default()
            .row()
            .align_items(Align::Center)
            .justify(Justify::Center)
            .size(px(b.height + 2.0 * b.padding), percent(100.0))
            .shrink(0.0)
            .hidden();
        let back = s.spawn(me, div().style(button_style.clone()));
        let back_icon =
            back_sprite.map(|sp| s.spawn(back, icon(sp).color(ink).style(icon_style.clone())));
        let divider = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .size(px(LINE), percent(DIVIDER_HEIGHT * 100.0))
                    .shrink(0.0)
                    .hidden(),
            ),
        );

        // The field: the rest of the bar, its icon and text on the left.
        let field = s.spawn(
            me,
            div().style(
                LayoutStyle::default()
                    .row()
                    .align_items(Align::Center)
                    .gap(px(b.padding))
                    .height(percent(100.0))
                    .flex_basis(px(0.0))
                    .grow(1.0)
                    .shrink(1.0)
                    .min_width(px(0.0))
                    .padding(Insets::new(px(0.0), px(b.padding), px(0.0), px(b.padding))),
            ),
        );
        let field_icon =
            field_sprite.map(|sp| s.spawn(field, icon(sp).color(ink).style(icon_style.clone())));
        let font_size = (f32::from(b.font_size) * FONT_SCALE).round() as u16;
        let text = s.spawn(
            field,
            text(b.font, PROMPT)
                .color(s.color(PROMPT_ROLE))
                .size(font_size)
                .overflow(TextOverflow::Ellipsis)
                .opaque(false),
        );
        let clear = s.spawn(me, div().style(button_style));
        let clear_icon =
            clear_sprite.map(|sp| s.spawn(clear, icon(sp).color(ink).style(icon_style)));

        EmojiSearchBar {
            back,
            back_icon,
            divider,
            field,
            field_icon,
            clear,
            clear_icon,
            text,
            typing: false,
            has_query: false,
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

/// What the field's text says: the prompt while nothing is typed (and
/// never before searching), else the query with the caret drawn before
/// char `caret`.
pub(crate) fn field_text(typing: bool, query: &str, caret: usize) -> String {
    if !typing {
        return PROMPT.to_string();
    }
    if query.is_empty() {
        return format!("{CARET}{PROMPT}");
    }
    let at = query
        .char_indices()
        .nth(caret)
        .map_or(query.len(), |(i, _)| i);
    format!("{}{CARET}{}", &query[..at], &query[at..])
}

fn show(ctx: &mut Context<'_, EmojiSearchBar>, typing: bool, query: &str, caret: usize) {
    let has_query = typing && !query.is_empty();
    let (back, divider, clear, text) = {
        let me = ctx.me();
        me.typing = typing;
        me.has_query = has_query;
        (me.back, me.divider, me.clear, me.text)
    };
    let display = |shown: bool| {
        if shown {
            Display::Flex
        } else {
            Display::Hidden
        }
    };
    ctx.at(back).unwrap().set_display(display(typing));
    ctx.at(divider).unwrap().set_display(display(typing));
    ctx.at(clear).unwrap().set_display(display(has_query));
    ctx.at(text)
        .unwrap()
        .set_text(field_text(typing, query, caret));
    restyle(ctx);
}

/// Paint the parts for the pressed state, the query and the theme.
fn restyle(ctx: &mut Context<'_, EmojiSearchBar>) {
    let fill = ctx.color(PRESSED_ROLE);
    let line = ctx.color(LINE_ROLE);
    let ink = ctx.color(INK_ROLE);
    let prompt = ctx.color(PROMPT_ROLE);
    let (back, divider, field, clear, text, icons, has_query, pressed) = {
        let me = ctx.me();
        (
            me.back,
            me.divider,
            me.field,
            me.clear,
            me.text,
            [me.back_icon, me.field_icon, me.clear_icon],
            me.has_query,
            me.pressed,
        )
    };
    let lit = |part| {
        if pressed == Some(part) {
            Paint::Quad(Quad::new(fill))
        } else {
            Paint::None
        }
    };
    ctx.at(back).unwrap().set_paint(lit(SearchPart::Back));
    ctx.at(field).unwrap().set_paint(lit(SearchPart::Field));
    ctx.at(clear).unwrap().set_paint(lit(SearchPart::Clear));
    ctx.at(divider)
        .unwrap()
        .set_paint(Paint::Quad(Quad::new(line)));
    for icon in icons.into_iter().flatten() {
        ctx.at(icon).unwrap().set_color(ink);
    }
    ctx.at(text)
        .unwrap()
        .set_color(if has_query { ink } else { prompt });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn part_at_finds_the_buttons_and_the_field() {
        let back = Rect::new(0.0, 0.0, 40.0, 40.0);
        let field = Rect::new(41.0, 0.0, 300.0, 40.0);
        let clear = Rect::new(341.0, 0.0, 40.0, 40.0);
        let p = |x| Point::new(x, 20.0);
        let at = |b, c, x| EmojiSearchBar::part_at(b, c, field, p(x));
        assert_eq!(at(Some(back), Some(clear), 10.0), Some(SearchPart::Back));
        assert_eq!(at(Some(back), Some(clear), 100.0), Some(SearchPart::Field));
        assert_eq!(at(Some(back), Some(clear), 360.0), Some(SearchPart::Clear));
        assert_eq!(at(Some(back), Some(clear), 400.0), None);
        // Hidden buttons are nothing.
        assert_eq!(at(None, None, 10.0), None);
        assert_eq!(at(None, None, 360.0), None);
    }

    #[test]
    fn field_text_draws_the_prompt_or_the_query_and_caret() {
        assert_eq!(field_text(false, "", 0), "Search emoji");
        assert_eq!(field_text(false, "piz", 3), "Search emoji");
        assert_eq!(field_text(true, "", 0), "|Search emoji");
        assert_eq!(field_text(true, "piz", 3), "piz|");
        assert_eq!(field_text(true, "piz", 1), "p|iz");
        assert_eq!(field_text(true, "café", 4), "café|");
        assert_eq!(field_text(true, "café", 3), "caf|é");
    }
}
