//! Spell-checking with hunspell: when the typed word is misspelled,
//! hunspell's suggestions for it, as corrections. A correctly spelled word
//! gets nothing here; completions come from another source (see
//! [`Merge`](super::Merge)).

use hunspell_rs::{CheckResult, Hunspell};

use crate::complete::{Candidate, Completer, Kind, WordQuery};

/// Where system hunspell dictionaries live.
const DICT_DIR: &str = "/usr/share/hunspell";

/// Words shorter than this (in chars) are not checked: too little to correct.
const MIN_LEN: usize = 2;

/// Corrections from hunspell.
pub(crate) struct HunspellCompleter {
    speller: Hunspell,
    /// The last word asked about and its candidates: `suggest` is slow (tens
    /// of ms on a phone), and the same word is asked about again whenever
    /// the context changes without the word changing.
    last: Option<(String, Vec<Candidate>)>,
}

impl HunspellCompleter {
    /// Load `lang` (e.g. `en_US`) from the system dictionaries; `None`
    /// (warned) if they are missing.
    pub(crate) fn load(lang: &str) -> Option<Self> {
        let aff = format!("{DICT_DIR}/{lang}.aff");
        let dic = format!("{DICT_DIR}/{lang}.dic");
        if !std::path::Path::new(&aff).exists() || !std::path::Path::new(&dic).exists() {
            tracing::warn!(lang, "no hunspell dictionary at {aff} / {dic}; left out");
            return None;
        }
        tracing::info!(lang, "hunspell dictionary loaded");
        Some(Self {
            speller: Hunspell::new(&aff, &dic),
            last: None,
        })
    }
}

impl Completer for HunspellCompleter {
    fn suggest(&mut self, q: &WordQuery) -> Vec<Candidate> {
        let word = format!("{}{}", q.word, q.word_after);
        if word.chars().count() < MIN_LEN {
            return Vec::new();
        }
        if let Some((last, found)) = &self.last
            && *last == word
        {
            return found.clone();
        }
        let found = match self.speller.check(&word) {
            CheckResult::FoundInDictionary => Vec::new(),
            CheckResult::MissingInDictionary => self
                .speller
                .suggest(&word)
                .into_iter()
                .take(q.max)
                .map(|text| Candidate {
                    text,
                    kind: Kind::Correction,
                })
                .collect(),
        };
        self.last = Some((word, found.clone()));
        found
    }

    fn accepts(&mut self, word: &str) -> bool {
        matches!(self.speller.check(word), CheckResult::FoundInDictionary)
    }

    fn learn(&mut self, word: &str) {
        // Words the user typed and kept are not misspellings: stop
        // correcting them this session.
        self.speller.add(word);
        self.last = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(word: &str) -> WordQuery<'_> {
        WordQuery {
            word,
            word_after: "",
            max: 3,
        }
    }

    /// Runs only where the system has the en_US dictionary.
    fn en() -> Option<HunspellCompleter> {
        HunspellCompleter::load("en_US")
    }

    #[test]
    fn corrects_misspellings_only() {
        let Some(mut h) = en() else {
            return;
        };
        let s = h.suggest(&q("recieve"));
        assert!(s.iter().any(|c| c.text == "receive"), "{s:?}");
        assert!(s.iter().all(|c| c.kind == Kind::Correction));
        assert!(h.suggest(&q("receive")).is_empty());
        assert!(h.suggest(&q("x")).is_empty());
        assert!(h.accepts("receive") && !h.accepts("recieve"));
    }

    #[test]
    fn learned_words_are_not_corrected() {
        let Some(mut h) = en() else {
            return;
        };
        assert!(!h.suggest(&q("mechanixy")).is_empty());
        h.learn("mechanixy");
        assert!(h.suggest(&q("mechanixy")).is_empty());
    }

    #[test]
    fn missing_dictionary_is_none() {
        assert!(HunspellCompleter::load("xx_NOPE").is_none());
    }
}
