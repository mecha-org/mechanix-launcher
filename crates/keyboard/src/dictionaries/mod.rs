//! Word-frequency lists, shared by everything that knows about words: the
//! next-letter predictor ([`crate::predict`]) and word completion
//! ([`crate::complete`]). Each list is loaded once, by spec, through the
//! [`Dictionaries`] cache.

use std::collections::HashMap;
use std::sync::Arc;

use mecha_wayland::prelude::*;

/// A word list: lowercase `[a-z']` words with their frequency counts, sorted
/// by word.
pub(crate) struct Dictionary {
    words: Vec<(Box<str>, u32)>,
}

impl Dictionary {
    /// Parse `word<TAB>count` lines. Blank and malformed lines and words with
    /// characters outside `[a-z']` (after lowercasing) are skipped; duplicate
    /// words have their counts summed.
    pub(crate) fn parse(tsv: &str) -> Dictionary {
        let mut words: Vec<(Box<str>, u32)> = Vec::new();
        for line in tsv.lines() {
            let Some((w, c)) = line.split_once('\t') else {
                continue;
            };
            let w = w.trim().to_lowercase();
            let Ok(c) = c.trim().parse::<u32>() else {
                continue;
            };
            if w.is_empty() || !is_normalised(&w) {
                continue;
            }
            words.push((w.into_boxed_str(), c));
        }
        words.sort_by(|a, b| a.0.cmp(&b.0));
        words.dedup_by(|dup, kept| {
            if dup.0 == kept.0 {
                kept.1 = kept.1.saturating_add(dup.1);
                true
            } else {
                false
            }
        });
        Dictionary { words }
    }

    /// A word list shipped in the binary, by name.
    pub(crate) fn bundled(name: &str) -> Option<Dictionary> {
        match name {
            "en" => Some(Self::parse(include_str!("../../resources/dict/en.tsv"))),
            "terminal" => Some(Self::parse(include_str!(
                "../../resources/dict/terminal.tsv"
            ))),
            _ => None,
        }
    }

    /// Load `spec` as a bundled name, else as a file path; if neither works,
    /// warn and return an empty dictionary.
    pub(crate) fn load(spec: &str) -> Dictionary {
        if let Some(d) = Self::bundled(spec) {
            return d;
        }
        match std::fs::read_to_string(spec) {
            Ok(text) => Self::parse(&text),
            Err(e) => {
                tracing::warn!("cannot load dictionary {spec:?}: {e}; using an empty one");
                Self::parse("")
            }
        }
    }

    /// Number of distinct words.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.words.len()
    }

    /// Every word starting with `prefix`, in word order. `prefix` must
    /// already be normalised (lowercase `[a-z']`), as every stored word is.
    pub(crate) fn prefixed(&self, prefix: &str) -> &[(Box<str>, u32)] {
        let start = self.words.partition_point(|(w, _)| &**w < prefix);
        let len = self.words[start..].partition_point(|(w, _)| w.starts_with(prefix));
        &self.words[start..start + len]
    }

    /// The count of `word` (already normalised), if it is in the list.
    pub(crate) fn count(&self, word: &str) -> Option<u32> {
        self.words
            .binary_search_by(|(w, _)| (**w).cmp(word))
            .ok()
            .map(|i| self.words[i].1)
    }
}

/// Whether `s` holds only characters a stored word can: `a`-`z` and `'`.
pub(crate) fn is_normalised(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_lowercase() || b == b'\'')
}

/// Every dictionary loaded so far, by the spec it was loaded from, so a list
/// used by several features is parsed once.
#[derive(Default)]
pub(crate) struct Dictionaries {
    loaded: HashMap<String, Arc<Dictionary>>,
}
impl Resource for Dictionaries {}

impl Dictionaries {
    /// The dictionary for `spec` (a bundled name or a path), loading it on
    /// first use.
    pub(crate) fn get(&mut self, spec: &str) -> Arc<Dictionary> {
        self.loaded
            .entry(spec.to_string())
            .or_insert_with(|| Arc::new(Dictionary::load(spec)))
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TINY: &str = "the\t100\nthat\t50\nthis\t30\nto\t80\nzoo\t1\n";

    #[test]
    fn parse_skips_bad_lines() {
        let d = Dictionary::parse("x\n\tq\nok\tnotanumber\nfine\t3\nÉcole\t5");
        assert_eq!(d.len(), 1);
        assert_eq!(&*d.words[0].0, "fine");
    }

    #[test]
    fn duplicates_sum() {
        let d = Dictionary::parse("ab\t1\nAB\t2\n");
        assert_eq!((d.len(), d.words[0].1), (1, 3));
    }

    #[test]
    fn load_falls_back_to_empty_dictionary() {
        let d = Dictionary::load("/nonexistent/dictionary.tsv");
        assert_eq!(d.len(), 0);
    }

    #[test]
    fn bundled_lists_load() {
        let en = Dictionary::bundled("en").expect("en is bundled");
        assert!(en.len() >= 20_000, "only {} words", en.len());
        let terminal = Dictionary::bundled("terminal").expect("terminal is bundled");
        assert!(terminal.len() >= 150, "only {} words", terminal.len());
        assert!(Dictionary::bundled("xx").is_none());
    }

    #[test]
    fn prefixed_is_the_words_with_that_prefix() {
        let d = Dictionary::parse(TINY);
        let words: Vec<&str> = d.prefixed("th").iter().map(|(w, _)| &**w).collect();
        assert_eq!(words, ["that", "the", "this"]);
        assert_eq!(d.prefixed("").len(), 5);
        assert!(d.prefixed("q").is_empty());
        assert!(d.prefixed("zzz").is_empty());
    }

    #[test]
    fn count_finds_exact_words_only() {
        let d = Dictionary::parse(TINY);
        assert_eq!(d.count("the"), Some(100));
        assert_eq!(d.count("th"), None);
        assert_eq!(d.count("zoos"), None);
    }

    #[test]
    fn cache_loads_once() {
        let mut dicts = Dictionaries::default();
        let a = dicts.get("terminal");
        let b = dicts.get("terminal");
        assert!(Arc::ptr_eq(&a, &b));
    }
}
