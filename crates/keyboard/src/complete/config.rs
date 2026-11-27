//! Which completion sources run, and in which fields: the one place to
//! change either. [`CompletionConfig::hardcoded`] stands in for a config
//! system.

use std::time::Duration;

use super::Completer;
use super::backends::{DictionaryCompleter, Merge};
use crate::dictionaries::Dictionaries;
use crate::field::Field;
use crate::layout::Family;

/// A completion source, or several.
#[derive(Debug, Clone, PartialEq)]
#[allow(dead_code)] // every variant is a choice for `hardcoded`, not all are used at once
pub(crate) enum Backend {
    /// A frequency word list (bundled name or path, as for prediction):
    /// completions by frequency, and corrections one edit away.
    Dictionary(String),
    /// Hunspell spell-checking with the named dictionary (`en_US` loads
    /// `/usr/share/hunspell/en_US.{aff,dic}`): corrections of misspelled
    /// words. Needs the `hunspell` feature.
    Hunspell(String),
    /// Several sources, their suggestions interleaved in this order.
    Merge(Vec<Backend>),
}

/// How long after accepting a suggestion a backspace undoes it (restoring
/// the word as typed) instead of deleting a character. Anything else done to
/// the word first (typing, moving the caret) ends the chance.
pub(crate) const UNDO_WINDOW: Duration = Duration::from_secs(3);

/// Whether accepting a suggestion also types a space after the word (in
/// fields where that fits; never in addresses).
pub(crate) const APPEND_SPACE: bool = false;

/// Whether a word that is almost certainly misspelled is corrected by itself
/// when a space or punctuation ends it (in text fields; never in addresses).
pub(crate) const AUTOCORRECT: bool = true;

/// Words shorter than this (in chars) are never autocorrected: too short to
/// be sure of.
pub(crate) const AUTOCORRECT_MIN_LEN: usize = 3;

/// Punctuation that ends a word and so autocorrects it, as whitespace does.
/// Not `'` (inside words), nor `-`, `/`, `@`, `_` and the like, which join
/// parts of one token.
const WORD_ENDERS: &[char] = &['.', ',', '!', '?', ';', ':', ')', ']', '}', '"'];

/// Whether typing `text` right after a word ends that word, autocorrecting
/// it: it starts with whitespace or with sentence punctuation.
pub(crate) fn ends_word(text: &str) -> bool {
    text.chars()
        .next()
        .is_some_and(|c| c.is_whitespace() || WORD_ENDERS.contains(&c))
}

/// Completion settings.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CompletionConfig {
    /// `false` = no suggestion bar anywhere.
    pub(crate) enabled: bool,
    pub(crate) backend: Backend,
    /// Most suggestions shown at once.
    pub(crate) max: usize,
    /// Whether accepting a suggestion types a space after it; see
    /// [`APPEND_SPACE`].
    pub(crate) append_space: bool,
    /// Whether misspelled words are corrected as they end; see
    /// [`AUTOCORRECT`].
    pub(crate) autocorrect: bool,
}

impl CompletionConfig {
    /// The completion settings in use. Replace with the config system.
    pub(crate) fn hardcoded() -> Self {
        let words = Backend::Dictionary("en".into());
        let backend = if cfg!(feature = "hunspell") {
            Backend::Merge(vec![words, Backend::Hunspell("en_US".into())])
        } else {
            words
        };
        Self {
            enabled: true,
            backend,
            max: 3,
            append_space: APPEND_SPACE,
            autocorrect: AUTOCORRECT,
        }
    }
}

/// How suggestions behave in a field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Policy {
    /// Whether accepting a suggestion types a space after it.
    pub(crate) append_space: bool,
    /// Whether a misspelled word is corrected by itself as it ends.
    pub(crate) autocorrect: bool,
}

/// The policy for `field` under `cfg`: a space after accepted words and
/// autocorrection where they fit, if `cfg` asks for them. `None` means no
/// suggestions, and no bar. Sensitive fields never get any; nor do
/// terminals, numbers and phones.
pub(crate) fn policy(field: Field, cfg: &CompletionConfig) -> Option<Policy> {
    if field.sensitive {
        return None;
    }
    match field.family {
        Family::Text => Some(Policy {
            append_space: cfg.append_space,
            autocorrect: cfg.autocorrect,
        }),
        // An address goes on after a completed part (`.`, `@`, `/`), and
        // its parts are names, not dictionary words.
        Family::Email | Family::Url => Some(Policy {
            append_space: false,
            autocorrect: false,
        }),
        _ => None,
    }
}

/// The source `backend` describes. A source that cannot be built (its
/// feature is off, its files are missing) is warned about and left out.
pub(crate) fn build(backend: &Backend, dicts: &mut Dictionaries) -> Box<dyn Completer> {
    match backend {
        Backend::Dictionary(spec) => Box::new(DictionaryCompleter::new(dicts.get(spec))),
        Backend::Hunspell(lang) => hunspell(lang),
        Backend::Merge(all) => Box::new(Merge::new(all.iter().map(|b| build(b, dicts)).collect())),
    }
}

#[cfg(feature = "hunspell")]
fn hunspell(lang: &str) -> Box<dyn Completer> {
    match super::backends::HunspellCompleter::load(lang) {
        Some(h) => Box::new(h),
        None => Box::new(Merge::new(Vec::new())),
    }
}

#[cfg(not(feature = "hunspell"))]
fn hunspell(lang: &str) -> Box<dyn Completer> {
    tracing::warn!(
        lang,
        "hunspell completion needs the `hunspell` feature; left out"
    );
    Box::new(Merge::new(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(family: Family, sensitive: bool) -> Field {
        Field { family, sensitive }
    }

    fn cfg(append_space: bool, autocorrect: bool) -> CompletionConfig {
        CompletionConfig {
            append_space,
            autocorrect,
            ..CompletionConfig::hardcoded()
        }
    }

    #[test]
    fn no_space_by_default() {
        assert!(!CompletionConfig::hardcoded().append_space);
    }

    #[test]
    fn autocorrect_by_default() {
        assert!(CompletionConfig::hardcoded().autocorrect);
    }

    #[test]
    fn text_gets_a_space_and_autocorrection_only_when_configured() {
        for space in [true, false] {
            for autocorrect in [true, false] {
                assert_eq!(
                    policy(field(Family::Text, false), &cfg(space, autocorrect)),
                    Some(Policy {
                        append_space: space,
                        autocorrect,
                    })
                );
            }
        }
    }

    #[test]
    fn addresses_never_get_a_space_or_autocorrection() {
        for f in [Family::Email, Family::Url] {
            assert_eq!(
                policy(field(f, false), &cfg(true, true)),
                Some(Policy {
                    append_space: false,
                    autocorrect: false,
                })
            );
        }
    }

    #[test]
    fn spaces_and_sentence_punctuation_end_words() {
        for t in [" ", "\n", ".", ",", "!", "?", ";", ":", ")", "\"", ". "] {
            assert!(ends_word(t), "{t:?}");
        }
        for t in ["", "a", "'", "-", "/", "@", "_", "1"] {
            assert!(!ends_word(t), "{t:?}");
        }
    }

    #[test]
    fn sensitive_terminal_number_and_phone_get_none() {
        let cfg = cfg(true, true);
        assert_eq!(policy(field(Family::Text, true), &cfg), None);
        for f in [
            Family::Password,
            Family::Terminal,
            Family::Number,
            Family::Phone,
        ] {
            assert_eq!(policy(field(f, false), &cfg), None, "{f:?}");
        }
    }
}
