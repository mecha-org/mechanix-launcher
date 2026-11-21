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

mod backends;
pub(crate) mod config;
mod edit;

use std::time::Duration;

use mecha_wayland::prelude::*;

pub(crate) use edit::{Suggestion, TextEdit};

use crate::dictionaries::Dictionaries;
use crate::field::Field;
use crate::predict::{Change, TypingContext};
use crate::protocol::input_method::InputMethodState;
use crate::widget::keyboard::Keyboard;
use config::{CompletionConfig, Policy};

/// How long the caret must rest before the suggestions follow it.
pub(crate) const CARET_SETTLE: Duration = Duration::from_millis(150);

/// What a source is asked about: the word around the cursor.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Query<'a> {
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
    fn suggest(&mut self, q: &Query) -> Vec<Candidate>;

    /// The user finished typing `word` (or accepted it): learn from it.
    /// Only called in fields that allow learning.
    fn learn(&mut self, _word: &str) {}
}

/// Completion state. Lives on the app as a `Resource`.
pub(crate) struct Completion {
    cfg: CompletionConfig,
    /// `None` while completion is off.
    completer: Option<Box<dyn Completer>>,
    /// What the bar shows now, in order.
    pub(crate) current: Vec<Suggestion>,
    /// The inputs [`refresh`] last saw, to skip unchanged ticks.
    seen: Option<Inputs>,
    /// The caret moved since `current` was computed and has not yet rested:
    /// its edits no longer match the text.
    stale: bool,
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
        Completion {
            cfg,
            completer,
            current: Vec::new(),
            seen: None,
            stale: false,
        }
    }

    /// The suggestion at `index`, if the bar shows one that still matches
    /// the text (not while the caret is settling).
    pub(crate) fn get(&self, index: usize) -> Option<&Suggestion> {
        if self.stale {
            return None;
        }
        self.current.get(index)
    }

    /// Whether the keyboard has a suggestion bar at all.
    pub(crate) fn enabled(&self) -> bool {
        self.completer.is_some()
    }

    /// The policy for `field` with the input method `active`: `None` when
    /// it gets no suggestions (and so no bar).
    fn policy(&self, field: Field, active: bool) -> Option<Policy> {
        (self.enabled() && active)
            .then(|| config::policy(field))
            .flatten()
    }

    /// Hand a finished word to the source.
    pub(crate) fn learn(&mut self, word: &str) {
        if let Some(c) = &mut self.completer {
            c.learn(word);
        }
    }

    /// Ask the source about the word in `ctx`.
    fn suggest(&mut self, ctx: &TypingContext, policy: Policy) -> Vec<Suggestion> {
        let Some(completer) = &mut self.completer else {
            return Vec::new();
        };
        let word = ctx.prefix();
        if word.is_empty() {
            return Vec::new();
        }
        let q = Query {
            word: &word,
            word_after: ctx.word_after(),
            max: self.cfg.max,
        };
        let candidates = completer.suggest(&q);
        edit::suggestions(&q, candidates, policy.append_space)
    }
}

/// The keyboard's word to its bar: whether it shows, and what it says.
#[derive(Debug, Clone)]
pub(crate) struct SuggestionsChanged {
    /// Whether the bar takes space at all.
    pub(crate) shown: bool,
    /// The suggestions' labels, in order; empty leaves the bar blank.
    pub(crate) labels: Vec<String>,
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
    // Words still suggested keep their slots, so the bar does not reshuffle
    // under a finger about to tap.
    let next = match policy {
        Some(p) => completion.suggest(ctx, p),
        None => Vec::new(),
    };
    completion.current = edit::stabilize(&completion.current, next);
    SuggestionsChanged {
        shown: policy.is_some(),
        labels: completion.current.iter().map(|s| s.label.clone()).collect(),
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

    fn completion() -> Completion {
        Completion::new(CompletionConfig::hardcoded(), &mut Dictionaries::default())
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
