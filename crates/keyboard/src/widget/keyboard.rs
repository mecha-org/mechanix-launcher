//! The keyboard widget: spawns one subtree per keymap view (only the current
//! view is shown), hands every touch on it to the input layer
//! ([`touch::attach`](crate::touch::attach)), and handles
//! [`SwitchView`](crate::view::SwitchView) to toggle which view is visible.

use mecha_wayland::prelude::*;

use crate::complete::{self, CARET_SETTLE, CaretMoved, Completion, SuggestionsChanged};
use crate::emoji_search::{self, EmojiSearch};
use crate::layout::style::KeyState;
use crate::layout::tokens::{Font, Fonts};
use crate::layout::{Cell, Family, KeyAction, KeyboardStyle, Keymap, Modifier, Page, Row, View};
use crate::modifiers::pointing_stick;
use crate::modifiers::shift::{self, HOLD_LOCK, Shift};
use crate::predict::TypingContext;
use crate::protocol::input_method::InputMethodState;
use crate::scale::KeyboardGeometry;
use crate::touch::long_press::LONG_PRESS;
use crate::touch::{self, KeyEntry, TouchState, TouchTracker, letter_views};
use crate::view::{BASE_VIEW, SwitchView, switch_view, view_for};
use crate::visibility::{ApplyKeyboardVisibility, KeyboardVisibilityExt};
use crate::widget::key::{Key, KeyBuilder};
use crate::widget::emoji_search_bar::{EmojiSearchBar, EmojiSearchBarBuilder};
use crate::widget::more_keys::{MoreKeys, MoreKeysBuilder};
use crate::widget::pointing_stick::{PointingStickOverlay, PointingStickOverlayBuilder};
use crate::widget::popup::{KeyPopup, KeyPopupBuilder};
use crate::widget::suggestion_bar::{BarFeedback, SuggestionBar, SuggestionBarBuilder};
use crate::widget::surface::{popup_headroom, row_weights, view_height_of};

/// One spawned view: its keymap name and the node toggled
/// `Display::Hidden`/`Flex` on switch.
pub(crate) struct ViewNode {
    pub(crate) family: Family,
    pub(crate) page: Page,
    pub(crate) name: String,
    pub(crate) node: Handle<Div>,
    /// Height the view takes when shown, padding included.
    pub(crate) height: f32,
    /// Each row's height weight, for re-laying the view out.
    pub(crate) weights: Vec<f32>,
    /// Every cell (key or spacer) node, whose half-gap inset follows the
    /// output scale.
    pub(crate) cells: Vec<Handle<Div>>,
}

/// The on-screen keyboard: every view, the index of the one currently shown,
/// and every latching modifier key (for armed-state repaint on latch toggle).
pub(crate) struct Keyboard {
    /// Every view, in keymap order.
    pub(crate) views: Vec<ViewNode>,
    /// Index into `views` of the view currently shown.
    pub(crate) current_view: usize,
    /// The focused field's layout family, set on focus; page keys stay inside it.
    pub(crate) family: Family,
    /// Every latching modifier key across all views, for armed-state repaint on
    /// latch toggle.
    pub(crate) latch_keys: Vec<(Modifier, Handle<Key>)>,
    /// Every Shift key across all views, for repaint on shift state change.
    pub(crate) shift_keys: Vec<Handle<Key>>,
    /// Every pointing stick key across all views, for repaint on mode change.
    pub(crate) pointing_stick_keys: Vec<Handle<Key>>,
    /// The hold-to-lock timer on ⇧, started while a Shift key is pressed.
    pub(crate) shift_timer: TimerId,
    /// Key corner radius, for repainting latch keys.
    pub(crate) radius: f32,
    /// Every key, per view, in keymap order: what the input layer resolves
    /// touches against.
    pub(crate) keys: Vec<Vec<KeyEntry>>,
    /// Per view: whether any key types a letter.
    pub(crate) letter_views: Vec<bool>,
    /// The contact being tracked across press, slide and release.
    pub(crate) tracker: TouchTracker,
    /// The input layer's other per-contact bookkeeping.
    pub(crate) touch: TouchState,
    /// The pressed key's bubble, painted above the keys.
    pub(crate) popup: Handle<KeyPopup>,
    /// The long-press choices row, painted above everything.
    pub(crate) choices_row: Handle<MoreKeys>,
    /// The pointing stick's direction pad, painted above everything.
    pub(crate) pointing_stick: Handle<PointingStickOverlay>,
    /// The long-press timer, started while a key with choices or the
    /// pointing stick is pressed.
    pub(crate) long_press_timer: TimerId,
    /// Height of the strip above the keys the pop-up and choices row may
    /// rise into.
    pub(crate) headroom: f32,
    /// Gap between the choices row and its key, at the current scale.
    pub(crate) popup_gap: f32,
    /// The suggestion bar over the top of the keys.
    pub(crate) bar: Handle<SuggestionBar>,
    /// The emoji search's results, over the search bar while searching.
    pub(crate) results_bar: Handle<SuggestionBar>,
    /// The emoji search bar, in the suggestion bar's place over an emoji
    /// page and under the results while searching.
    pub(crate) search_bar: Handle<EmojiSearchBar>,
    /// The height of one bar at the current scale; 0 when completion is
    /// off (and so there are no bars).
    pub(crate) bar_height: f32,
    /// Whether completion wants the suggestion bar for the focused field.
    pub(crate) suggestions_shown: bool,
    /// Whether emoji can be searched (completion has an emoji list).
    pub(crate) emoji_search: bool,
    /// The emoji search being typed, if any.
    pub(crate) search: Option<EmojiSearch>,
    /// The bars shown over the view now.
    pub(crate) top: TopBar,
}

/// What shows over the view.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TopBar {
    /// Nothing.
    None,
    /// The suggestion bar.
    Suggestions,
    /// The emoji search bar, over an emoji page.
    Picker,
    /// The search results over the emoji search bar, over the letters.
    Searching,
}

impl TopBar {
    /// How many bars' heights it takes.
    pub(crate) fn rows(self) -> f32 {
        match self {
            TopBar::None => 0.0,
            TopBar::Suggestions | TopBar::Picker => 1.0,
            TopBar::Searching => 2.0,
        }
    }

    /// Where the emoji search bar sits, below the container's top: under
    /// the results while searching.
    pub(crate) fn search_bar_top(self, bar_height: f32) -> f32 {
        if self == TopBar::Searching {
            bar_height
        } else {
            0.0
        }
    }
}

impl Keyboard {
    /// The index of the view with this name, for storing as the current view.
    /// Resolved once per switch so later reads stay O(1) array indexing.
    pub(crate) fn index_of(&self, name: &str) -> Option<usize> {
        self.views.iter().position(|v| v.name == name)
    }

    /// The height the keyboard takes now: the shown view, and the bars
    /// that show.
    pub(crate) fn shown_height(&self) -> f32 {
        self.views[self.current_view].height + self.bars_height()
    }

    /// The height of the bars shown over the view.
    pub(crate) fn bars_height(&self) -> f32 {
        self.top.rows() * self.bar_height
    }

    /// The container's padding: [`container_padding`], with the shown
    /// bars' height added on top.
    pub(crate) fn padding(&self, style: &KeyboardStyle) -> Insets<Val> {
        container_padding(style, self.bars_height())
    }

    /// What should show over the view now: the search while one is typed,
    /// the emoji search bar over an emoji page, else the suggestion bar if
    /// completion wants it. The emoji search bar needs the suggestions too:
    /// they mean the input method is active, which typing an emoji takes.
    pub(crate) fn wanted_top(&self) -> TopBar {
        if self.bar_height <= 0.0 {
            TopBar::None
        } else if self.search.is_some() {
            TopBar::Searching
        } else if !self.suggestions_shown {
            TopBar::None
        } else if self.emoji_search && self.views[self.current_view].page == Page::Emoji {
            TopBar::Picker
        } else {
            TopBar::Suggestions
        }
    }
}

/// The keyboard container's padding, `extra_top` added on top. Each cell
/// insets its key by half a gap, so the sides take half a gap less: the
/// outermost keys then sit `padding` from the edges, as the top and bottom
/// rows do.
pub(crate) fn container_padding(style: &KeyboardStyle, extra_top: f32) -> Insets<Val> {
    let side = (style.padding - style.gap / 2.0).max(0.0);
    Insets::new(
        px(style.padding + extra_top),
        px(side),
        px(style.padding),
        px(side),
    )
}

/// A row of cells, centred: a row adding up to less than its view's widest
/// leaves the same room at both ends.
pub(crate) fn row_style() -> LayoutStyle {
    LayoutStyle::default().row().justify(Justify::Center)
}

/// What a view's rows are measured against: 100, or its widest row if that
/// is wider or every row is narrower (a 40-wide pad fills the keyboard).
pub(crate) fn view_span(view: &View) -> f32 {
    let widest = view.rows.iter().map(row_span).fold(0.0, f32::max);
    if widest > 0.0 { widest } else { 100.0 }
}

/// What a row's cells add up to.
pub(crate) fn row_span(row: &Row) -> f32 {
    row.cells.iter().map(Cell::width).sum()
}

/// A cell (key or spacer) `width` of `span` of the row, its key inset by half
/// a `gap` on each side. The gap is inside the cell, so a cell's share of the
/// row never depends on how many others the row has: equal widths are equal
/// sizes in every row.
pub(crate) fn cell_style(width: f32, span: f32, gap: f32) -> LayoutStyle {
    LayoutStyle::default()
        .row()
        .width(percent(width / span * 100.0))
        .shrink(0.0)
        .padding(cell_padding(gap))
}

/// A cell's inset: half a gap at each side.
pub(crate) fn cell_padding(gap: f32) -> Insets<Val> {
    Insets::new(px(0.0), px(gap / 2.0), px(0.0), px(gap / 2.0))
}

pub(crate) struct KeyboardBuilder {
    pub(crate) fonts: Fonts,
    pub(crate) keymap: Keymap,
    /// The suggestion bar's height; 0 when completion is off.
    pub(crate) bar_height: f32,
    /// Whether emoji can be searched (completion has an emoji list).
    pub(crate) emoji_search: bool,
}

impl Build for KeyboardBuilder {
    type Widget = Keyboard;
}

impl Widget for Keyboard {
    type Builder = KeyboardBuilder;
    fn build(b: KeyboardBuilder, me: Handle<Self>, s: &mut Spawner<'_, Self>) -> Self {
        // Keyboard container: a column as wide as its parent and as tall as
        // the shown view, surface background, wherever it is (at home the
        // window puts it on the bottom). Its gap only separates views, and
        // just one view is ever shown.
        let geometry = *s.resource::<KeyboardGeometry>();
        let style = geometry.style();
        let popup_gap = geometry.popup_gap();
        *s.component_mut::<LayoutStyle>(me).unwrap() = LayoutStyle::default()
            .column()
            .width(percent(100.0))
            .shrink(0.0)
            .gap(px(style.row_gap))
            .padding(container_padding(&style, 0.0));
        let bg_role = b.keymap.keyboard.background_color;
        *s.component_mut::<Paint>(me).unwrap() = Paint::Quad(Quad::new(s.color(bg_role)));

        s.on_theme(me, move |ctx| {
            let bg = ctx.color(bg_role);
            ctx.set_paint(Paint::Quad(Quad::new(bg)));
        });

        let initial_view = b
            .keymap
            .views
            .iter()
            .position(|v| v.name == BASE_VIEW)
            .unwrap_or(0);

        // Hidden until a field that gets suggestions is focused; absolute,
        // over the container's top padding, which grows to make room.
        let bar = s.spawn(
            me,
            SuggestionBarBuilder {
                font: b.fonts.id(style.font),
                font_size: style.font_size,
                height: b.bar_height,
                padding: style.padding * 2.0,
                dividers: true,
            },
        );
        let results_bar = s.spawn(
            me,
            SuggestionBarBuilder {
                font: b.fonts.id(style.font),
                font_size: style.font_size,
                height: b.bar_height,
                padding: style.padding,
                dividers: false,
            },
        );
        let search_bar = s.spawn(
            me,
            EmojiSearchBarBuilder {
                font: b.fonts.id(style.font),
                font_size: style.font_size,
                height: b.bar_height,
                padding: style.padding * 2.0,
            },
        );

        // ── spawn all views ─────────────────────────────────────────────
        let mut views = Vec::with_capacity(b.keymap.views.len());
        let mut latch_keys = Vec::new();
        let mut shift_keys = Vec::new();
        let mut pointing_stick_keys = Vec::new();
        let mut keys = Vec::with_capacity(b.keymap.views.len());

        for (vi, view) in b.keymap.views.iter().enumerate() {
            let weights = row_weights(view);
            let height = view_height_of(&style, &weights);
            let view_style = LayoutStyle::default()
                .column()
                .size(percent(100.0), px(height - 2.0 * style.padding))
                .gap(px(style.row_gap));
            let view_style = if vi == initial_view {
                view_style
            } else {
                view_style.hidden()
            };
            let view_div = s.spawn(me, div().style(view_style));
            views.push(ViewNode {
                name: view.name.clone(),
                family: view.family,
                page: view.page,
                node: view_div,
                height,
                weights: weights.clone(),
                cells: Vec::new(),
            });
            let span = view_span(view);
            let mut view_keys = Vec::new();

            for (row, &weight) in view.rows.iter().zip(&weights) {
                // Rows share the view's height by weight.
                let row_div = s.spawn(
                    view_div,
                    div().style(
                        row_style()
                            .width(percent(100.0))
                            .flex_basis(px(0.0))
                            .grow(weight),
                    ),
                );

                for cell in &row.cells {
                    let cell_div = s.spawn(
                        row_div,
                        div().style(cell_style(cell.width(), span, style.gap)),
                    );
                    views.last_mut().unwrap().cells.push(cell_div);
                    // A spacer is a blank cell; it spawns no key, so it is
                    // never in `view_keys` and never hit.
                    let Cell::Key(key) = cell else { continue };
                    // The label keeps its normal font while latched.
                    let label_style = key.look.style(KeyState::Normal);
                    let key_handle = s.spawn(
                        cell_div,
                        KeyBuilder {
                            font: b.fonts.id(label_style.font.unwrap_or(style.font)),
                            font_size: label_style.font_size.unwrap_or(style.font_size),
                            hint_font: b.fonts.id(Font::GeistMono),
                            key: key.clone(),
                            radius: style.radius,
                        },
                    );

                    match key.action {
                        KeyAction::LatchModifier(m) => latch_keys.push((m, key_handle)),
                        KeyAction::Shift => shift_keys.push(key_handle),
                        KeyAction::PointingStick => pointing_stick_keys.push(key_handle),
                        _ => {}
                    }

                    view_keys.push(
                        KeyEntry::new(key_handle, key.action.clone(), key.choices.clone())
                            .with_choice_actions(key.choice_actions.clone()),
                    );
                }
            }
            keys.push(view_keys);
        }

        // Last child, so it paints above every key.
        let popup = s.spawn(
            me,
            KeyPopupBuilder {
                keyboard: me,
                font: b.fonts.id(style.font),
                font_size: style.font_size,
                radius: style.radius,
                headroom: popup_headroom(&style, popup_gap),
                gap: popup_gap,
            },
        );

        // After the pop-up, so it paints above it too.
        let choices_row = s.spawn(
            me,
            MoreKeysBuilder {
                font: b.fonts.id(style.font),
                font_size: style.font_size,
                radius: style.radius,
                look: b.keymap.choices_look,
            },
        );

        // After the choices row; only one of them is ever open.
        let pointing_stick = s.spawn(
            me,
            PointingStickOverlayBuilder {
                radius: style.radius,
                look: b.keymap.pointing_stick_look,
            },
        );

        let shift_timer = s.add_timeout(HOLD_LOCK, |ctx, _| shift::held(ctx));
        let long_press_timer = s.add_timeout(LONG_PRESS, |ctx, _| touch::long_pressed(ctx));

        touch::attach(s, me);
        // A field was focused: cancel any touch first (even on the same
        // view, nothing it holds belongs in the new field), drop Shift and
        // the pointing stick's select mode, and show the field's view. The
        // word was already reset by the input method's activate, which then
        // took the word under the caret from the client: clearing it here
        // would lose that (a tap inside a word re-activates the field).
        s.on::<SwitchView>(me, |ctx, event| {
            touch::abort(ctx);
            ctx.resource_mut::<Shift>().reset();
            shift::refresh(ctx);
            pointing_stick::reset(ctx);
            emoji_search::cancel(ctx);
            ctx.me().family = event.field.family;
            let me = ctx.me();
            if let Some(idx) = view_for(&me.views, event.field.family, Page::Letters) {
                let name = me.views[idx].name.clone();
                switch_view(ctx, &name);
            }
            // The same view: the search's bars still go.
            apply_top_bar(ctx);
        });

        // The suggestions changed: fill the bar, and show or hide it.
        s.on::<SuggestionsChanged>(me, show_suggestions);

        // The caret moved: the bar keeps what it shows until the caret has
        // rested for `CARET_SETTLE` (each move restarts the wait), then
        // follows it.
        let settle_timer = s.add_timeout(CARET_SETTLE, |ctx, _| {
            let e = {
                let (mut completion, typing, st) = ctx.fetch::<(
                    ResMut<Completion>,
                    Res<TypingContext>,
                    Res<InputMethodState>,
                )>();
                complete::settle(&mut completion, typing, st)
            };
            if let Some(e) = e {
                show_suggestions(ctx, &e);
            }
        });
        s.on::<CaretMoved>(me, move |ctx, _| {
            ctx.start_timer(settle_timer);
        });

        // Every hide (flick, input-method deactivate, `hide_keyboard`) ends
        // here: cancel any touch so nothing is typed into a hidden keyboard
        // and no long-press opens a row on it, and drop Shift and the
        // pointing stick's select mode.
        s.on::<ApplyKeyboardVisibility>(me, |ctx, _| {
            if ctx.keyboard_visible() {
                return;
            }
            touch::abort(ctx);
            ctx.resource_mut::<Shift>().reset();
            shift::refresh(ctx);
            pointing_stick::reset(ctx);
            if emoji_search::active(ctx) {
                emoji_search::cancel(ctx);
                apply_top_bar(ctx);
            }
        });

        Keyboard {
            views,
            current_view: initial_view,
            family: Family::Text,
            latch_keys,
            shift_keys,
            pointing_stick_keys,
            shift_timer,
            radius: style.radius,
            letter_views: letter_views(&b.keymap),
            keys,
            tracker: TouchTracker::default(),
            touch: TouchState::default(),
            popup,
            choices_row,
            pointing_stick,
            long_press_timer,
            headroom: popup_headroom(&style, popup_gap),
            popup_gap,
            bar,
            results_bar,
            search_bar,
            bar_height: b.bar_height,
            suggestions_shown: false,
            emoji_search: b.emoji_search,
            search: None,
            top: TopBar::None,
        }
    }
}

/// Fill the suggestion bar (kept filled while another bar shows in its
/// place), showing or hiding it (which changes the keyboard's height) as
/// `e` says. Any touch is dropped first if the bars change under it.
fn show_suggestions(ctx: &mut Context<'_, Keyboard>, e: &SuggestionsChanged) {
    if e.shown != ctx.me().suggestions_shown {
        ctx.me().suggestions_shown = e.shown;
        if ctx.me().wanted_top() != ctx.me().top {
            touch::abort(ctx);
            apply_top_bar(ctx);
        }
    }
    let bar = ctx.me().bar.id();
    ctx.emit(BarFeedback::Show(e.labels.clone()), [bar]);
}

/// Show the bars [`Keyboard::wanted_top`] asks for over the current view:
/// the container's top padding makes or gives back their room, and the
/// surface's zone and input region follow the keyboard's height. Called on
/// every view switch too, the height having changed either way. A contact
/// on a bar that goes away is dropped.
pub(crate) fn apply_top_bar(ctx: &mut Context<'_, Keyboard>) {
    let top = ctx.me().wanted_top();
    if top != ctx.me().top {
        touch::end_bar(ctx);
        ctx.me().top = top;
    }
    let (bar, results, search, bar_h) = {
        let me = ctx.me();
        (me.bar, me.results_bar, me.search_bar, me.bar_height)
    };
    let display = |shown: bool| {
        if shown {
            Display::Flex
        } else {
            Display::Hidden
        }
    };
    ctx.at(bar)
        .unwrap()
        .set_display(display(top == TopBar::Suggestions));
    ctx.at(results)
        .unwrap()
        .set_display(display(top == TopBar::Searching));
    let search_shown = matches!(top, TopBar::Picker | TopBar::Searching);
    ctx.at(search).unwrap().set_style(
        EmojiSearchBar::style(bar_h, top.search_bar_top(bar_h)).display(display(search_shown)),
    );
    let style = ctx.resource::<KeyboardGeometry>().style();
    let insets = ctx.me().padding(&style);
    if let Some(style) = ctx.component::<LayoutStyle>().cloned() {
        ctx.set_style(style.padding(insets));
    }
    crate::widget::surface::view_shown(ctx);
}
