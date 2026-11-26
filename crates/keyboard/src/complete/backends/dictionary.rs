//! Completion from a frequency word list: the most frequent words starting
//! with the typed word and, when the typed word is not in the list, the most
//! frequent words one edit away from it.

use std::sync::Arc;

use crate::complete::{Candidate, Completer, Kind, WordQuery};
use crate::dictionaries::{Dictionary, is_normalised};

/// A correction is offered next to completions only when it is this many
/// times more frequent than the best completion: "teh" is far likelier a
/// slip for "the" than the start of "teheran", but "th" is just unfinished.
const CORRECTION_DOMINANCE: u64 = 20;

/// Letters an edit may insert or substitute.
const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz'";

/// Suggests from a [`Dictionary`].
pub(crate) struct DictionaryCompleter {
    dict: Arc<Dictionary>,
}

impl DictionaryCompleter {
    pub(crate) fn new(dict: Arc<Dictionary>) -> Self {
        Self { dict }
    }

    /// Up to `n` words that start with `prefix` and end with `suffix`,
    /// longer than both together, most frequent first.
    fn completions(&self, prefix: &str, suffix: &str, n: usize) -> Vec<(&str, u32)> {
        let mut found: Vec<(&str, u32)> = self
            .dict
            .prefixed(prefix)
            .iter()
            .filter(|(w, _)| w.len() > prefix.len() + suffix.len() && w.ends_with(suffix))
            .map(|(w, c)| (&**w, *c))
            .collect();
        top(&mut found, n)
    }

    /// Up to `n` listed words one deletion, transposition, substitution or
    /// insertion away from `word`, most frequent first.
    fn corrections(&self, word: &str, n: usize) -> Vec<(String, u32)> {
        let mut found: Vec<(String, u32)> = Vec::new();
        for edit in edits1(word) {
            if found.iter().any(|(w, _)| *w == edit) {
                continue;
            }
            if let Some(c) = self.dict.count(&edit) {
                found.push((edit, c));
            }
        }
        found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        found.truncate(n);
        found
    }
}

/// The `n` most frequent of `found`, ties by word.
fn top<'a>(found: &mut [(&'a str, u32)], n: usize) -> Vec<(&'a str, u32)> {
    found.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
    found.iter().take(n).copied().collect()
}

/// Every string one edit away from the ASCII `word`.
fn edits1(word: &str) -> Vec<String> {
    let b = word.as_bytes();
    let mut out = Vec::with_capacity(b.len() * 55 + 27);
    let make = |parts: &[&[u8]]| String::from_utf8(parts.concat()).ok();
    for i in 0..=b.len() {
        let (l, r) = b.split_at(i);
        if !r.is_empty() {
            out.extend(make(&[l, &r[1..]]));
        }
        if r.len() >= 2 {
            out.extend(make(&[l, &[r[1], r[0]], &r[2..]]));
        }
        for &c in ALPHABET {
            if let Some(&first) = r.first()
                && first != c
            {
                out.extend(make(&[l, &[c], &r[1..]]));
            }
            out.extend(make(&[l, &[c], r]));
        }
    }
    out
}

impl Completer for DictionaryCompleter {
    fn suggest(&mut self, q: &WordQuery) -> Vec<Candidate> {
        // The list is lowercase ASCII; whole-word matching needs both halves.
        let word = format!("{}{}", q.word, q.word_after).to_lowercase();
        if word.is_empty() || !is_normalised(&word) {
            return Vec::new();
        }
        // Completions extend what is before the cursor.
        let (prefix, suffix) = (q.word.to_lowercase(), q.word_after.to_lowercase());
        let completions = self.completions(&prefix, &suffix, q.max + 1);
        // A word in the list is spelled right; only an unknown one is
        // corrected, and only once it is long enough to guess at. Next to
        // completions, only a far more frequent correction is offered.
        let mut corrections = Vec::new();
        if word.len() >= 2 && self.dict.count(&word).is_none() {
            let best = completions.first().map_or(0, |(_, c)| u64::from(*c));
            corrections = self.corrections(&word, q.max);
            corrections.retain(|(_, c)| best == 0 || u64::from(*c) > best * CORRECTION_DOMINANCE);
        }
        // Corrections first: the typed word looks wrong.
        let fixes = corrections.into_iter().map(|(w, _)| Candidate {
            text: w,
            kind: Kind::Correction,
        });
        let completes = completions.into_iter().map(|(w, _)| Candidate {
            text: w.to_string(),
            kind: Kind::Completion,
        });
        fixes.chain(completes).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TINY: &str = "the\t100\nthat\t50\nthis\t30\nthey\t40\nto\t80\nten\t20\ntea\t5\n\
                        document\t9\ndocuments\t8\ndocumentation\t3\ntehran\t2\n";

    fn completer() -> DictionaryCompleter {
        DictionaryCompleter::new(Arc::new(Dictionary::parse(TINY)))
    }

    fn suggest(word: &str, after: &str) -> Vec<(String, Kind)> {
        let q = WordQuery {
            word,
            word_after: after,
            max: 3,
        };
        completer()
            .suggest(&q)
            .into_iter()
            .map(|c| (c.text, c.kind))
            .collect()
    }

    fn texts(s: &[(String, Kind)]) -> Vec<&str> {
        s.iter().map(|(t, _)| t.as_str()).collect()
    }

    #[test]
    fn completes_by_frequency() {
        let s = suggest("th", "");
        assert_eq!(texts(&s)[..3], ["the", "that", "they"]);
        assert!(s.iter().all(|(_, k)| *k == Kind::Completion));
    }

    #[test]
    fn a_known_word_is_completed_not_corrected() {
        let s = suggest("document", "");
        assert_eq!(texts(&s), ["documents", "documentation"]);
    }

    #[test]
    fn misspelling_gets_corrections_first() {
        // "tehran" starts with "teh", but "the" is far more frequent.
        let s = suggest("teh", "");
        assert_eq!(s[0], ("the".to_string(), Kind::Correction));
        assert!(texts(&s).contains(&"tehran"));
        // With nothing to complete, every close word is offered.
        let s = suggest("tez", "");
        assert!(
            texts(&s).contains(&"tea") && texts(&s).contains(&"ten"),
            "{s:?}"
        );
    }

    #[test]
    fn case_is_ignored() {
        assert_eq!(texts(&suggest("TH", ""))[0], "the");
    }

    #[test]
    fn mid_word_completions_keep_the_part_after_the_cursor() {
        // "doc|ument": only longer words ending in what follows the caret.
        let s = suggest("doc", "ument");
        assert!(texts(&s).is_empty(), "{s:?}");
        let s = suggest("doc", "s");
        assert_eq!(texts(&s), ["documents"]);
    }

    #[test]
    fn non_ascii_words_get_nothing() {
        assert!(suggest("café", "").is_empty());
        assert!(suggest("", "").is_empty());
    }

    #[test]
    fn bundled_list_answers_quickly() {
        let mut c = DictionaryCompleter::new(Arc::new(Dictionary::bundled("en").unwrap()));
        let start = std::time::Instant::now();
        for w in ["a", "t", "th", "documen", "teh", "recieve", "xqzv"] {
            let q = WordQuery {
                word: w,
                word_after: "",
                max: 3,
            };
            std::hint::black_box(c.suggest(&q));
        }
        assert!(
            start.elapsed() < std::time::Duration::from_millis(200),
            "{:?}",
            start.elapsed()
        );
        let q = WordQuery {
            word: "recieve",
            word_after: "",
            max: 3,
        };
        assert_eq!(c.suggest(&q)[0].text, "receive");
    }
}
