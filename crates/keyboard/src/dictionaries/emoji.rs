//! Emoji lists for emoji suggestions: each emoji with the names it is
//! found by (shortcodes like `smile`, keywords like `happy`). Read from
//! GitHub gemoji's `db/emoji.json`, compiled ([`compile`]) into a form
//! searched in place: the bundled list at build time, so loading it costs
//! nothing, a list from a path at runtime.

mod compile;

use std::borrow::Cow;
use std::io::Read;

use compile::{Compiled, EMOJI_LEN, KEYWORD, NAME_LEN, SHORTCODE, SHORTCODE_WORD};

/// An emoji list, searched by name.
pub(crate) struct EmojiList {
    /// Every emoji and name, back to back; entries point into it.
    text: Cow<'static, str>,
    /// [`EMOJI_LEN`] bytes per emoji, in the list's order; see
    /// [`compile::EMOJI_LEN`].
    emoji: Cow<'static, [u8]>,
    /// [`NAME_LEN`] bytes per name, sorted by name; see
    /// [`compile::NAME_LEN`].
    names: Cow<'static, [u8]>,
}

/// How well an emoji's name matches a query; lower is better.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Match {
    /// A shortcode is the query.
    Exact,
    /// A shortcode starts with the query.
    Shortcode,
    /// A later word of a shortcode (`slightly_smiling_face`) or a keyword
    /// starts with the query.
    Word,
}

/// A bundled list as `build.rs` compiled it, by name.
macro_rules! compiled {
    ($name:literal) => {
        EmojiList {
            text: Cow::Borrowed(include_str!(concat!(env!("OUT_DIR"), "/", $name, ".text"))),
            emoji: Cow::Borrowed(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/",
                $name,
                ".emoji"
            ))),
            names: Cow::Borrowed(include_bytes!(concat!(
                env!("OUT_DIR"),
                "/",
                $name,
                ".names"
            ))),
        }
    };
}

impl EmojiList {
    /// Parse and compile gemoji JSON; see [`compile::compile`]. `None` if it
    /// is not a gemoji list.
    pub(crate) fn parse(json: &str) -> Option<EmojiList> {
        let Compiled { text, emoji, names } = compile::compile(json)?;
        Some(EmojiList {
            text: text.into(),
            emoji: emoji.into(),
            names: names.into(),
        })
    }

    /// Parse gzip-compressed gemoji JSON; `None` if it does not inflate to
    /// a gemoji list.
    pub(crate) fn parse_gz(gz: &[u8]) -> Option<EmojiList> {
        let mut json = String::new();
        flate2::read::GzDecoder::new(gz)
            .read_to_string(&mut json)
            .ok()?;
        Self::parse(&json)
    }

    /// An emoji list shipped in the binary, by name, compiled at build time
    /// from `resources/emoji/<name>.json.gz`.
    pub(crate) fn bundled(name: &str) -> Option<EmojiList> {
        match name {
            "gemoji" => Some(compiled!("gemoji")),
            _ => None,
        }
    }

    /// Load `spec` as a bundled name, else as a file path (gzipped if it
    /// ends in `.gz`); if neither works, warn and return an empty list.
    pub(crate) fn load(spec: &str) -> EmojiList {
        if let Some(list) = Self::bundled(spec) {
            return list;
        }
        let loaded = std::fs::read(spec)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                let list = if spec.ends_with(".gz") {
                    Self::parse_gz(&bytes)
                } else {
                    String::from_utf8(bytes).ok().and_then(|t| Self::parse(&t))
                };
                list.ok_or_else(|| "not a gemoji list".to_string())
            });
        loaded.unwrap_or_else(|e| {
            tracing::warn!("cannot load emoji list {spec:?}: {e}; using an empty one");
            Self::empty()
        })
    }

    /// A list with no emoji.
    fn empty() -> EmojiList {
        EmojiList {
            text: Cow::Borrowed(""),
            emoji: Cow::Borrowed(&[]),
            names: Cow::Borrowed(&[]),
        }
    }

    /// Number of emoji.
    #[cfg(test)]
    pub(crate) fn len(&self) -> usize {
        self.emoji.len() / EMOJI_LEN
    }

    /// Emoji `i`.
    fn emoji_at(&self, i: usize) -> &str {
        let e = &self.emoji[i * EMOJI_LEN..][..EMOJI_LEN];
        let start = u32::from_le_bytes([e[0], e[1], e[2], e[3]]) as usize;
        &self.text[start..start + usize::from(e[4])]
    }

    /// Every emoji, in the list's order.
    #[cfg(test)]
    pub(crate) fn all(&self) -> impl Iterator<Item = &str> {
        (0..self.len()).map(|i| self.emoji_at(i))
    }

    /// Number of names.
    fn name_count(&self) -> usize {
        self.names.len() / NAME_LEN
    }

    /// Name entry `i`: the name, its kind and its emoji's index.
    fn name_at(&self, i: usize) -> (&str, u8, usize) {
        let n = &self.names[i * NAME_LEN..][..NAME_LEN];
        let start = u32::from_le_bytes([n[0], n[1], n[2], n[3]]) as usize;
        let name = &self.text[start..start + usize::from(n[4])];
        (name, n[5], usize::from(u16::from_le_bytes([n[6], n[7]])))
    }

    /// The emoji with a name starting with `query` (any casing), best
    /// first, at most `max`: an exact shortcode, then shortcodes starting
    /// with it (shorter first), then emoji with a shortcode word or keyword
    /// starting with it; ties in the list's order.
    pub(crate) fn search(&self, query: &str, max: usize) -> Vec<&str> {
        let q = query.to_lowercase();
        let first = self.lower_bound(&q);
        // Per emoji its best (match, matched name's length); the names
        // starting with `q` are one run of the sorted index.
        let mut best: Vec<(Match, usize, usize)> = Vec::new();
        for i in first..self.name_count() {
            let (name, kind, emoji) = self.name_at(i);
            if !name.starts_with(&q) {
                break;
            }
            let m = match kind {
                SHORTCODE if name == q => Match::Exact,
                SHORTCODE => Match::Shortcode,
                SHORTCODE_WORD | KEYWORD => Match::Word,
                _ => continue,
            };
            let rank = (m, name.len(), emoji);
            match best.iter_mut().find(|b| b.2 == emoji) {
                Some(b) if rank < *b => *b = rank,
                Some(_) => {}
                None => best.push(rank),
            }
        }
        best.sort_unstable();
        best.into_iter()
            .take(max)
            .map(|(.., i)| self.emoji_at(i))
            .collect()
    }

    /// The first name not before `q`.
    fn lower_bound(&self, q: &str) -> usize {
        let (mut lo, mut hi) = (0, self.name_count());
        while lo < hi {
            let mid = lo + (hi - lo) / 2;
            if self.name_at(mid).0 < q {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        lo
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
      {"emoji": "😀", "description": "grinning face", "aliases": ["grinning"],
       "tags": ["smile", "happy"], "unicode_version": "6.1"},
      {"emoji": "😃", "description": "grinning face with big eyes",
       "aliases": ["smiley"], "tags": ["happy", "joy", "haha"]},
      {"emoji": "😄", "description": "grinning face with smiling eyes",
       "aliases": ["smile"], "tags": ["happy", "joy", "laugh", "pleased"]},
      {"emoji": "🙂", "description": "slightly smiling face",
       "aliases": ["slightly_smiling_face"], "tags": []},
      {"emoji": "👍", "description": "thumbs up", "aliases": ["+1", "thumbsup"],
       "tags": ["approve", "ok"]},
      {"emoji": "", "description": "no emoji", "aliases": ["nothing"]},
      {"emoji": "❓", "description": "no shortcode", "aliases": []}
    ]"#;

    fn sample() -> EmojiList {
        EmojiList::parse(SAMPLE).unwrap()
    }

    #[test]
    fn entries_without_an_emoji_or_shortcode_are_skipped() {
        let list = sample();
        assert_eq!(list.len(), 5);
        assert!(list.search("nothing", 3).is_empty());
        assert!(list.search("shortcode", 3).is_empty());
    }

    #[test]
    fn an_exact_shortcode_then_prefixes_shorter_first_then_words() {
        let list = sample();
        // "smile" exact; "smiley" a longer shortcode; 🙂's shortcode word
        // "smiling" and 😀's tag "smile" are words.
        assert_eq!(list.search("smile", 5), ["😄", "😃", "😀"]);
        assert_eq!(list.search("smil", 5), ["😄", "😃", "😀", "🙂"]);
        assert_eq!(list.search("SMIL", 2), ["😄", "😃"]);
    }

    #[test]
    fn shortcodes_of_symbols_and_keywords_match() {
        let list = sample();
        assert_eq!(list.search("+1", 3), ["👍"]);
        assert_eq!(list.search("thumbs", 3), ["👍"]);
        assert_eq!(list.search("approve", 3), ["👍"]);
        // Description words, but not stopwords.
        assert_eq!(list.search("eyes", 3), ["😃", "😄"]);
        assert!(list.search("with", 3).is_empty());
    }

    #[test]
    fn each_emoji_once_with_its_best_match() {
        let list = sample();
        // 😄 is "smile" exactly and has the keyword "smiling" too.
        let found = list.search("smi", 10);
        assert_eq!(found.iter().filter(|e| **e == "😄").count(), 1);
    }

    #[test]
    fn not_json_or_not_gemoji_is_none() {
        assert!(EmojiList::parse("not json").is_none());
        assert!(EmojiList::parse(r#"{"emoji": "😀"}"#).is_none());
        assert_eq!(EmojiList::parse("[]").unwrap().len(), 0);
    }

    #[test]
    fn gz_and_missing_files() {
        use std::io::Write;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(SAMPLE.as_bytes()).unwrap();
        let list = EmojiList::parse_gz(&gz.finish().unwrap()).unwrap();
        assert_eq!(list.len(), 5);
        assert!(EmojiList::parse_gz(b"not gzip").is_none());
        assert_eq!(EmojiList::load("/nonexistent/emoji.json.gz").len(), 0);
    }

    #[test]
    fn the_build_compiles_like_the_runtime() {
        let gz = include_bytes!("../../resources/emoji/gemoji.json.gz");
        let runtime = EmojiList::parse_gz(gz).unwrap();
        let built = EmojiList::bundled("gemoji").unwrap();
        assert_eq!(built.text, runtime.text);
        assert_eq!(built.emoji, runtime.emoji);
        assert_eq!(built.names, runtime.names);
        assert!(EmojiList::bundled("xx").is_none());
    }

    #[test]
    fn bundled_list_loads() {
        let list = EmojiList::bundled("gemoji").unwrap();
        assert!(list.len() > 1800, "only {}", list.len());
        assert_eq!(list.search("smile", 1), ["😄"]);
        assert_eq!(list.search("+1", 1), ["👍"]);
        assert_eq!(list.search("pizz", 3), ["🍕"]);
    }
}
