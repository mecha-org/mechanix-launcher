//! Word lists, shared by everything that knows about words, each loaded
//! once, by spec, through the [`Dictionaries`] cache. Two kinds:
//!
//! - [`Dictionary`]: lowercase `[a-z']` words with counts, for the
//!   next-letter predictor ([`crate::predict`]), which needs only letters.
//! - [`Wordlist`]: words as written, with frequencies, flags and
//!   replacements (AOSP LatinIME's format), for word completion and
//!   autocorrection ([`crate::complete`]).
//! - [`EmojiList`]: emoji with their shortcodes and keywords (GitHub
//!   gemoji's format), for emoji suggestions ([`crate::complete`]).

mod emoji;
mod wordlist;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use mecha_wayland::prelude::*;

pub(crate) use emoji::EmojiList;
pub(crate) use wordlist::{Word, Wordlist};

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
    #[cfg(test)]
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

/// Every list loaded so far, by the spec it was loaded from, so a list used
/// by several features is parsed once.
#[derive(Default)]
pub(crate) struct Dictionaries {
    loaded: HashMap<String, Arc<Dictionary>>,
    wordlists: HashMap<String, Loading<Wordlist>>,
    emoji: HashMap<String, Loading<EmojiList>>,
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

    /// The word list for `spec` (a bundled locale or a path). A bundled
    /// list was compiled at build time and is ready at once; one from a path
    /// is compiled in the background on first use, which takes long enough
    /// (half a second on the device) that startup should not wait for it.
    pub(crate) fn wordlist(&mut self, spec: &str) -> Loading<Wordlist> {
        self.wordlists
            .entry(spec.to_string())
            .or_insert_with(|| match Wordlist::bundled(spec) {
                Some(list) => Loading::ready(list),
                None => {
                    let spec = spec.to_string();
                    Loading::spawn(move || Wordlist::load(&spec))
                }
            })
            .clone()
    }

    /// The emoji list for `spec` (a bundled name or a path), loaded as
    /// [`wordlist`](Self::wordlist) loads word lists: a bundled one ready at
    /// once, one from a path compiled in the background on first use.
    pub(crate) fn emoji(&mut self, spec: &str) -> Loading<EmojiList> {
        self.emoji
            .entry(spec.to_string())
            .or_insert_with(|| match EmojiList::bundled(spec) {
                Some(list) => Loading::ready(list),
                None => {
                    let spec = spec.to_string();
                    Loading::spawn(move || EmojiList::load(&spec))
                }
            })
            .clone()
    }
}

/// A value being made on a background thread, shared by every clone.
pub(crate) struct Loading<T>(Arc<OnceLock<T>>);

impl<T> Clone for Loading<T> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<T: Send + Sync + 'static> Loading<T> {
    /// Start making the value with `make`; if no thread can be started,
    /// make it now.
    pub(crate) fn spawn<F: FnOnce() -> T + Send + 'static>(make: F) -> Self {
        let cell = Arc::new(OnceLock::new());
        // Shared so the job can still run here if the thread cannot start.
        let job = Arc::new(Mutex::new(Some(make)));
        let run = {
            let cell = cell.clone();
            move |job: &Mutex<Option<F>>| {
                let make = job.lock().unwrap_or_else(|e| e.into_inner()).take();
                if let Some(make) = make {
                    let _ = cell.set(make());
                }
            }
        };
        let started = {
            let (job, run) = (job.clone(), run.clone());
            std::thread::Builder::new()
                .name("word-list".into())
                .spawn(move || run(&job))
        };
        if let Err(e) = started {
            tracing::warn!("cannot load in the background ({e}); loading now");
            run(&job);
        }
        Self(cell)
    }

    /// A value that is ready already.
    pub(crate) fn ready(value: T) -> Self {
        Self(Arc::new(OnceLock::from(value)))
    }

    /// The value, once it is made.
    pub(crate) fn get(&self) -> Option<&T> {
        self.0.get()
    }

    /// The value, waiting for it.
    #[cfg(test)]
    pub(crate) fn wait(&self) -> &T {
        loop {
            if let Some(v) = self.get() {
                return v;
            }
            std::thread::yield_now();
        }
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
    fn loading_becomes_ready() {
        let l = Loading::spawn(|| 7);
        assert_eq!(l.wait(), &7);
        assert_eq!(l.clone().get(), Some(&7));
        assert_eq!(Loading::ready(3).get(), Some(&3));
    }

    #[test]
    fn cache_loads_once() {
        let mut dicts = Dictionaries::default();
        let a = dicts.get("terminal");
        let b = dicts.get("terminal");
        assert!(Arc::ptr_eq(&a, &b));
    }
}
