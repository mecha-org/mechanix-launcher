//! Completion and correction from a [`Wordlist`]: the most frequent listed
//! words starting with the typed word, in their listed spelling ("iph" →
//! "iPhone"), and, for a word not listed as typed, its fix: the list's own
//! replacement ("im" → "I'm"), its listed casing ("monday" → "Monday"), or
//! the most frequent words one edit away ("teh" → "the", "dont" → "don't").
//!
//! Frequencies are the list's log-scale `f`: in the bundled list, ten times
//! as frequent is about 30 steps higher.

use crate::complete::edit::all_caps;
use crate::complete::{Candidate, Completer, Kind, WordQuery};
use crate::dictionaries::{Loading, Word, Wordlist};

/// A correction is offered next to completions only when it is this many
/// steps (about twenty times) more frequent than the best completion: "teh"
/// is far likelier a slip for "the" than the start of "tehran", but "th" is
/// just unfinished.
const CORRECTION_DOMINANCE: u8 = 38;

/// The best correction is sure enough to apply by itself (an
/// [`Autocorrection`](Kind::Autocorrection)) only when it is this many
/// steps (about five times) more frequent than the next one: "recieve" is
/// "receive" (not "relieve"), but "waht" could as well be "want" as "what".
const AUTOCORRECT_DOMINANCE: u8 = 20;

/// ... and a common word, this frequent at least: an unlisted word close to
/// only a rare one is likelier a name or jargon.
const AUTOCORRECT_MIN_F: u8 = 100;

/// ... and the typed word at least this long (in chars): too short to be
/// sure of.
const AUTOCORRECT_MIN_LEN: usize = 3;

/// ... and, for words shorter than this, two letters swapped ("teh",
/// "adn"): a short word one letter off a common one is as often slang or
/// an abbreviation ("thx", "hmm").
const AUTOCORRECT_ANY_EDIT_LEN: usize = 4;

/// A word with its apostrophe left out ("dont") is autocorrected to the
/// contraction when that is at least this frequent: "shouldn't" (93) and
/// "haven't" (97) are, "doest" would not be.
const CONTRACTION_MIN_F: u8 = 60;

/// Letters an edit may insert or substitute.
const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz'";

/// Suggests from a [`Wordlist`]; nothing while it is still loading.
pub(crate) struct WordlistCompleter {
    list: Loading<Wordlist>,
}

impl WordlistCompleter {
    pub(crate) fn new(list: Loading<Wordlist>) -> Self {
        Self { list }
    }
}

/// A loaded list, asked about words.
#[derive(Clone, Copy)]
struct Lexicon<'a>(&'a Wordlist);

impl<'a> Lexicon<'a> {
    /// Up to `n` suggestable words that start with `prefix` and end with
    /// `suffix` (both lowercase), longer than both together, most frequent
    /// first.
    fn completions(&self, prefix: &str, suffix: &str, n: usize) -> Vec<Word<'a>> {
        let min = prefix.chars().count() + suffix.chars().count();
        let mut found: Vec<Word> = self
            .0
            .prefixed(prefix)
            .filter(|w| w.suggest && w.text.chars().count() > min)
            .filter(|w| suffix.is_empty() || w.text.to_lowercase().ends_with(suffix))
            .collect();
        by_frequency(&mut found);
        found.truncate(n);
        found
    }

    /// Up to `n` suggestable words one deletion, transposition,
    /// substitution or insertion away from the lowercase `word`, most
    /// frequent first; of several casings, the most frequent.
    fn corrections(&self, word: &str, n: usize) -> Vec<Word<'a>> {
        let mut found: Vec<Word> = Vec::new();
        for edit in edits1(word) {
            if let Some(w) = self.0.lookup(&edit).find(|w| w.suggest)
                && !found.contains(&w)
            {
                found.push(w);
            }
        }
        by_frequency(&mut found);
        found.truncate(n);
        found
    }

    /// Whether `typed` is a listed word in a casing that fits it.
    fn spelled(&self, typed: &str) -> bool {
        self.0
            .lookup(typed)
            .any(|w| w.real && casing_fits(typed, w.text))
    }

    /// The listed spelling of `typed` when it is listed only in other
    /// casings ("monday" → "Monday"). Not into capitals: an acronym typed
    /// in lowercase ("ok", "lol") is meant that way.
    fn recased(&self, typed: &str) -> Option<&'a str> {
        if self.spelled(typed) {
            return None;
        }
        self.0
            .lookup(typed)
            .find(|w| w.real && w.suggest && !all_caps(w.text))
            .map(|w| w.text)
    }

    /// The fixes for `typed` (`lower` lowercased), best first, if it is
    /// not spelled right; `best_completion` is the most frequent completion's
    /// frequency.
    fn fixes(&self, typed: &str, lower: &str, best_completion: Option<u8>) -> Vec<Candidate> {
        // The list's own replacement: certain for an entry it marks as no
        // word, but a real word ("ill" → "I'll") another source may know.
        if let Some(to) = self.0.replacement(typed) {
            let kind = if self.0.lookup(typed).any(|w| w.real) {
                Kind::Autocorrection
            } else {
                Kind::Replacement
            };
            return vec![candidate(to, kind)];
        }
        if let Some(listed) = self.recased(typed) {
            return vec![candidate(listed, Kind::Autocorrection)];
        }
        if self.spelled(typed) || typed.chars().count() < 2 {
            return Vec::new();
        }
        let mut corrections = self.corrections(lower, 3);
        // A left-out apostrophe is the likeliest slip of all (it is off the
        // letter page): the contraction goes first, sure unless rare, even
        // where a real word is as close ("wasnt": "want", "youre": "your").
        let contraction = self.contraction(lower);
        let sure = match contraction {
            Some(c) => c.f >= CONTRACTION_MIN_F,
            None => sure(lower, &corrections),
        };
        corrections.retain(|w| Some(*w) != contraction);
        // Next to completions, only a far more frequent correction is
        // offered (the retain keeps the first or drops it).
        if let Some(best) = best_completion {
            corrections.retain(|w| w.f > best.saturating_add(CORRECTION_DOMINANCE));
        }
        contraction
            .into_iter()
            .chain(corrections)
            .enumerate()
            .map(|(i, w)| {
                let kind = if i == 0 && sure {
                    Kind::Autocorrection
                } else {
                    Kind::Correction
                };
                candidate(w.text, kind)
            })
            .collect()
    }

    /// The suggestable listed word that is the lowercase `word` with an
    /// apostrophe put in ("dont" → "don't", "ive" → "I've"), the most
    /// frequent if several.
    fn contraction(&self, word: &str) -> Option<Word<'a>> {
        (1..word.len())
            .filter(|&i| word.is_char_boundary(i))
            .filter_map(|i| {
                let with = format!("{}'{}", &word[..i], &word[i..]);
                self.0.lookup(&with).find(|w| w.suggest)
            })
            .max_by_key(|w| w.f)
    }
}

fn candidate(text: &str, kind: Kind) -> Candidate {
    Candidate {
        text: text.to_string(),
        kind,
    }
}

/// Most frequent first, ties by spelling.
fn by_frequency(words: &mut [Word]) {
    words.sort_by(|a, b| b.f.cmp(&a.f).then_with(|| a.text.cmp(b.text)));
}

/// Whether `typed` is the listed word `listed` in a casing that fits: as
/// listed, capitalised (as at a sentence start), in capitals, or any casing
/// of a lowercase word.
fn casing_fits(typed: &str, listed: &str) -> bool {
    typed == listed || listed.chars().all(|c| !c.is_uppercase()) || all_caps(typed) || {
        let mut chars = listed.chars();
        chars
            .next()
            .is_some_and(|c| c.to_uppercase().chain(chars).eq(typed.chars()))
    }
}

/// Whether the first of `corrections` (most frequent first) of the unknown
/// lowercase `word` is sure enough to autocorrect it: see
/// [`AUTOCORRECT_DOMINANCE`] and what follows it.
fn sure(word: &str, corrections: &[Word]) -> bool {
    let Some(best) = corrections.first() else {
        return false;
    };
    let next = corrections.get(1).map_or(0, |w| w.f);
    let len = word.chars().count();
    best.f >= AUTOCORRECT_MIN_F
        && best.f >= next.saturating_add(AUTOCORRECT_DOMINANCE)
        && len >= AUTOCORRECT_MIN_LEN
        && (len >= AUTOCORRECT_ANY_EDIT_LEN || swapped(word, &best.text.to_lowercase()))
}

/// Whether `a` is `b` with two neighbouring letters swapped.
fn swapped(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let Some(i) = a.iter().zip(b).position(|(x, y)| x != y) else {
        return false;
    };
    i + 1 < a.len() && a[i] == b[i + 1] && a[i + 1] == b[i] && a[i + 2..] == b[i + 2..]
}

/// Every string one edit away from `word`, editing its bytes with
/// [`ALPHABET`]; edits that split a multi-byte character are dropped.
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
    out.sort_unstable();
    out.dedup();
    out
}

impl Lexicon<'_> {
    fn suggest(&self, q: &WordQuery) -> Vec<Candidate> {
        let typed = format!("{}{}", q.word, q.word_after);
        if typed.is_empty() {
            return Vec::new();
        }
        // Completions extend what is before the cursor.
        let (prefix, suffix) = (q.word.to_lowercase(), q.word_after.to_lowercase());
        let completions = self.completions(&prefix, &suffix, q.max + 1);
        // Fixes first: the typed word looks wrong.
        let mut out = self.fixes(
            &typed,
            &typed.to_lowercase(),
            completions.first().map(|w| w.f),
        );
        out.extend(
            completions
                .into_iter()
                .map(|w| candidate(w.text, Kind::Completion)),
        );
        out
    }
}

impl Completer for WordlistCompleter {
    fn suggest(&mut self, q: &WordQuery) -> Vec<Candidate> {
        self.list
            .get()
            .map_or_else(Vec::new, |list| Lexicon(list).suggest(q))
    }

    fn accepts(&mut self, word: &str) -> bool {
        self.list
            .get()
            .is_some_and(|list| Lexicon(list).spelled(word))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TINY: &str = "\
dictionary=main:en,locale=en,description=Tiny,date=1,version=1
 word=the,f=222
 word=that,f=192
 word=this,f=200
 word=they,f=180
 word=to,f=215
 word=ten,f=140
 word=tea,f=120
 word=Tehran,f=80
 word=document,f=150
 word=documents,f=145
 word=documentation,f=130
 word=Monday,f=111
 word=iPhone,f=100
 word=don't,f=185
 word=done,f=150
 word=ill,f=112
  shortcut=I'll,f=whitelist
 word=im,f=0,not_a_word=true
  shortcut=I'm,f=whitelist
 word=I'm,f=116
 word=thong,f=90,possibly_offensive=true
";

    fn completer() -> WordlistCompleter {
        WordlistCompleter::new(Loading::ready(Wordlist::parse(TINY)))
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

    fn first(word: &str) -> (String, Kind) {
        suggest(word, "").remove(0)
    }

    #[test]
    fn completes_by_frequency_in_the_listed_spelling() {
        let s = suggest("th", "");
        assert_eq!(texts(&s)[..3], ["the", "this", "that"]);
        assert!(s.iter().all(|(_, k)| *k == Kind::Completion));
        assert_eq!(texts(&suggest("iph", "")), ["iPhone"]);
        assert_eq!(texts(&suggest("TH", ""))[0], "the");
    }

    #[test]
    fn a_known_word_is_completed_not_corrected() {
        assert_eq!(
            texts(&suggest("document", "")),
            ["documents", "documentation"]
        );
        assert_eq!(texts(&suggest("Monday", "")), Vec::<&str>::new());
    }

    #[test]
    fn words_never_to_suggest_are_known_but_not_suggested() {
        assert!(!texts(&suggest("tho", "")).contains(&"thong"));
        assert!(completer().accepts("thong"));
    }

    #[test]
    fn misspellings_get_corrections_first() {
        // "Tehran" starts with "teh", but "the" is far more frequent.
        let s = suggest("teh", "");
        assert_eq!(s[0], ("the".into(), Kind::Autocorrection));
        assert!(texts(&s).contains(&"Tehran"));
        // An apostrophe left out.
        assert_eq!(first("dont"), ("don't".into(), Kind::Autocorrection));
        // "tea" and "ten" are both close: offered, not sure.
        let s = suggest("tez", "");
        assert!(texts(&s).contains(&"tea") && texts(&s).contains(&"ten"));
        assert!(s.iter().all(|(_, k)| *k == Kind::Correction), "{s:?}");
    }

    #[test]
    fn listed_replacements_and_casing() {
        // No word: replaced whatever else knows it.
        assert_eq!(first("im"), ("I'm".into(), Kind::Replacement));
        assert_eq!(first("Im"), ("I'm".into(), Kind::Replacement));
        // A real word: another source may veto it.
        assert_eq!(first("ill"), ("I'll".into(), Kind::Autocorrection));
        assert_eq!(first("monday"), ("Monday".into(), Kind::Autocorrection));
        assert_eq!(first("iphone"), ("iPhone".into(), Kind::Autocorrection));
    }

    #[test]
    fn casings_that_fit() {
        let mut c = completer();
        for w in [
            "the", "The", "THE", "Monday", "MONDAY", "iPhone", "IPHONE", "IPhone",
        ] {
            assert!(c.accepts(w), "{w}");
        }
        for w in ["monday", "iphone", "teh", "im", "dont"] {
            assert!(!c.accepts(w), "{w}");
        }
    }

    #[test]
    fn mid_word_completions_keep_the_part_after_the_cursor() {
        // "doc|ument": only longer words ending in what follows the caret.
        assert!(texts(&suggest("doc", "ument")).is_empty());
        assert_eq!(texts(&suggest("doc", "s")), ["documents"]);
    }

    #[test]
    fn nothing_for_nothing() {
        assert!(suggest("", "").is_empty());
    }

    #[test]
    fn swapped_is_one_neighbour_swap() {
        assert!(swapped("teh", "the") && swapped("hte", "the") && swapped("adn", "and"));
        assert!(!swapped("the", "the") && !swapped("thx", "the") && !swapped("tah", "the"));
        assert!(!swapped("teh", "then"));
    }

    fn bundled() -> WordlistCompleter {
        WordlistCompleter::new(Loading::ready(Wordlist::bundled("en_US").unwrap()))
    }

    #[test]
    fn autocorrections_from_the_bundled_list() {
        let mut c = bundled();
        let mut auto = |word| {
            let q = WordQuery {
                word,
                word_after: "",
                max: 3,
            };
            c.suggest(&q)
                .into_iter()
                .find(|c| matches!(c.kind, Kind::Autocorrection | Kind::Replacement))
                .map(|c| c.text)
        };
        for (typo, fix) in [
            ("teh", "the"),
            ("hte", "the"),
            ("adn", "and"),
            ("recieve", "receive"),
            ("becuase", "because"),
            ("thier", "their"),
            ("definately", "definitely"),
            ("dont", "don't"),
            ("didnt", "didn't"),
            ("youre", "you're"),
            ("wasnt", "wasn't"),
            ("theyre", "they're"),
            ("ive", "I've"),
            ("havent", "haven't"),
            ("wierd", "weird"),
            ("im", "I'm"),
            ("i", "I"),
            ("alot", "a lot"),
            ("monday", "Monday"),
        ] {
            assert_eq!(auto(typo).as_deref(), Some(fix), "{typo}");
        }
        for word in [
            "the", "hmm", "thx", "kubectl", "waht", "th", "documen", "ok", "tv",
        ] {
            assert_eq!(auto(word), None, "{word}");
        }
    }

    #[test]
    fn bundled_list_answers_quickly() {
        let mut c = bundled();
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
    }
}
