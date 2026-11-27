//! Wires touches on the keyboard container to keys: each contact event is
//! resolved against the current view's laid-out key rectangles (biased by the
//! next-letter prior), run through the [`TouchTracker`], and its outputs
//! become keystrokes ([`dispatch`], [`hold`], [`release`]) and
//! [`KeyFeedback`] for the key widgets.
//!
//! The tracker's key indices always refer to the current view: when the view
//! changes under an active contact, that contact is cancelled against the old
//! view (its highlight and hold end; nothing is typed). The order of
//! operations per event (deferral, finishing an earlier finger, resolving)
//! lives in [`flow`]; this file binds it to the widgets.

use std::collections::HashMap;

use mecha_wayland::prelude::*;
use xkbcommon::xkb::Keysym;

use super::KeyFeedback;
use super::defer::PendingPress;
use super::flow::{self, Board, Flow};
use super::glide::GlideSpec;
use super::long_press::{ChoicesRow, LONG_PRESS};
use super::pointing_stick::{POINTING_STICK_HOLD, PointingStickPad};
use super::resolve::KeyGeom;
use super::tracker::{TouchOut, TouchTracker};
use crate::complete::{Completion, Kind};
use crate::emoji_search;
use crate::feedbackd;
use crate::field::Field;
use crate::input::{
    accept_emoji_suggestion, accept_suggestion, commit_emoji, dispatch, dispatch_choice, hold,
    hold_direction, release, repeats, tap_direction,
};
use crate::layout::KeyAction;
use crate::layout::emoji::Emoji;
use crate::modifiers::shift::{self, Shift};
use crate::predict::{LetterPrior, Prediction, TypingContext};
use crate::skin_tone;
use crate::visibility::KeyboardVisibilityExt;
use crate::widget::emoji_search_bar::{EmojiSearchBar, SearchBarFeedback, SearchPart};
use crate::widget::keyboard::Keyboard;
use crate::widget::keyboard::TopBar;
use crate::widget::more_keys::ChoicesFeedback;
use crate::widget::pointing_stick::PointingStickFeedback;
use crate::widget::suggestion_bar::{BarFeedback, SuggestionBar};

/// Bookkeeping the input layer keeps besides the tracker.
#[derive(Default)]
pub(crate) struct TouchState {
    /// Last event time per contact, for `PressCancelled` (which carries none).
    last_time: HashMap<ContactId, u32>,
    /// The keysym a running hold is pressing.
    held: Option<Keysym>,
    /// Whether the active profile shows the key pop-up, taken at the press.
    popup: bool,
    /// Deferral and per-contact prior.
    flow: Flow,
    /// The contact that went down on a bar over the keys, and what it is
    /// over. It never reaches the keys.
    bar: Option<(ContactId, BarTarget)>,
    /// The skin tone row the bar contact opened by holding an emoji that
    /// takes one; it then picks from the row, not the bar.
    bar_row: Option<ChoicesRow>,
}

impl TouchState {
    /// The skin tone row a bar contact has open, if any.
    #[cfg(test)]
    pub(crate) fn bar_row(&self) -> Option<&ChoicesRow> {
        self.bar_row.as_ref()
    }
}

/// Which bar a contact is on, and what on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BarTarget {
    /// The suggestion bar, over this suggestion if any.
    Suggestion(Option<usize>),
    /// The emoji search results, over this result if any.
    Result(Option<usize>),
    /// The emoji search bar, over this part if any.
    Search(Option<SearchPart>),
}

impl BarTarget {
    /// The same bar, over `other`'s part if `other` is on it too, else
    /// over nothing: a contact stays on the bar it went down on.
    fn moved_to(self, other: Option<BarTarget>) -> BarTarget {
        use BarTarget::*;
        match (self, other) {
            (Suggestion(_), Some(Suggestion(i))) => Suggestion(i),
            (Result(_), Some(Result(i))) => Result(i),
            (Search(_), Some(Search(p))) => Search(p),
            (Suggestion(_), _) => Suggestion(None),
            (Result(_), _) => Result(None),
            (Search(_), _) => Search(None),
        }
    }

    /// Whether it is over something a release acts on.
    fn on_item(self) -> bool {
        match self {
            BarTarget::Suggestion(i) | BarTarget::Result(i) => i.is_some(),
            BarTarget::Search(p) => p.is_some(),
        }
    }
}

/// The keyboard as the [`flow`] sees it.
struct Kb<'c, 'a>(&'c mut Context<'a, Keyboard>);

impl Board for Kb<'_, '_> {
    fn geoms(&mut self) -> Vec<KeyGeom> {
        key_geoms(self.0)
    }
    fn predict(&mut self) -> (LetterPrior, f32) {
        let (prior, strength, popup) = predict(self.0);
        self.0.me().touch.popup = popup;
        (prior, strength)
    }
    fn repeats(&mut self, index: usize) -> bool {
        let me = self.0.me();
        repeats(&me.keys[me.current_view][index].action)
    }
    fn has_choices(&mut self, index: usize) -> bool {
        let me = self.0.me();
        !me.keys[me.current_view][index].choices.is_empty()
    }
    fn choices_row(&mut self, index: usize) -> Option<ChoicesRow> {
        let me = self.0.me();
        let entry = me.keys[me.current_view].get(index)?;
        let choices = entry.choices.clone();
        if choices.is_empty() {
            return None;
        }
        let tones = entry.skin_tone_base.is_some();
        let gap = me.popup_gap;
        let key = *key_geoms(self.0).get(index)?;
        let bounds = overlay_bounds(self.0)?;
        // A skin-tone row opens with the tone the key shows over it.
        let anchor = if tones {
            skin_tone::choice_of(self.0.resource::<Emoji>().skin_tone())
        } else {
            0
        };
        Some(ChoicesRow::open_at(key.rect, bounds, gap, choices, anchor))
    }
    fn is_pointing_stick(&mut self, index: usize) -> bool {
        let me = self.0.me();
        // While an emoji search is typed it is a plain key that does
        // nothing: no direction pad.
        if me.search.is_some() {
            return false;
        }
        matches!(
            me.keys[me.current_view].get(index).map(|k| &k.action),
            Some(KeyAction::PointingStick)
        )
    }
    fn pointing_stick_pad(&mut self, index: usize, origin: Point) -> Option<PointingStickPad> {
        let gap = self.0.me().popup_gap;
        let key = *key_geoms(self.0).get(index)?;
        let bounds = overlay_bounds(self.0)?;
        Some(PointingStickPad::open(key.rect, bounds, gap, origin))
    }
    fn glide(&mut self, index: usize) -> Option<GlideSpec> {
        // The space bar glides sideways. (A gliding pointing stick would
        // return `GlideSpec::both` here and skip its direction pad.)
        let me = self.0.me();
        let node = me.keys[me.current_view].get(index).and_then(|k| {
            matches!(&k.action, KeyAction::EmitText(t) if t == " ").then_some(k.node)
        })?;
        let rect = self.0.at(node)?.component::<ComputedLayout>()?.rect;
        (rect.height() > 0.0).then(|| GlideSpec::horizontal(rect.height()))
    }
    fn tracker(&mut self) -> &mut TouchTracker {
        &mut self.0.me().tracker
    }
    fn flow(&mut self) -> &mut Flow {
        &mut self.0.me().touch.flow
    }
    fn act(&mut self, out: Vec<TouchOut>, time: u32) {
        run(self.0, out, time);
    }
    fn close(&mut self) {
        self.0.set_keyboard_visibility(false);
    }
}

/// Route every touch on the keyboard container `me` through the input layer.
/// Registered on the container only: the window's `bottom_gap` strip below it
/// gets no handler, so taps there stay dead.
pub(crate) fn attach(s: &mut Spawner<'_, Keyboard>, me: Handle<Keyboard>) {
    s.on::<Press>(me, |ctx, e| {
        if let Some(target) = bar_hit(ctx, e.position) {
            bar_press(ctx, e.contact, target);
            return;
        }
        ctx.me().touch.last_time.insert(e.contact, e.time);
        let p = PendingPress {
            contact: e.contact,
            position: e.position,
            time: e.time,
        };
        flow::press(&mut Kb(ctx), p);
    });

    s.on::<PressMoved>(me, |ctx, e| {
        if let Some(target) = bar_contact(ctx, e.contact) {
            if ctx.me().touch.bar_row.is_some() {
                bar_row_moved(ctx, e.position);
                return;
            }
            let over = bar_hit(ctx, e.position);
            bar_press(ctx, e.contact, target.moved_to(over));
            return;
        }
        ctx.me().touch.last_time.insert(e.contact, e.time);
        flow::moved(&mut Kb(ctx), e.contact, e.position, e.time);
    });

    s.on::<Release>(me, |ctx, e| {
        if bar_contact(ctx, e.contact).is_some() {
            if let Some(row) = ctx.me().touch.bar_row.clone() {
                let target = bar_end(ctx);
                if let (Some(target), Some(choice)) = (target, row.highlighted) {
                    bar_pick_tone(ctx, target, &row, choice);
                }
                return;
            }
            match bar_end(ctx) {
                Some(BarTarget::Suggestion(Some(i))) => accept_suggestion(ctx, i),
                Some(BarTarget::Result(Some(i))) => emoji_search::pick(ctx, i),
                Some(BarTarget::Search(Some(SearchPart::Field))) => emoji_search::enter(ctx),
                Some(BarTarget::Search(Some(SearchPart::Back))) => emoji_search::exit(ctx),
                Some(BarTarget::Search(Some(SearchPart::Clear))) => emoji_search::clear(ctx),
                _ => {}
            }
            return;
        }
        ctx.me().touch.last_time.remove(&e.contact);
        // Commits the last resolved (highlighted) key, not a fresh resolve
        // at the release position: by design.
        flow::release(&mut Kb(ctx), e.contact, e.time);
    });

    s.on::<PressCancelled>(me, |ctx, e| {
        if bar_contact(ctx, e.contact).is_some() {
            bar_end(ctx);
            return;
        }
        let time = ctx.me().touch.last_time.remove(&e.contact).unwrap_or(0);
        flow::cancel(&mut Kb(ctx), e.contact, time);
    });
}

/// The focused field changed or the keyboard hid: cancel any active contact
/// against the current view (nothing is typed, its row closes, its hold and
/// timers end). Call before switching the view.
pub(crate) fn abort(ctx: &mut Context<'_, Keyboard>) {
    bar_end(ctx);
    let me = ctx.me();
    let time = me
        .tracker
        .contact()
        .and_then(|c| me.touch.last_time.get(&c).copied())
        .unwrap_or(0);
    flow::abort(&mut Kb(ctx), time);
}

/// What bar over the keys `p` (window coordinates) falls on, and what on
/// it: `None` off every bar that shows.
fn bar_hit(ctx: &mut Context<'_, Keyboard>, p: Point) -> Option<BarTarget> {
    let (top, bar, results, search) = {
        let me = ctx.me();
        (me.top, me.bar, me.results_bar, me.search_bar)
    };
    match top {
        TopBar::None => None,
        TopBar::Suggestions => cell_hit(ctx, bar, p).map(BarTarget::Suggestion),
        TopBar::Picker => search_hit(ctx, search, p).map(BarTarget::Search),
        TopBar::Searching => cell_hit(ctx, results, p)
            .map(BarTarget::Result)
            .or_else(|| search_hit(ctx, search, p).map(BarTarget::Search)),
    }
}

/// The laid-out rectangle of `node`.
fn rect_of<V: Widget>(ctx: &mut Context<'_, Keyboard>, node: Handle<V>) -> Option<Rect> {
    ctx.at(node)?.component::<ComputedLayout>().map(|l| l.rect)
}

/// Where `p` falls on the cells of `bar`: `None` off it, else the filled
/// cell under `p`, if any.
fn cell_hit(
    ctx: &mut Context<'_, Keyboard>,
    bar: Handle<SuggestionBar>,
    p: Point,
) -> Option<Option<usize>> {
    if !rect_of(ctx, bar)?.contains(p) {
        return None;
    }
    let (cells, filled) = {
        let w = ctx.widget::<SuggestionBar>(bar)?;
        (w.cells().to_vec(), w.filled().to_vec())
    };
    let rects: Vec<Rect> = cells
        .into_iter()
        .map(|c| rect_of(ctx, c).unwrap_or_default())
        .collect();
    Some(SuggestionBar::cell_at(&rects, &filled, p))
}

/// Where `p` falls on the emoji search bar: `None` off it, else the part
/// under `p`, if any.
fn search_hit(
    ctx: &mut Context<'_, Keyboard>,
    bar: Handle<EmojiSearchBar>,
    p: Point,
) -> Option<Option<SearchPart>> {
    if !rect_of(ctx, bar)?.contains(p) {
        return None;
    }
    let (back, clear, field) = {
        let w = ctx.widget::<EmojiSearchBar>(bar)?;
        (w.back(), w.clear(), w.field())
    };
    let back = back.and_then(|b| rect_of(ctx, b));
    let clear = clear.and_then(|c| rect_of(ctx, c));
    let field = rect_of(ctx, field).unwrap_or_default();
    Some(EmojiSearchBar::part_at(back, clear, field, p))
}

/// What the bar `contact` went down on is over, if it went down on one.
fn bar_contact(ctx: &mut Context<'_, Keyboard>, contact: ContactId) -> Option<BarTarget> {
    ctx.me()
        .touch
        .bar
        .filter(|(c, _)| *c == contact)
        .map(|(_, t)| t)
}

/// `contact` is on a bar, over `target`: light it, with a tick of feedback
/// when it lands on a new item.
fn bar_press(ctx: &mut Context<'_, Keyboard>, contact: ContactId, target: BarTarget) {
    let before = ctx.me().touch.bar.replace((contact, target));
    if before.map(|(_, t)| t) == Some(target) {
        return;
    }
    if target.on_item() {
        feedbackd::key_pressed(ctx);
    }
    light(ctx, target);
    // Holding an emoji that takes a skin tone opens its tones; moving onto
    // another item restarts the wait.
    let timer = ctx.me().bar_long_press_timer;
    ctx.cancel_timer(timer);
    if bar_tones(ctx, target).is_some() {
        ctx.start_timer_for(timer, LONG_PRESS);
    }
}

/// Light what `target` is over on its bar (nothing if it is over nothing).
fn light(ctx: &mut Context<'_, Keyboard>, target: BarTarget) {
    let (bar, results, search) = {
        let me = ctx.me();
        (me.bar.id(), me.results_bar.id(), me.search_bar.id())
    };
    match target {
        BarTarget::Suggestion(i) => ctx.emit(BarFeedback::Press(i), [bar]),
        BarTarget::Result(i) => ctx.emit(BarFeedback::Press(i), [results]),
        BarTarget::Search(p) => ctx.emit(SearchBarFeedback::Press(p), [search]),
    }
}

/// The bar contact, if any, ended: unlight its bar and return what it
/// ended over.
fn bar_end(ctx: &mut Context<'_, Keyboard>) -> Option<BarTarget> {
    let (_, target) = ctx.me().touch.bar.take()?;
    let timer = ctx.me().bar_long_press_timer;
    ctx.cancel_timer(timer);
    if ctx.me().touch.bar_row.take().is_some() {
        show_choices(ctx, ChoicesFeedback::Close);
    }
    light(ctx, target.moved_to(None));
    Some(target)
}

/// The emoji a bar item `target` is over, untoned, and its skin tone
/// choices (it untoned, then light to dark), if it takes a tone.
fn bar_tones(ctx: &mut Context<'_, Keyboard>, target: BarTarget) -> Option<(String, Vec<String>)> {
    let base = match target {
        BarTarget::Suggestion(Some(i)) => {
            let s = ctx.resource::<Completion>().get(i)?.clone();
            (s.kind == Kind::Emoji).then_some(s.edit.insert)?
        }
        BarTarget::Result(Some(i)) => ctx.me().search.as_ref()?.results.get(i)?.clone(),
        _ => return None,
    };
    let tones = ctx.resource_mut::<Emoji>().tones(&base)?;
    let choices = std::iter::once(base.clone()).chain(tones).collect();
    Some((base, choices))
}

/// The bar long-press timer fired: open the skin tone row over the held
/// bar emoji, the tone picked last over it.
pub(crate) fn bar_long_pressed(ctx: &mut Context<'_, Keyboard>) {
    let Some((_, target)) = ctx.me().touch.bar else {
        return;
    };
    let (Some((_, choices)), Some(cell)) = (bar_tones(ctx, target), bar_cell_rect(ctx, target))
    else {
        return;
    };
    let Some(bounds) = overlay_bounds(ctx) else {
        return;
    };
    // One emoji-sized (square) cell per choice, the anchor centred over
    // the held item; the row rises into the headroom above the bar.
    let side = cell.size.height;
    let anchor_rect = Rect::new(
        cell.origin.x + (cell.size.width - side) / 2.0,
        cell.origin.y,
        side,
        side,
    );
    let gap = ctx.me().popup_gap;
    let anchor = skin_tone::choice_of(ctx.resource::<Emoji>().skin_tone());
    let row = ChoicesRow::open_at(anchor_rect, bounds, gap, choices, anchor);
    ctx.me().touch.bar_row = Some(row.clone());
    show_emoji_row(ctx, &row);
}

/// The laid-out rectangle of the bar cell `target` is over.
fn bar_cell_rect(ctx: &mut Context<'_, Keyboard>, target: BarTarget) -> Option<Rect> {
    let (bar, i) = match target {
        BarTarget::Suggestion(Some(i)) => (ctx.me().bar, i),
        BarTarget::Result(Some(i)) => (ctx.me().results_bar, i),
        _ => return None,
    };
    let cell = *ctx.widget::<SuggestionBar>(bar)?.cells().get(i)?;
    rect_of(ctx, cell)
}

/// The bar contact moved to `p` with its skin tone row open: light the
/// choice under it, if any.
fn bar_row_moved(ctx: &mut Context<'_, Keyboard>, p: Point) {
    let Some(row) = ctx.me().touch.bar_row.as_mut() else {
        return;
    };
    let before = row.highlighted;
    row.track(p);
    let now = row.highlighted;
    let slot = now.map(|c| row.choice_at(c));
    if now != before {
        show_choices(ctx, ChoicesFeedback::Highlight(slot));
    }
}

/// The bar contact let go on choice `choice` of its skin tone row, held
/// on bar item `target`: type that form of the emoji as the item would,
/// and pick its tone for every emoji.
fn bar_pick_tone(
    ctx: &mut Context<'_, Keyboard>,
    target: BarTarget,
    row: &ChoicesRow,
    choice: usize,
) {
    let Some(emoji) = row.choices.get(choice).cloned() else {
        return;
    };
    match target {
        BarTarget::Suggestion(Some(i)) => accept_emoji_suggestion(ctx, i, &emoji),
        BarTarget::Result(Some(_)) => commit_emoji(ctx, &emoji),
        _ => return,
    }
    let me = ctx.handle().id();
    ctx.emit(skin_tone::PickSkinTone(skin_tone::tone_of(choice)), [me]);
}

/// Show `row`, a row of emoji (its sprites loaded first).
fn show_emoji_row(ctx: &mut Context<'_, Keyboard>, row: &ChoicesRow) {
    let Some(container) = ctx.component::<ComputedLayout>().map(|l| l.rect) else {
        return;
    };
    {
        let (mut emoji, mut atlas) = ctx.fetch::<(ResMut<Emoji>, ResMut<Atlas>)>();
        for cell in &row.choices {
            emoji.ensure(&mut atlas, cell);
        }
    }
    let rect = Rect::new(
        row.rect.origin.x - container.origin.x,
        row.rect.origin.y - container.origin.y,
        row.rect.size.width,
        row.rect.size.height,
    );
    show_choices(
        ctx,
        ChoicesFeedback::Open {
            rect,
            cells: (0..row.choices.len())
                .map(|slot| row.choices[row.choice_at(slot)].clone())
                .collect(),
            highlighted: row.highlighted.map(|c| row.choice_at(c)),
            emoji: true,
        },
    );
}

/// The bars over the keys changed: drop any contact on them, acting on
/// nothing.
pub(crate) fn end_bar(ctx: &mut Context<'_, Keyboard>) {
    bar_end(ctx);
}

/// The long-press timer fired: open the held key's choices row.
pub(crate) fn long_pressed(ctx: &mut Context<'_, Keyboard>) {
    let me = ctx.me();
    let time = me
        .tracker
        .contact()
        .and_then(|c| me.touch.last_time.get(&c).copied())
        .unwrap_or(0);
    flow::long_pressed(&mut Kb(ctx), time);
}

/// Where a choices row or direction pad may open, in window coordinates: the
/// keyboard container and the headroom strip above it, which they may rise
/// into as the pop-up does.
fn overlay_bounds(ctx: &mut Context<'_, Keyboard>) -> Option<Rect> {
    let headroom = ctx.me().headroom;
    let container = ctx.component::<ComputedLayout>()?.rect;
    Some(Rect::new(
        container.origin.x,
        container.origin.y - headroom,
        container.size.width,
        container.size.height + headroom,
    ))
}

/// The prior over the next letter, the prediction strength and whether the
/// pop-up is on, for the focused field and the word typed so far.
fn predict(ctx: &Context<'_, Keyboard>) -> (LetterPrior, f32, bool) {
    let field = Field::current(ctx);
    let (profile, predictor) = ctx.resource::<Prediction>().active(field);
    let prior = predictor.prior(&ctx.resource::<TypingContext>().prefix());
    (prior, profile.strength, profile.popup)
}

/// The current view's keys, with their laid-out rectangles.
fn key_geoms(ctx: &mut Context<'_, Keyboard>) -> Vec<KeyGeom> {
    let view = ctx.me().current_view;
    let keys: Vec<(Handle<_>, Option<char>, bool)> = ctx.me().keys[view]
        .iter()
        .map(|k| (k.node, k.letter, k.hidden))
        .collect();
    keys.into_iter()
        .map(|(node, letter, hidden)| {
            // A hidden key's layout may be stale: put it where no touch is.
            let rect = if hidden {
                Rect::new(f32::MIN, f32::MIN, 0.0, 0.0)
            } else {
                ctx.at(node)
                    .and_then(|c| c.component::<ComputedLayout>().map(|l| l.rect))
                    .unwrap_or_default()
            };
            KeyGeom { rect, letter }
        })
        .collect()
}

/// Act on a batch of tracker outputs against the current view, then cancel
/// the active contact if acting switched the view.
fn run(ctx: &mut Context<'_, Keyboard>, out: Vec<TouchOut>, time: u32) {
    let view = ctx.me().current_view;
    apply(ctx, view, out, time);
    settle(ctx, view, time);
}

/// If the view is no longer `view`, cancel the active contact, whose key
/// indices refer to `view`.
fn settle(ctx: &mut Context<'_, Keyboard>, view: usize, time: u32) {
    let me = ctx.me();
    let now = me.current_view;
    let out = finish_stale(&mut me.tracker, view, now, time);
    apply(ctx, view, out, time);
}

/// The outputs that end `tracker`'s active contact if its view `then` is no
/// longer the current view `now`; none if the view is unchanged.
fn finish_stale(tracker: &mut TouchTracker, then: usize, now: usize, time: u32) -> Vec<TouchOut> {
    if then == now {
        return Vec::new();
    }
    tracker.cancel_active(time)
}

/// Act on the tracker's outputs, in order; indices refer to `view`.
fn apply(ctx: &mut Context<'_, Keyboard>, view: usize, out: Vec<TouchOut>, time: u32) {
    for o in out {
        match o {
            TouchOut::Pressed(i) => {
                feedbackd::key_pressed(ctx);
                if let Some(KeyAction::Shift) = entry_action(ctx, view, i) {
                    shift::pressed(ctx);
                }
                // Holding a key with choices opens them, and holding the
                // pointing stick (in half the time) its direction pad;
                // sliding onto another key restarts the wait.
                let wait = ctx.me().keys[view].get(i).and_then(|k| {
                    if matches!(k.action, KeyAction::PointingStick) {
                        Some(POINTING_STICK_HOLD)
                    } else {
                        (!k.choices.is_empty()).then_some(LONG_PRESS)
                    }
                });
                if let Some(wait) = wait {
                    let id = ctx.me().long_press_timer;
                    ctx.start_timer_for(id, wait);
                }
                feedback(ctx, view, i, |key| KeyFeedback::Pressed { key });
            }
            TouchOut::Unpressed(i) => {
                if let Some(KeyAction::Shift) = entry_action(ctx, view, i) {
                    shift::unpressed(ctx);
                }
                let id = ctx.me().long_press_timer;
                ctx.cancel_timer(id);
                feedback(ctx, view, i, |key| KeyFeedback::Released { key });
            }
            TouchOut::Commit(i) => {
                if let Some(action) = entry_action(ctx, view, i) {
                    dispatch(ctx, &action, time);
                }
            }
            TouchOut::Hold(i) => {
                // A backspace that undid a suggestion holds nothing, so
                // its release neither repeats nor drops the word.
                if let Some(KeyAction::EmitKeysym(ks)) = entry_action(ctx, view, i)
                    && hold(ctx, ks)
                {
                    ctx.me().touch.held = Some(ks);
                }
            }
            TouchOut::StopHold { held_ms } => {
                release(ctx);
                if ctx.me().touch.held.take() == Some(Keysym::BackSpace) {
                    ctx.resource_mut::<TypingContext>().backspace_held(held_ms);
                }
            }
            TouchOut::OpenChoices(i) => open_choices(ctx, view, i),
            TouchOut::HighlightChoice(h) => {
                let slot = ctx
                    .me()
                    .tracker
                    .choices()
                    .and_then(|row| h.map(|c| row.choice_at(c)));
                show_choices(ctx, ChoicesFeedback::Highlight(slot));
            }
            TouchOut::CommitChoice { key, choice } => {
                show_choices(ctx, ChoicesFeedback::Close);
                let entry = ctx.me().keys[view].get(key);
                // A choice with its own action (a keyboard switch) runs it;
                // any other types its text.
                let action = entry.and_then(|k| k.choice_actions.get(choice).cloned());
                let picked = entry.and_then(|k| Some((k.choices.get(choice)?.clone(), k.letter)));
                // A skin-tone choice also becomes the tone of every emoji.
                let tones = entry.is_some_and(|k| k.skin_tone_base.is_some());
                if let Some(action) = action {
                    dispatch(ctx, &action, time);
                } else if let Some((text, letter)) = picked {
                    dispatch_choice(ctx, &text, letter, time);
                }
                if tones {
                    let me = ctx.handle().id();
                    ctx.emit(skin_tone::PickSkinTone(skin_tone::tone_of(choice)), [me]);
                }
            }
            TouchOut::CloseChoices => show_choices(ctx, ChoicesFeedback::Close),
            TouchOut::OpenPointingStick(_) => open_pointing_stick(ctx),
            TouchOut::PointingStick(direction) => {
                show_pointing_stick(ctx, PointingStickFeedback::Light(direction));
                if let Some(d) = direction {
                    feedbackd::key_pressed(ctx);
                    ctx.me().touch.held = Some(d.keysym());
                    hold_direction(ctx, d.keysym());
                }
            }
            TouchOut::ClosePointingStick => {
                show_pointing_stick(ctx, PointingStickFeedback::Close);
            }
            TouchOut::OpenGlide(i) => {
                // Take the contact over: its moves then come straight here,
                // skipping the toolkit's hit test of the whole keyboard tree
                // (every view, the emoji page included) and its enter/exit
                // events on every move of the glide.
                if let Some(c) = ctx.me().tracker.contact() {
                    ctx.claim_contact(c);
                }
                // No hold wait or pop-up while gliding; the key stays lit.
                let id = ctx.me().long_press_timer;
                ctx.cancel_timer(id);
                if let Some(node) = ctx.me().keys[view].get(i).map(|k| k.node.id()) {
                    let popup = ctx.me().popup.id();
                    ctx.emit(KeyFeedback::Released { key: node }, [popup]);
                }
            }
            TouchOut::Glide(steps) => {
                // Feedback once per movement (start, reversal, restart
                // after a rest), not per move or per arrow.
                if steps.tick {
                    feedbackd::key_pressed(ctx);
                }
                for (d, n) in steps.runs() {
                    tap_direction(ctx, d, n);
                }
            }
        }
    }
}

/// Key `i` of `view` was long-pressed and the tracker opened its row: hide
/// the key's pop-up and show the row, cased as shift will type it.
fn open_choices(ctx: &mut Context<'_, Keyboard>, view: usize, i: usize) {
    if let Some(node) = ctx.me().keys[view].get(i).map(|k| k.node.id()) {
        let popup = ctx.me().popup.id();
        ctx.emit(KeyFeedback::Released { key: node }, [popup]);
    }
    let Some(row) = ctx.me().tracker.choices().cloned() else {
        return;
    };
    let Some(container) = ctx.component::<ComputedLayout>().map(|l| l.rect) else {
        return;
    };
    let emoji = ctx.me().keys[view]
        .get(i)
        .is_some_and(|k| k.skin_tone_base.is_some());
    let state = ctx.resource::<Shift>().state();
    let cells: Vec<String> = (0..row.choices.len())
        .map(|slot| {
            let choice = &row.choices[row.choice_at(slot)];
            if emoji {
                choice.clone()
            } else {
                shift::cased_choice(choice, state)
            }
        })
        .collect();
    if emoji {
        let (mut sprites, mut atlas) = ctx.fetch::<(ResMut<Emoji>, ResMut<Atlas>)>();
        for cell in &cells {
            sprites.ensure(&mut atlas, cell);
        }
    }
    let rect = Rect::new(
        row.rect.origin.x - container.origin.x,
        row.rect.origin.y - container.origin.y,
        row.rect.size.width,
        row.rect.size.height,
    );
    let highlighted = row.highlighted.map(|c| row.choice_at(c));
    show_choices(
        ctx,
        ChoicesFeedback::Open {
            rect,
            cells,
            highlighted,
            emoji,
        },
    );
}

/// The tracker opened the pointing stick's direction pad: show it, nothing
/// lit until the finger moves.
fn open_pointing_stick(ctx: &mut Context<'_, Keyboard>) {
    let Some(pad) = ctx.me().tracker.pointing_stick().cloned() else {
        return;
    };
    let Some(container) = ctx.component::<ComputedLayout>().map(|l| l.rect) else {
        return;
    };
    let r = pad.rect();
    let cells = pad.cells.map(|c| {
        Rect::new(
            c.origin.x - r.origin.x,
            c.origin.y - r.origin.y,
            c.size.width,
            c.size.height,
        )
    });
    let rect = Rect::new(
        r.origin.x - container.origin.x,
        r.origin.y - container.origin.y,
        r.size.width,
        r.size.height,
    );
    show_pointing_stick(
        ctx,
        PointingStickFeedback::Open {
            rect,
            cells,
            lit: pad.direction,
        },
    );
}

/// Tell the direction pad widget.
fn show_pointing_stick(ctx: &mut Context<'_, Keyboard>, e: PointingStickFeedback) {
    let pad = ctx.me().pointing_stick.id();
    ctx.emit(e, [pad]);
}

/// Tell the choices row widget.
fn show_choices(ctx: &mut Context<'_, Keyboard>, e: ChoicesFeedback) {
    let row = ctx.me().choices_row.id();
    ctx.emit(e, [row]);
}

/// The action of key `i` in `view`, if it exists.
fn entry_action(ctx: &mut Context<'_, Keyboard>, view: usize, i: usize) -> Option<KeyAction> {
    ctx.me().keys[view].get(i).map(|k| k.action.clone())
}

/// Tell the UI about key `i` of `view`: the key itself and, while the
/// contact's profile has the pop-up on, the pop-up.
fn feedback(
    ctx: &mut Context<'_, Keyboard>,
    view: usize,
    i: usize,
    make: impl FnOnce(NodeId) -> KeyFeedback,
) {
    let Some(node) = ctx.me().keys[view].get(i).map(|k| k.node.id()) else {
        return;
    };
    let me = ctx.me();
    if me.touch.popup {
        let popup = me.popup.id();
        ctx.emit(make(node), [node, popup]);
    } else {
        ctx.emit(make(node), [node]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::touch::tracker::KeyRef;

    const A: ContactId = ContactId::Touch(1);

    #[test]
    fn view_change_cancels_active_contact() {
        let mut t = TouchTracker::default();
        t.press(
            A,
            Some(KeyRef {
                index: 4,
                repeats: false,
                choices: false,
            }),
            0,
        );
        assert_eq!(finish_stale(&mut t, 0, 0, 10), vec![]);
        assert_eq!(finish_stale(&mut t, 0, 1, 10), vec![TouchOut::Unpressed(4)]);
        assert_eq!(t.release(A, 20), vec![]);
    }

    #[test]
    fn view_change_closes_choices_and_types_nothing() {
        let mut t = TouchTracker::default();
        t.press(
            A,
            Some(KeyRef {
                index: 2,
                repeats: false,
                choices: true,
            }),
            0,
        );
        let row = ChoicesRow::open(
            Rect::new(0.0, 100.0, 40.0, 40.0),
            Rect::new(0.0, 0.0, 400.0, 200.0),
            4.0,
            vec!["1".into()],
        );
        t.long_pressed(A, row);
        assert_eq!(
            finish_stale(&mut t, 0, 1, 400),
            vec![TouchOut::CloseChoices, TouchOut::Unpressed(2)]
        );
        assert!(!t.overlay_open());
        assert_eq!(t.release(A, 450), vec![]);
    }

    #[test]
    fn view_change_without_contact_does_nothing() {
        let mut t = TouchTracker::default();
        assert_eq!(finish_stale(&mut t, 0, 1, 10), vec![]);
    }
}
