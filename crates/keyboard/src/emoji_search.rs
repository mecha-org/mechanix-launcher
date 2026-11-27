//! Emoji search: over an emoji page the suggestion bar gives way to the
//! emoji search bar ([`EmojiSearchBar`]); a tap on it shows the letters,
//! whose keys then type into a query of the keyboard's own instead of the
//! focused field, and a row of results (a [`SuggestionBar`]) over the
//! search bar. A tap on a result types the emoji into the field and leaves
//! the search open; the back button (or the emoji key) goes back to the
//! emoji page the search started from. A space-bar glide moves the query's
//! caret; the pointing stick does nothing meanwhile.
//!
//! [`EmojiSearchBar`]: crate::widget::emoji_search_bar::EmojiSearchBar
//! [`SuggestionBar`]: crate::widget::suggestion_bar::SuggestionBar

use mecha_wayland::prelude::*;

use crate::complete::Completion;
use crate::input;
use crate::layout::Page;
use crate::modifiers::shift::{self, Shift};
use crate::view::{switch_view, view_for};
use crate::widget::emoji_search_bar::SearchBarFeedback;
use crate::widget::keyboard::{Keyboard, apply_top_bar};
use crate::widget::suggestion_bar::{BarFeedback, BarLabel};

/// How many results the row above the search bar shows.
pub(crate) const RESULT_SLOTS: usize = 8;

/// A search being typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct EmojiSearch {
    /// What is typed, lowercase.
    pub(crate) query: String,
    /// The query's caret, in chars.
    pub(crate) caret: usize,
    /// The emoji page to go back to.
    pub(crate) return_view: usize,
    /// The results the row shows, left to right.
    pub(crate) results: Vec<String>,
}

impl EmojiSearch {
    pub(crate) fn new(return_view: usize) -> Self {
        Self {
            query: String::new(),
            caret: 0,
            return_view,
            results: Vec::new(),
        }
    }

    /// The byte offset of char `n` of the query (its end past the last).
    fn byte_at(&self, n: usize) -> usize {
        self.query
            .char_indices()
            .nth(n)
            .map_or(self.query.len(), |(i, _)| i)
    }

    /// Type `text` (lowercased) at the caret, the caret after it.
    pub(crate) fn push(&mut self, text: &str) {
        let text = text.to_lowercase();
        let at = self.byte_at(self.caret);
        self.query.insert_str(at, &text);
        self.caret += text.chars().count();
    }

    /// Delete the char before the caret; whether there was one.
    pub(crate) fn pop(&mut self) -> bool {
        if self.caret == 0 {
            return false;
        }
        let at = self.byte_at(self.caret - 1);
        self.query.remove(at);
        self.caret -= 1;
        true
    }

    /// Delete the whole query; whether there was any.
    pub(crate) fn clear(&mut self) -> bool {
        self.caret = 0;
        !std::mem::take(&mut self.query).is_empty()
    }

    /// Move the caret `n` chars left or right, stopping at the query's
    /// ends.
    pub(crate) fn step(&mut self, left: bool, n: u32) {
        let n = n as usize;
        let len = self.query.chars().count();
        self.caret = if left {
            self.caret.saturating_sub(n)
        } else {
            (self.caret + n).min(len)
        };
    }
}

/// Whether an emoji search is being typed.
pub(crate) fn active(ctx: &mut Context<'_, Keyboard>) -> bool {
    ctx.me().search.is_some()
}

/// The search bar's field was tapped over an emoji page: start a search
/// from it, showing the letters of the focused field's family.
pub(crate) fn enter(ctx: &mut Context<'_, Keyboard>) {
    if active(ctx) {
        return;
    }
    let (letters, here) = {
        let me = ctx.me();
        (view_for(&me.views, me.family, Page::Letters), me.current_view)
    };
    let Some(letters) = letters else {
        tracing::warn!("no letters view to search emoji with");
        return;
    };
    ctx.resource_mut::<Shift>().reset();
    shift::refresh(ctx);
    ctx.me().search = Some(EmojiSearch::new(here));
    let name = ctx.me().views[letters].name.clone();
    switch_view(ctx, &name);
    refresh(ctx);
    tracing::info!("emoji search started");
}

/// Back from the search to the emoji page it started from.
pub(crate) fn exit(ctx: &mut Context<'_, Keyboard>) {
    let Some(search) = ctx.me().search.take() else {
        return;
    };
    let name = ctx.me().views.get(search.return_view).map(|v| v.name.clone());
    match name {
        Some(name) if ctx.me().current_view != search.return_view => switch_view(ctx, &name),
        // Already there: the bars still change.
        _ => apply_top_bar(ctx),
    }
    show_prompt(ctx);
}

/// Drop the search without switching views: the field changed or the
/// keyboard hid. The caller shows what comes next.
pub(crate) fn cancel(ctx: &mut Context<'_, Keyboard>) {
    if ctx.me().search.take().is_some() {
        show_prompt(ctx);
    }
}

/// Type `text` into the query.
pub(crate) fn type_text(ctx: &mut Context<'_, Keyboard>, text: &str) {
    if let Some(s) = ctx.me().search.as_mut() {
        s.push(text);
    }
    refresh(ctx);
}

/// Delete the char before the query's caret.
pub(crate) fn backspace(ctx: &mut Context<'_, Keyboard>) {
    let changed = ctx.me().search.as_mut().is_some_and(EmojiSearch::pop);
    if changed {
        refresh(ctx);
    }
}

/// Clear the whole query.
pub(crate) fn clear(ctx: &mut Context<'_, Keyboard>) {
    let changed = ctx.me().search.as_mut().is_some_and(EmojiSearch::clear);
    if changed {
        refresh(ctx);
    }
}

/// Move the query's caret `n` chars.
pub(crate) fn step(ctx: &mut Context<'_, Keyboard>, left: bool, n: u32) {
    if let Some(s) = ctx.me().search.as_mut() {
        s.step(left, n);
    }
    show_query(ctx);
}

/// Type result `i` into the focused field; the search stays open.
pub(crate) fn pick(ctx: &mut Context<'_, Keyboard>, i: usize) {
    let emoji = ctx
        .me()
        .search
        .as_ref()
        .and_then(|s| s.results.get(i).cloned());
    if let Some(emoji) = emoji {
        input::commit_emoji(ctx, &emoji);
    }
}

/// Search for the query and show it and its results.
pub(crate) fn refresh(ctx: &mut Context<'_, Keyboard>) {
    let Some(query) = ctx.me().search.as_ref().map(|s| s.query.clone()) else {
        return;
    };
    let results = ctx
        .resource::<Completion>()
        .emoji_search(&query, RESULT_SLOTS);
    let labels = results
        .iter()
        .map(|e| Some(BarLabel::Emoji(e.clone())))
        .collect();
    if let Some(s) = ctx.me().search.as_mut() {
        s.results = results;
    }
    let row = ctx.me().results_bar.id();
    ctx.emit(BarFeedback::Show(labels), [row]);
    show_query(ctx);
}

/// Show the query and its caret in the search bar.
fn show_query(ctx: &mut Context<'_, Keyboard>) {
    let Some((query, caret)) = ctx
        .me()
        .search
        .as_ref()
        .map(|s| (s.query.clone(), s.caret))
    else {
        return;
    };
    let bar = ctx.me().search_bar.id();
    ctx.emit(
        SearchBarFeedback::Show {
            typing: true,
            query,
            caret,
        },
        [bar],
    );
}

/// Put the search bar back to its prompt.
fn show_prompt(ctx: &mut Context<'_, Keyboard>) {
    let bar = ctx.me().search_bar.id();
    ctx.emit(
        SearchBarFeedback::Show {
            typing: false,
            query: String::new(),
            caret: 0,
        },
        [bar],
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typing_goes_in_at_the_caret_lowercase() {
        let mut s = EmojiSearch::new(3);
        s.push("P");
        s.push("zz");
        assert_eq!((s.query.as_str(), s.caret), ("pzz", 3));
        s.step(true, 2);
        s.push("i");
        assert_eq!((s.query.as_str(), s.caret), ("pizz", 2));
        assert_eq!(s.return_view, 3);
    }

    #[test]
    fn backspace_deletes_before_the_caret() {
        let mut s = EmojiSearch::new(0);
        s.push("café");
        assert!(s.pop());
        assert_eq!(s.query, "caf");
        s.step(true, 1);
        assert!(s.pop());
        assert_eq!((s.query.as_str(), s.caret), ("cf", 1));
        s.step(true, 5);
        assert!(!s.pop());
        assert_eq!(s.query, "cf");
    }

    #[test]
    fn clear_empties_the_query() {
        let mut s = EmojiSearch::new(0);
        s.push("pizza");
        s.step(true, 2);
        assert!(s.clear());
        assert_eq!((s.query.as_str(), s.caret), ("", 0));
        assert!(!s.clear());
    }

    #[test]
    fn the_caret_stays_in_the_query() {
        let mut s = EmojiSearch::new(0);
        s.push("red héart");
        s.step(false, 4);
        assert_eq!(s.caret, 9);
        s.step(true, 20);
        assert_eq!(s.caret, 0);
        s.step(false, 5);
        s.push("x");
        assert_eq!(s.query, "red hxéart");
    }
}
