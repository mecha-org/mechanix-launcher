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
//! fills it.

mod backends;
pub(crate) mod config;
mod edit;

use mecha_wayland::prelude::*;

pub(crate) use edit::{Suggestion, TextEdit};

use crate::dictionaries::Dictionaries;
use crate::field::Field;
use crate::predict::TypingContext;
use crate::protocol::input_method::InputMethodState;
use crate::widget::keyboard::Keyboard;
use config::{CompletionConfig, Policy};

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
    /// The inputs [`refresh`] last computed for, to skip unchanged ticks.
    seen: Option<Inputs>,
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
        }
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

/// System: after each tick, recompute the suggestions if the word, the
/// focused field or the input method's state changed, and tell the keyboard.
pub(crate) fn refresh(app: &mut App, _: &PostTick) {
    if !app.has_resource::<Completion>() || !app.has_resource::<InputMethodState>() {
        return;
    }
    let (active, field) = {
        let st = app.resource::<InputMethodState>();
        (
            st.should_commit(),
            Field::from_purpose(st.current.purpose, st.current.hint),
        )
    };
    let inputs = Inputs {
        rev: app.resource::<TypingContext>().rev(),
        field,
        active,
    };
    let (shown, labels) = {
        let (mut completion, ctx) = app.query::<(ResMut<Completion>, Res<TypingContext>)>();
        if completion.seen == Some(inputs) {
            return;
        }
        completion.seen = Some(inputs);
        let policy = completion.policy(field, active);
        let current = match policy {
            Some(p) => completion.suggest(ctx, p),
            None => Vec::new(),
        };
        let labels = current.iter().map(|s| s.label.clone()).collect();
        completion.current = current;
        (policy.is_some(), labels)
    };
    let targets: Vec<NodeId> = app.widgets::<Keyboard>().map(|(id, _)| id).collect();
    app.emit(SuggestionsChanged { shown, labels }, targets);
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
