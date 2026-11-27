//! Word completion and correction for the suggestion bar.
//!
//! A [`Completer`] is a source of candidate words for the word being typed:
//! the bundled word list, hunspell, or several merged. Sources only rank
//! words; [`edit`] turns them into [`Suggestion`]s, each carrying the exact
//! edit a tap on it makes. Which sources run, and in which fields, is chosen
//! in [`config`], the one place to swap them.
//!
//! [`refresh`] runs after every tick: when the word, the focused field or the
//! input method's state changed, it asks the active source again and tells
//! the keyboard ([`SuggestionsChanged`]), which shows or hides the bar and
//! fills it. Typing refreshes at once; a caret move (a tap in the text, a
//! glide, the pointing stick) only once the caret has rested for
//! [`CARET_SETTLE`] ([`CaretMoved`], then [`settle`]), so a moving caret
//! neither runs the sources at every step nor flickers the bar.
//!
//! When a space or punctuation ends a word a source is sure is misspelled,
//! the keyboard replaces it with the correction as it types the separator
//! ([`Completion::autocorrection`]). Accepting a suggestion or an
//! autocorrection can be taken back by a backspace right after
//! ([`Completion::take_undo`]); a word restored that way is not
//! autocorrected again.
//!
//! Typing `:` and part of an emoji's name (`:smil`) fills the bar with the
//! emoji it names instead ([`emoji`]); a tap replaces the `:smil`, and the
//! name is not autocorrected meanwhile.

mod backends;
pub(crate) mod config;
mod edit;
mod emoji;

use std::collections::HashSet;
use std::time::{Duration, Instant};

use mecha_wayland::prelude::*;

pub(crate) use edit::{Suggestion, TextEdit};

use crate::dictionaries::{Dictionaries, EmojiList, Loading};
use crate::field::Field;
use crate::predict::{Change, TypingContext};
use crate::protocol::input_method::InputMethodState;
use crate::widget::keyboard::Keyboard;
use crate::widget::suggestion_bar::BarLabel;
use config::{CompletionConfig, Policy};

/// How long the caret must rest before the suggestions follow it.
pub(crate) const CARET_SETTLE: Duration = Duration::from_millis(150);

/// What a source is asked about: the word around the cursor.
#[derive(Debug, Clone, Copy)]
pub(crate) struct WordQuery<'a> {
    /// The part of the word before the cursor, as typed.
    pub(crate) word: &'a str,
    /// The part of the word after the cursor (a caret moved into a word).
    pub(crate) word_after: &'a str,
    /// How many suggestions the bar shows.
    pub(crate) max: usize,
}

/// Why a word is suggested.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// It starts with the typed word.
    Completion,
    /// The typed word looks misspelled; this is a likely intended word.
    Correction,
    /// The typed word is almost certainly misspelled and this is almost
    /// certainly the intended word: it replaces the typed one by itself when
    /// the word ends, unless another source takes the typed word as spelled
    /// right (see [`Completer::accepts`]). At most one per word.
    Autocorrection,
    /// A word list's own fix for an entry it marks as no word ("im" →
    /// "I'm"): applied like an [`Autocorrection`](Kind::Autocorrection),
    /// whatever other sources say.
    Replacement,
    /// An emoji whose name starts with the `:name` typed
    /// ([`emoji::shortcode_query`]); it replaces the `:` and the name.
    Emoji,
}

impl Kind {
    /// Whether the suggestion replaces the typed word by itself as the
    /// word ends.
    pub(crate) fn applies_itself(self) -> bool {
        matches!(self, Kind::Autocorrection | Kind::Replacement)
    }
}

/// A word a source suggests, in the source's own casing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) text: String,
    pub(crate) kind: Kind,
}

/// A source of word suggestions. Implement this to plug in a new engine.
pub(crate) trait Completer {
    /// Candidates for the word in `q`, best first. May return more than
    /// `q.max`, and words equal to the typed one; the caller drops those.
    fn suggest(&mut self, q: &WordQuery) -> Vec<Candidate>;

    /// Whether this source takes `word` (in its typed casing) as spelled
    /// right, which vetoes another source's [`Kind::Autocorrection`] of it
    /// (not a [`Kind::Replacement`]).
    /// A source that cannot tell says `false`.
    fn accepts(&mut self, _word: &str) -> bool {
        false
    }

    /// The user finished typing `word` (or accepted it): learn from it.
    /// Only called in fields that allow learning.
    fn learn(&mut self, _word: &str) {}
}

/// Completion state. Lives on the app as a `Resource`.
pub(crate) struct Completion {
    cfg: CompletionConfig,
    /// `None` while completion is off.
    completer: Option<Box<dyn Completer>>,
    /// The emoji `:name` finds; `None` while that is off.
    emoji: Option<Loading<EmojiList>>,
    /// What the bar shows now, one per slot left to right (`None` an empty
    /// slot); empty while there are no suggestions.
    pub(crate) current: Vec<Option<Suggestion>>,
    /// The inputs [`refresh`] last saw, to skip unchanged ticks.
    seen: Option<Inputs>,
    /// The caret moved since `current` was computed and has not yet rested:
    /// its edits no longer match the text.
    stale: bool,
    /// How to take back the last accepted suggestion.
    undo: Option<Undo>,
    /// Words (lowercase) whose suggestion or autocorrection was undone:
    /// the user meant them, so they are not autocorrected again.
    kept: HashSet<String>,
}

/// An accepted suggestion, remembered so a backspace right after can take it
/// back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Undo {
    /// The word as it was typed (both sides of the caret).
    pub(crate) word: String,
    /// What the undo puts back: the word, then whatever was typed along
    /// with the edit (the separator that triggered an autocorrection).
    pub(crate) original: String,
    /// What the edit typed in place of `original`, ending at the caret.
    pub(crate) inserted: String,
    /// The typing context's local revision right after: anything this
    /// keyboard does to the text first makes the undo stale. The client's
    /// reports do not (an app may report the edit in steps).
    local_rev: u64,
    at: Instant,
}

impl Undo {
    /// The edit that puts the original word back, the caret after it.
    pub(crate) fn edit(&self) -> TextEdit {
        TextEdit {
            delete_before: self.inserted.len() as u32,
            delete_after: 0,
            insert: self.original.clone(),
        }
    }
}
impl Resource for Completion {}

/// Everything the suggestions depend on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Inputs {
    rev: u64,
    field: Field,
    active: bool,
}

impl Completion {
    pub(crate) fn new(cfg: CompletionConfig, dicts: &mut Dictionaries) -> Completion {
        let completer = cfg.enabled.then(|| config::build(&cfg.backend, dicts));
        let emoji = cfg
            .emoji
            .as_deref()
            .filter(|_| cfg.enabled)
            .map(|spec| dicts.emoji(spec));
        Completion {
            cfg,
            completer,
            emoji,
            current: Vec::new(),
            seen: None,
            stale: false,
            undo: None,
            kept: HashSet::new(),
        }
    }

    /// A suggestion or autocorrection replaced `word`, and the `typed_after`
    /// it was typed with, by `inserted`, leaving the typing context at local
    /// revision `local_rev`: remember how to undo it.
    pub(crate) fn remember_undo(
        &mut self,
        word: String,
        typed_after: &str,
        inserted: String,
        local_rev: u64,
    ) {
        self.undo = Some(Undo {
            original: format!("{word}{typed_after}"),
            word,
            inserted,
            local_rev,
            at: Instant::now(),
        });
    }

    /// The undo of the last accepted suggestion, if it is still good at
    /// `now` for the typing context `ctx`: within [`config::UNDO_WINDOW`],
    /// nothing typed or pressed here since, and the inserted text still
    /// right before the caret (the caret was not moved away). It can be
    /// taken once; the word it restores is never autocorrected again.
    pub(crate) fn take_undo(&mut self, ctx: &TypingContext, now: Instant) -> Option<Undo> {
        let undo = self.undo.take()?;
        let fresh = now.saturating_duration_since(undo.at) <= config::UNDO_WINDOW;
        let untouched = undo.local_rev == ctx.local_rev();
        let in_place = ctx.before().ends_with(&undo.inserted);
        if !(fresh && untouched && in_place) {
            return None;
        }
        self.kept.insert(undo.word.to_lowercase());
        Some(undo)
    }

    /// The edit to make as `text` is typed, if it autocorrects the word
    /// before the cursor: replacing that word with its correction, followed
    /// by `text`. Only when `text` ends a word ([`config::ends_word`]), the
    /// field autocorrects, the word was just typed here (not reached by
    /// moving the caret) and does not go on after the cursor, is not all
    /// capitals (likely an acronym), not one the user kept by undoing its
    /// correction, and a source is sure of the correction
    /// ([`Kind::applies_itself`]).
    pub(crate) fn autocorrection(
        &mut self,
        ctx: &TypingContext,
        st: &InputMethodState,
        text: &str,
    ) -> Option<TextEdit> {
        let (field, active) = field_state(st);
        self.autocorrection_in(ctx, field, active, text)
    }

    /// [`autocorrection`](Self::autocorrection) in `field`, with the input
    /// method `active`.
    fn autocorrection_in(
        &mut self,
        ctx: &TypingContext,
        field: Field,
        active: bool,
        text: &str,
    ) -> Option<TextEdit> {
        let policy = self.policy(field, active)?;
        if !policy.autocorrect
            || !config::ends_word(text)
            || ctx.last_change() != Change::Typed
            || !ctx.word_after().is_empty()
        {
            return None;
        }
        // The name of an emoji being looked up is no misspelling.
        if policy.emoji && emoji::shortcode_query(ctx.before()).is_some() {
            return None;
        }
        let word = ctx.prefix();
        if word.is_empty() || edit::all_caps(&word) || self.kept.contains(&word.to_lowercase()) {
            return None;
        }
        let policy = Policy {
            append_space: false,
            ..policy
        };
        let fix = self
            .suggest(ctx, policy)
            .into_iter()
            .find(|s| s.kind.applies_itself())?;
        Some(TextEdit {
            insert: format!("{}{text}", fix.edit.insert),
            ..fix.edit
        })
    }

    /// The suggestion at `index`, if the bar shows one that still matches
    /// the text (not while the caret is settling).
    pub(crate) fn get(&self, index: usize) -> Option<&Suggestion> {
        if self.stale {
            return None;
        }
        self.current.get(index)?.as_ref()
    }

    /// Whether the keyboard has a suggestion bar at all.
    pub(crate) fn enabled(&self) -> bool {
        self.completer.is_some()
    }

    /// The policy for `field` with the input method `active`: `None` when
    /// it gets no suggestions (and so no bar).
    fn policy(&self, field: Field, active: bool) -> Option<Policy> {
        (self.enabled() && active)
            .then(|| config::policy(field, &self.cfg))
            .flatten()
    }

    /// Whether the emoji search bar can work: completion is on (so the
    /// keyboard has a bar) with an emoji list. True while a list from a
    /// path is still loading; it finds nothing until then.
    pub(crate) fn has_emoji_search(&self) -> bool {
        self.enabled() && self.emoji.is_some()
    }

    /// The emoji search bar's results for `query`, best first, at most
    /// `max` (see [`EmojiList::search_words`]); with nothing typed yet,
    /// the list's first emoji. Empty without a (loaded) emoji list.
    pub(crate) fn emoji_search(&self, query: &str, max: usize) -> Vec<String> {
        let Some(list) = self.emoji.as_ref().and_then(Loading::get) else {
            return Vec::new();
        };
        let found = if query.trim().is_empty() {
            list.popular(max)
        } else {
            list.search_words(query, max)
        };
        found.into_iter().map(str::to_owned).collect()
    }

    /// Hand a finished word to the source.
    pub(crate) fn learn(&mut self, word: &str) {
        if let Some(c) = &mut self.completer {
            c.learn(word);
        }
    }

    /// Ask the source about the word in `ctx`, or, while a `:name` is
    /// being typed, the emoji it names (the words if none match).
    fn suggest(&mut self, ctx: &TypingContext, policy: Policy) -> Vec<Suggestion> {
        if let Some(emoji) = self.emoji_suggestions(ctx, policy)
            && !emoji.is_empty()
        {
            return emoji;
        }
        let Some(completer) = &mut self.completer else {
            return Vec::new();
        };
        let word = ctx.prefix();
        if word.is_empty() {
            return Vec::new();
        }
        let q = WordQuery {
            word: &word,
            word_after: ctx.word_after(),
            max: self.cfg.max,
        };
        let candidates = completer.suggest(&q);
        edit::suggestions(&q, candidates, policy.append_space)
    }

    /// The emoji for the `:name` right before the cursor, if one is being
    /// typed (and not with the caret moved into a word) and `policy` allows
    /// emoji (none while a list from a path is still loading).
    fn emoji_suggestions(&self, ctx: &TypingContext, policy: Policy) -> Option<Vec<Suggestion>> {
        let list = self.emoji.as_ref().filter(|_| policy.emoji)?.get()?;
        if !ctx.word_after().is_empty() {
            return None;
        }
        let query = emoji::shortcode_query(ctx.before())?;
        Some(emoji::suggestions(list, query, self.cfg.max))
    }
}

/// The keyboard's word to its bar: whether it shows, and what it says.
#[derive(Debug, Clone)]
pub(crate) struct SuggestionsChanged {
    /// Whether the bar takes space at all.
    pub(crate) shown: bool,
    /// Each slot's label, left to right (`None` an empty slot); empty
    /// leaves the bar blank.
    pub(crate) labels: Vec<Option<BarLabel>>,
    /// Whether emoji can be typed: the input method is active. The virtual
    /// keyboard's fallback keymap has no emoji.
    pub(crate) emoji: bool,
}
impl Event for SuggestionsChanged {}

/// The keyboard's cue that the caret moved: (re)start the wait for it to
/// rest, then call [`settle`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct CaretMoved;
impl Event for CaretMoved {}

/// The focused field and whether the input method is active.
fn field_state(st: &InputMethodState) -> (Field, bool) {
    let field = Field::from_purpose(st.current.purpose, st.current.hint);
    (field, st.should_commit())
}

/// Recompute the suggestions now, for the current word and field.
fn recompute(
    completion: &mut Completion,
    ctx: &TypingContext,
    field: Field,
    active: bool,
) -> SuggestionsChanged {
    completion.stale = false;
    let policy = completion.policy(field, active);
    // The best word goes in the middle, but words still suggested keep their
    // slots, so the bar does not reshuffle under a finger about to tap.
    let next = match policy {
        Some(p) => completion.suggest(ctx, p),
        None => Vec::new(),
    };
    completion.current = edit::stabilize(&completion.current, next, completion.cfg.max);
    SuggestionsChanged {
        shown: policy.is_some(),
        emoji: active,
        labels: completion
            .current
            .iter()
            .map(|s| s.as_ref().map(Suggestion::bar_label))
            .collect(),
    }
}

/// What [`refresh`] does about a change.
#[derive(Debug, PartialEq, Eq)]
enum Step {
    /// Nothing changed.
    Skip,
    /// Recompute now.
    Now,
    /// The caret moved within the same field: wait for it to rest.
    Settle,
}

/// The step for inputs going from `before` to `now`, the word's last change
/// being `last`.
fn step(before: Option<Inputs>, now: Inputs, last: Change) -> Step {
    match before {
        Some(b) if b == now => Step::Skip,
        Some(b) if b.field == now.field && b.active == now.active && last == Change::Moved => {
            Step::Settle
        }
        _ => Step::Now,
    }
}

/// System: after each tick, if the word, the focused field or the input
/// method's state changed, recompute the suggestions and tell the keyboard,
/// or, for a caret move, have the keyboard wait for the caret to rest.
pub(crate) fn refresh(app: &mut App, _: &PostTick) {
    if !app.has_resource::<Completion>() || !app.has_resource::<InputMethodState>() {
        return;
    }
    let (field, active) = field_state(app.resource::<InputMethodState>());
    let targets: Vec<NodeId> = app.widgets::<Keyboard>().map(|(id, _)| id).collect();
    let event = {
        let (mut completion, ctx) = app.query::<(ResMut<Completion>, Res<TypingContext>)>();
        let inputs = Inputs {
            rev: ctx.rev(),
            field,
            active,
        };
        let step = step(completion.seen, inputs, ctx.last_change());
        completion.seen = Some(inputs);
        match step {
            Step::Skip => return,
            // Without suggestions there is nothing to go stale or flicker.
            Step::Settle if completion.policy(field, active).is_none() => return,
            Step::Settle => {
                completion.stale = true;
                None
            }
            Step::Now => Some(recompute(&mut completion, ctx, field, active)),
        }
    };
    match event {
        Some(e) => app.emit(e, targets),
        None => app.emit(CaretMoved, targets),
    }
}

/// The caret has rested: recompute the suggestions if a move left them
/// stale. `None` if they are current.
pub(crate) fn settle(
    completion: &mut Completion,
    ctx: &TypingContext,
    st: &InputMethodState,
) -> Option<SuggestionsChanged> {
    if !completion.stale {
        return None;
    }
    let (field, active) = field_state(st);
    Some(recompute(completion, ctx, field, active))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Family;

    /// Completion as configured, its word list loaded.
    fn completion() -> Completion {
        completion_with(CompletionConfig::hardcoded())
    }

    fn completion_with(cfg: CompletionConfig) -> Completion {
        let mut dicts = Dictionaries::default();
        dicts.wordlist("en_US").wait();
        Completion::new(cfg, &mut dicts)
    }

    fn text() -> Field {
        Field {
            family: Family::Text,
            sensitive: false,
        }
    }

    fn typed(word: &str) -> TypingContext {
        let mut ctx = TypingContext::default();
        ctx.committed(word);
        ctx
    }

    #[test]
    fn suggests_completions_for_the_typed_word() {
        let mut c = completion();
        let policy = c.policy(text(), true).unwrap();
        let s = c.suggest(&typed("documen"), policy);
        assert!(!s.is_empty());
        assert!(s.iter().all(|s| s.label.starts_with("documen")), "{s:?}");
        assert!(s.len() <= 3);
    }

    #[test]
    fn nothing_for_an_empty_word() {
        let mut c = completion();
        let policy = c.policy(text(), true).unwrap();
        assert!(c.suggest(&typed("hello "), policy).is_empty());
    }

    #[test]
    fn no_bar_without_an_active_input_method() {
        let c = completion();
        assert!(c.policy(text(), false).is_none());
        assert!(c.policy(text(), true).is_some());
    }

    fn inputs(rev: u64, family: Family) -> Inputs {
        Inputs {
            rev,
            field: Field {
                family,
                sensitive: false,
            },
            active: true,
        }
    }

    #[test]
    fn typing_refreshes_now_and_caret_moves_settle() {
        let a = inputs(1, Family::Text);
        assert_eq!(step(Some(a), a, Change::Moved), Step::Skip);
        assert_eq!(
            step(Some(a), inputs(2, Family::Text), Change::Typed),
            Step::Now
        );
        assert_eq!(
            step(Some(a), inputs(2, Family::Text), Change::Moved),
            Step::Settle
        );
        // A new field, or the first look, shows (or hides) the bar at once.
        assert_eq!(
            step(Some(a), inputs(2, Family::Email), Change::Moved),
            Step::Now
        );
        assert_eq!(step(None, a, Change::Moved), Step::Now);
    }

    #[test]
    fn stale_suggestions_cannot_be_taken_until_settled() {
        let mut c = completion();
        let ctx = typed("documen");
        recompute(&mut c, &ctx, text(), true);
        assert!(c.get(0).is_some());
        c.stale = true;
        assert!(c.get(0).is_none());
        // The default state has no active input method: settling recomputes
        // for that, hiding the bar.
        let st = InputMethodState::default();
        let e = settle(&mut c, &ctx, &st).unwrap();
        assert!(!e.shown);
        assert!(c.get(0).is_none());
        assert!(settle(&mut c, &ctx, &st).is_none());
    }

    /// A context where "teh" was just replaced by `inserted`, and `c`
    /// remembering that.
    fn accepted(c: &mut Completion, inserted: &str) -> TypingContext {
        let mut ctx = TypingContext::default();
        ctx.set_surrounding("say teh", 7);
        ctx.replaced_word(inserted);
        c.remember_undo("teh".into(), "", inserted.into(), ctx.local_rev());
        ctx
    }

    #[test]
    fn undo_restores_the_typed_word_once() {
        let mut c = completion();
        let ctx = accepted(&mut c, "the");
        let now = Instant::now();
        let undo = c.take_undo(&ctx, now).unwrap();
        assert_eq!(
            undo.edit(),
            TextEdit {
                delete_before: 3,
                delete_after: 0,
                insert: "teh".into()
            }
        );
        assert!(c.take_undo(&ctx, now).is_none());
    }

    #[test]
    fn undo_survives_the_app_reporting_the_edit_in_steps() {
        let mut c = completion();
        let mut ctx = accepted(&mut c, "the ");
        // The word deleted, then the replacement inserted, as GTK reports.
        ctx.set_surrounding("say ", 4);
        ctx.set_surrounding("say the ", 8);
        assert!(c.take_undo(&ctx, Instant::now()).is_some());
    }

    #[test]
    fn undo_is_gone_after_typing_a_caret_move_or_the_window() {
        let mut c = completion();
        let mut ctx = accepted(&mut c, "the");
        ctx.committed("y");
        assert!(c.take_undo(&ctx, Instant::now()).is_none());
        // A stale one is dropped, not kept for later.
        ctx.backspace();
        assert!(c.take_undo(&ctx, Instant::now()).is_none());

        let mut ctx = accepted(&mut c, "the");
        ctx.set_surrounding("say the word", 12);
        assert!(c.take_undo(&ctx, Instant::now()).is_none(), "caret moved");

        let ctx = accepted(&mut c, "the");
        let late = Instant::now() + config::UNDO_WINDOW + Duration::from_millis(1);
        assert!(c.take_undo(&ctx, late).is_none());
    }

    /// The edit typing `typed` makes after `ctx` in a text field.
    fn autocorrect(c: &mut Completion, ctx: &TypingContext, typed: &str) -> Option<TextEdit> {
        c.autocorrection_in(ctx, text(), true, typed)
    }

    #[test]
    fn a_space_or_punctuation_autocorrects_a_sure_misspelling() {
        let mut c = completion();
        for (sep, insert) in [(" ", "the "), (".", "the."), (", ", "the, ")] {
            assert_eq!(
                autocorrect(&mut c, &typed("teh"), sep),
                Some(TextEdit {
                    delete_before: 3,
                    delete_after: 0,
                    insert: insert.into()
                }),
                "{sep:?}"
            );
        }
        // Cased like the typed word.
        let fix = autocorrect(&mut c, &typed("Teh"), " ").unwrap();
        assert_eq!(fix.insert, "The ");
        let fix = autocorrect(&mut c, &typed("say recieve"), "!").unwrap();
        assert_eq!((fix.delete_before, fix.insert.as_str()), (7, "receive!"));
    }

    #[test]
    fn what_is_not_autocorrected() {
        let mut c = completion();
        // Not a word end, a known word, an unfinished one, too short to be
        // sure, all capitals.
        assert!(autocorrect(&mut c, &typed("teh"), "'").is_none());
        assert!(autocorrect(&mut c, &typed("teh"), "-").is_none());
        assert!(autocorrect(&mut c, &typed("the"), " ").is_none());
        assert!(autocorrect(&mut c, &typed("documen"), " ").is_none());
        assert!(autocorrect(&mut c, &typed("ot"), " ").is_none());
        assert!(autocorrect(&mut c, &typed("TEH"), " ").is_none());
        // A word reached by moving the caret, or one going on after it.
        let mut ctx = TypingContext::default();
        ctx.set_surrounding("say teh", 7);
        assert!(autocorrect(&mut c, &ctx, " ").is_none());
        ctx.set_surrounding("say tehx", 7);
        ctx.committed("");
        assert!(autocorrect(&mut c, &ctx, " ").is_none());
    }

    #[test]
    fn autocorrection_follows_the_field() {
        let mut c = completion();
        let ctx = typed("teh");
        let email = Field {
            family: Family::Email,
            sensitive: false,
        };
        assert!(c.autocorrection_in(&ctx, email, true, " ").is_none());
        assert!(c.autocorrection_in(&ctx, text(), false, " ").is_none());
        let cfg = CompletionConfig {
            autocorrect: false,
            ..CompletionConfig::hardcoded()
        };
        let mut off = completion_with(cfg);
        assert!(autocorrect(&mut off, &ctx, " ").is_none());
    }

    #[test]
    fn undoing_an_autocorrection_restores_the_word_and_keeps_it() {
        let mut c = completion();
        let mut ctx = typed("say teh");
        let fix = autocorrect(&mut c, &ctx, " ").unwrap();
        ctx.replaced_word(&fix.insert);
        c.remember_undo("teh".into(), " ", fix.insert.clone(), ctx.local_rev());
        assert_eq!(ctx.before(), "say the ");

        let undo = c.take_undo(&ctx, Instant::now()).unwrap();
        assert_eq!(
            undo.edit(),
            TextEdit {
                delete_before: 4,
                delete_after: 0,
                insert: "teh ".into()
            }
        );
        ctx.replaced_before(&undo.inserted, &undo.original);
        assert_eq!(ctx.before(), "say teh ");
        // Typed again, the word the user kept stays as it is.
        let ctx = typed("teh");
        assert!(autocorrect(&mut c, &ctx, " ").is_none());
    }

    #[test]
    fn undoing_a_tapped_suggestion_keeps_the_word_too() {
        let mut c = completion();
        let ctx = accepted(&mut c, "the");
        assert!(c.take_undo(&ctx, Instant::now()).is_some());
        assert!(autocorrect(&mut c, &typed("teh"), " ").is_none());
    }

    fn labels(s: &[Suggestion]) -> Vec<&str> {
        s.iter().map(|s| s.label.as_str()).collect()
    }

    #[test]
    fn a_colon_and_a_name_suggest_emoji() {
        let mut c = completion();
        let policy = c.policy(text(), true).unwrap();
        let s = c.suggest(&typed("hi :smil"), policy);
        assert_eq!(labels(&s)[..2], ["😄", "😃"]);
        assert!(s.iter().all(|s| s.kind == Kind::Emoji));
        assert_eq!(s[0].edit.delete_before, 5);
        // A name that is no word at all.
        assert_eq!(labels(&c.suggest(&typed(":+1"), policy))[0], "👍");
    }

    #[test]
    fn words_when_no_emoji_matches_or_no_colon() {
        let mut c = completion();
        let policy = c.policy(text(), true).unwrap();
        let s = c.suggest(&typed(":zqxw"), policy);
        assert!(s.iter().all(|s| s.kind != Kind::Emoji), "{s:?}");
        let s = c.suggest(&typed("smil"), policy);
        assert!(s.iter().all(|s| s.kind != Kind::Emoji), "{s:?}");
        // A time is not a name.
        let s = c.suggest(&typed("at 10:30"), policy);
        assert!(s.is_empty(), "{s:?}");
    }

    #[test]
    fn no_emoji_in_addresses_or_when_off() {
        let url = Field {
            family: Family::Url,
            sensitive: false,
        };
        let mut c = completion();
        let policy = c.policy(url, true).unwrap();
        let s = c.suggest(&typed("host:smil"), policy);
        assert!(s.iter().all(|s| s.kind != Kind::Emoji), "{s:?}");
        let s = c.suggest(&typed(":smil"), policy);
        assert!(s.iter().all(|s| s.kind != Kind::Emoji), "{s:?}");

        let mut off = completion_with(CompletionConfig {
            emoji: None,
            ..CompletionConfig::hardcoded()
        });
        let policy = off.policy(text(), true).unwrap();
        let s = off.suggest(&typed(":smil"), policy);
        assert!(s.iter().all(|s| s.kind != Kind::Emoji), "{s:?}");
    }

    #[test]
    fn an_emoji_name_is_not_autocorrected() {
        let mut c = completion();
        assert!(autocorrect(&mut c, &typed(":smil"), " ").is_none());
        assert!(autocorrect(&mut c, &typed(":teh"), ":").is_none());
        // Without the colon it is a word like any other.
        assert!(autocorrect(&mut c, &typed("teh"), " ").is_some());
    }

    #[test]
    fn undoing_an_emoji_brings_back_the_name() {
        let mut c = completion();
        let mut ctx = typed("hi :smil");
        let s = c.suggest(&ctx, c.policy(text(), true).unwrap());
        let edit = &s[0].edit;
        // As `input::replace_word` records the accepted suggestion.
        let deleted = ctx.text_before(edit.delete_before);
        assert_eq!(deleted, ":smil");
        ctx.replaced_before(&deleted, &edit.insert);
        c.remember_undo(deleted, "", edit.insert.clone(), ctx.local_rev());
        assert_eq!(ctx.before(), "hi 😄");

        let undo = c.take_undo(&ctx, Instant::now()).unwrap();
        assert_eq!(
            undo.edit(),
            TextEdit {
                delete_before: "😄".len() as u32,
                delete_after: 0,
                insert: ":smil".into()
            }
        );
    }

    #[test]
    fn disabled_completion_has_no_bar() {
        let cfg = CompletionConfig {
            enabled: false,
            ..CompletionConfig::hardcoded()
        };
        let c = Completion::new(cfg, &mut Dictionaries::default());
        assert!(!c.enabled());
        assert!(c.policy(text(), true).is_none());
    }
}
