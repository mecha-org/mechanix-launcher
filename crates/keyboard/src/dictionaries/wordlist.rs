//! Word lists for word completion and autocorrection: words as written
//! (capitals, apostrophes, accents), each with a frequency, flags for words
//! never to suggest, and whitelisted replacements ("im" → "I'm"). Read from
//! AOSP LatinIME's "combined" format, compiled ([`compile`]) into a form
//! searched in place: the bundled lists at build time, so loading one costs
//! nothing, a list from a path at runtime.

mod compile;

use std::borrow::Cow;
use std::cmp::Ordering;
use std::collections::HashMap;
use std::io::Read;

use compile::{Compiled, ENTRY_LEN, REAL, SUGGEST, folded};

/// A word list, searched without regard to case.
pub(crate) struct Wordlist {
    /// Every word, back to back; entries point into it.
    text: Cow<'static, str>,
    /// [`ENTRY_LEN`] bytes per word, sorted by word without regard to case,
    /// then most frequent first; see [`compile::ENTRY_LEN`].
    entries: Cow<'static, [u8]>,
    /// Whitelisted replacements, by the replaced word in lowercase.
    replacements: HashMap<String, String>,
}

/// One listed word.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Word<'a> {
    /// As written in the list.
    pub(crate) text: &'a str,
    /// Log-scale frequency, 0-255: one step is about 1.15 times as likely.
    pub(crate) f: u8,
    /// Whether it may be suggested: not profanity, not offensive, not a
    /// shortcut-only entry.
    pub(crate) suggest: bool,
    /// Whether it is a word at all (a shortcut-only entry is not).
    pub(crate) real: bool,
}

/// A bundled list as `build.rs` compiled it, by locale.
macro_rules! compiled {
    ($name:literal) => {
        Wordlist::new(
            Cow::Borrowed(include_str!(concat!(env!("OUT_DIR"), "/", $name, ".words"))),
            Cow::Borrowed(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/",
                $name,
                ".entries"
            ))),
            include_str!(concat!(env!("OUT_DIR"), "/", $name, ".replacements")),
        )
    };
}

impl Wordlist {
    fn new(text: Cow<'static, str>, entries: Cow<'static, [u8]>, replacements: &str) -> Self {
        let replacements = replacements
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(from, to)| (from.to_string(), to.to_string()))
            .collect();
        Self {
            text,
            entries,
            replacements,
        }
    }

    /// Parse and compile the combined format; see [`compile::compile`].
    pub(crate) fn parse(combined: &str) -> Wordlist {
        let Compiled {
            text,
            entries,
            replacements,
        } = compile::compile(combined);
        Self::new(text.into(), entries.into(), &replacements)
    }

    /// Parse gzip-compressed combined text; `None` if it does not inflate
    /// to UTF-8.
    pub(crate) fn parse_gz(gz: &[u8]) -> Option<Wordlist> {
        let mut text = String::new();
        flate2::read::GzDecoder::new(gz)
            .read_to_string(&mut text)
            .ok()?;
        Some(Self::parse(&text))
    }

    /// A word list shipped in the binary, by locale, compiled at build
    /// time from `resources/dict/<locale>_wordlist.combined.gz`.
    pub(crate) fn bundled(name: &str) -> Option<Wordlist> {
        match name {
            "en_US" => Some(compiled!("en_US")),
            _ => None,
        }
    }

    /// Load `spec` as a bundled name, else as a file path (gzipped if it
    /// ends in `.gz`); if neither works, warn and return an empty list.
    pub(crate) fn load(spec: &str) -> Wordlist {
        if let Some(w) = Self::bundled(spec) {
            return w;
        }
        let loaded = std::fs::read(spec)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                let list = if spec.ends_with(".gz") {
                    Self::parse_gz(&bytes)
                } else {
                    String::from_utf8(bytes).ok().map(|t| Self::parse(&t))
                };
                list.ok_or_else(|| "not a combined word list".to_string())
            });
        loaded.unwrap_or_else(|e| {
            tracing::warn!("cannot load word list {spec:?}: {e}; using an empty one");
            Self::parse("")
        })
    }

    /// Number of entries.
    pub(crate) fn len(&self) -> usize {
        self.entries.len() / ENTRY_LEN
    }

    /// Entry `i`'s word.
    fn text_at(&self, i: usize) -> &str {
        let e = &self.entries[i * ENTRY_LEN..][..ENTRY_LEN];
        let start = u32::from_le_bytes([e[0], e[1], e[2], e[3]]) as usize;
        &self.text[start..start + usize::from(e[4])]
    }

    /// Entry `i`.
    fn word(&self, i: usize) -> Word<'_> {
        let e = &self.entries[i * ENTRY_LEN..][..ENTRY_LEN];
        Word {
            text: self.text_at(i),
            f: e[5],
            suggest: e[6] & SUGGEST != 0,
            real: e[6] & REAL != 0,
        }
    }

    /// The first entry not before `word` without regard to case.
    fn lower_bound(&self, word: &str) -> usize {
        let (mut lo, mut hi) = (0, self.len());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if cmp_folded(self.text_at(mid), word) == Ordering::Less {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }

    /// Every casing of `word` listed (`ifs`, `IFs`, ...), most frequent
    /// first.
    pub(crate) fn lookup(&self, word: &str) -> impl Iterator<Item = Word<'_>> {
        (self.lower_bound(word)..self.len())
            .take_while(move |&i| cmp_folded(self.text_at(i), word) == Ordering::Equal)
            .map(|i| self.word(i))
    }

    /// Every listed word starting with `prefix`, without regard to case, in
    /// no useful order.
    pub(crate) fn prefixed(&self, prefix: &str) -> impl Iterator<Item = Word<'_>> {
        (self.lower_bound(prefix)..self.len())
            .take_while(move |&i| starts_with_folded(self.text_at(i), prefix))
            .map(|i| self.word(i))
    }

    /// The whitelisted replacement for `word` (any case), if there is one.
    pub(crate) fn replacement(&self, word: &str) -> Option<&str> {
        let key: String = folded(word).collect();
        self.replacements.get(&key).map(String::as_str)
    }
}

/// `a` and `b` compared without regard to case: the order
/// [`compile::compile`] sorts entries in.
fn cmp_folded(a: &str, b: &str) -> Ordering {
    folded(a).cmp(folded(b))
}

/// Whether `s` starts with `prefix`, without regard to case.
fn starts_with_folded(s: &str, prefix: &str) -> bool {
    let mut s = folded(s);
    folded(prefix).all(|p| s.next() == Some(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "\
dictionary=main:en,locale=en,description=Sample,date=1,version=1
 word=the,f=222,flags=,originalFreq=222
  bigram=end,f=200
 word=Monday,f=111,flags=,originalFreq=111
 word=US,f=0,flags=,originalFreq=152
 word=us,f=160,flags=,originalFreq=134
 word=ill,f=112,flags=,originalFreq=128
  shortcut=I'll,f=whitelist
 word=im,f=0,not_a_word=true
  shortcut=I'm,f=whitelist
 word=shortcut,f=176
  shortcut=target,f=10
 word=rude,f=90,flags=offensive,originalFreq=90
 word=meh,f=80,flags=,originalFreq=80,possibly_offensive=true
 word=café,f=103,flags=,originalFreq=103
 word=broken,f=lots
";

    fn texts<'a>(words: impl Iterator<Item = Word<'a>>) -> Vec<&'a str> {
        words.map(|w| w.text).collect()
    }

    #[test]
    fn lookup_ignores_case_and_keeps_the_listed_spelling() {
        let w = Wordlist::parse(SAMPLE);
        assert_eq!(texts(w.lookup("us")), ["us", "US"], "most frequent first");
        assert_eq!(texts(w.lookup("MONDAY")), ["Monday"]);
        assert_eq!(texts(w.lookup("Café")), ["café"]);
        assert!(w.lookup("mon").next().is_none());
        assert!(w.lookup("broken").next().is_none(), "malformed");
    }

    #[test]
    fn prefixed_ignores_case() {
        let w = Wordlist::parse(SAMPLE);
        let mut found = texts(w.prefixed("m"));
        found.sort();
        assert_eq!(found, ["Monday", "meh"]);
        assert_eq!(w.prefixed("").count(), w.len());
        assert!(w.prefixed("zz").next().is_none());
    }

    #[test]
    fn flags_decide_what_is_suggested() {
        let w = Wordlist::parse(SAMPLE);
        let first = |word| w.lookup(word).next().unwrap();
        assert!(first("the").suggest && first("the").real);
        let us_caps = w.lookup("us").find(|w| w.text == "US").unwrap();
        assert!(
            !us_caps.suggest && us_caps.real,
            "f=0: real, never suggested"
        );
        assert!(!first("rude").suggest && first("rude").real);
        assert!(!first("meh").suggest && first("meh").real);
        assert!(!first("im").suggest && !first("im").real);
    }

    #[test]
    fn only_whitelisted_shortcuts_replace() {
        let w = Wordlist::parse(SAMPLE);
        assert_eq!(w.replacement("im"), Some("I'm"));
        assert_eq!(w.replacement("Ill"), Some("I'll"));
        assert_eq!(w.replacement("shortcut"), None);
        assert_eq!(w.replacement("the"), None);
    }

    #[test]
    fn bundled_list_loads_with_casing_and_apostrophes() {
        let w = Wordlist::bundled("en_US").expect("en_US is bundled");
        assert!(w.len() > 150_000, "only {}", w.len());
        assert_eq!(texts(w.lookup("i'm")), ["I'm"]);
        assert_eq!(texts(w.lookup("iphone")), ["iPhone"]);
        assert_eq!(w.replacement("im"), Some("I'm"));
        assert!(Wordlist::bundled("xx").is_none());
    }

    #[test]
    fn the_build_compiles_like_the_runtime() {
        let gz = include_bytes!("../../resources/dict/en_US_wordlist.combined.gz");
        let runtime = Wordlist::parse_gz(gz).unwrap();
        let built = Wordlist::bundled("en_US").unwrap();
        assert_eq!(built.text, runtime.text);
        assert_eq!(built.entries, runtime.entries);
        assert_eq!(built.replacements, runtime.replacements);
    }

    #[test]
    fn gz_and_missing_files() {
        use std::io::Write;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(SAMPLE.as_bytes()).unwrap();
        let w = Wordlist::parse_gz(&gz.finish().unwrap()).unwrap();
        assert_eq!(w.len(), Wordlist::parse(SAMPLE).len());
        assert!(Wordlist::parse_gz(b"not gzip").is_none());
        assert_eq!(Wordlist::load("/nonexistent/list.combined.gz").len(), 0);
    }
}
