//! Turning a source's candidate words into [`Suggestion`]s: what each one
//! shows and the exact edit a tap on it makes. Every source shares this, so
//! the byte arithmetic and casing live in one place.

use super::{Candidate, Kind, WordQuery};

/// What accepting a suggestion does to the field, sent as one input-method
/// commit: delete `delete_before` bytes before the cursor and
/// `delete_after` bytes after it, then insert `insert` at the cursor.
/// Lengths are in bytes, as `zwp_input_method_v2.delete_surrounding_text`
/// takes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct TextEdit {
    pub(crate) delete_before: u32,
    pub(crate) delete_after: u32,
    pub(crate) insert: String,
}

/// One entry of the suggestion bar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Suggestion {
    /// What the bar shows.
    pub(crate) label: String,
    pub(crate) kind: Kind,
    /// What a tap on it does.
    pub(crate) edit: TextEdit,
}

/// The suggestions for `q` from a source's ranked `candidates`: cased like
/// the typed word, each replacing the whole word around the cursor (with a
/// space after it if `append_space`), at most `q.max` of them. Candidates
/// that would leave the word as typed, and repeats, are dropped.
pub(crate) fn suggestions(
    q: &WordQuery,
    candidates: Vec<Candidate>,
    append_space: bool,
) -> Vec<Suggestion> {
    let typed = format!("{}{}", q.word, q.word_after);
    let mut out: Vec<Suggestion> = Vec::with_capacity(q.max);
    for c in candidates {
        if out.len() >= q.max {
            break;
        }
        let label = recase(&typed, &c.text);
        if label.is_empty() || label == typed || out.iter().any(|s| s.label == label) {
            continue;
        }
        let mut insert = label.clone();
        if append_space {
            insert.push(' ');
        }
        out.push(Suggestion {
            label,
            kind: c.kind,
            edit: TextEdit {
                delete_before: q.word.len() as u32,
                delete_after: q.word_after.len() as u32,
                insert,
            },
        });
    }
    out
}

/// The bar's `slots` slots, left to right, in the order the best words take
/// them: the middle one first (left of middle when `slots` is even), then
/// outwards, alternating sides and starting with the side that has more
/// room (the left on a tie), so the words stay centred.
pub(crate) fn center_out(slots: usize) -> Vec<usize> {
    if slots == 0 {
        return Vec::new();
    }
    let mid = (slots - 1) / 2;
    let mut order = vec![mid];
    let (mut left, mut right) = (mid, mid + 1);
    while order.len() < slots {
        // Slots still free on each side.
        let (l, r) = (left, slots - right);
        if l >= r {
            left -= 1;
            order.push(left);
        } else {
            order.push(right);
            right += 1;
        }
    }
    order
}

/// `next` (best first) laid out in the bar's `slots` slots, left to right,
/// kept steady: a word `prev` also showed (in any casing) keeps its slot, so
/// the word under a finger about to tap stays put; the rest take the free
/// slots from the middle outwards ([`center_out`]), best first. No words
/// leave the bar empty.
pub(crate) fn stabilize(
    prev: &[Option<Suggestion>],
    next: Vec<Suggestion>,
    slots: usize,
) -> Vec<Option<Suggestion>> {
    if next.is_empty() {
        return Vec::new();
    }
    let mut out: Vec<Option<Suggestion>> = (0..slots).map(|_| None).collect();
    let mut rest = Vec::new();
    for s in next {
        let label = s.label.to_lowercase();
        let was = prev
            .iter()
            .position(|p| p.as_ref().is_some_and(|p| p.label.to_lowercase() == label));
        match was {
            Some(i) if out.get(i).is_some_and(Option::is_none) => out[i] = Some(s),
            _ => rest.push(s),
        }
    }
    let mut rest = rest.into_iter();
    for i in center_out(slots) {
        if out[i].is_none() {
            out[i] = rest.next();
        }
    }
    out
}

/// Whether `word` is in all capitals: two or more letters, all uppercase.
pub(crate) fn all_caps(word: &str) -> bool {
    let mut letters = word.chars().filter(|c| c.is_alphabetic());
    letters.clone().count() >= 2 && letters.all(char::is_uppercase)
}

/// `candidate` cased to match `typed`: all capitals if `typed` is (two or
/// more letters, all uppercase), a capital first letter if `typed` starts
/// with one, else as the source gave it (keeping capitals it has, like
/// "I" or a name).
fn recase(typed: &str, candidate: &str) -> String {
    if all_caps(typed) {
        return candidate.to_uppercase();
    }
    if typed.chars().next().is_some_and(char::is_uppercase) {
        let mut chars = candidate.chars();
        return match chars.next() {
            Some(first) => first.to_uppercase().chain(chars).collect(),
            None => String::new(),
        };
    }
    candidate.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q<'a>(word: &'a str, word_after: &'a str) -> WordQuery<'a> {
        WordQuery {
            word,
            word_after,
            max: 3,
        }
    }

    fn cands(words: &[&str]) -> Vec<Candidate> {
        words
            .iter()
            .map(|w| Candidate {
                text: w.to_string(),
                kind: Kind::Completion,
            })
            .collect()
    }

    fn labels(s: &[Suggestion]) -> Vec<&str> {
        s.iter().map(|s| s.label.as_str()).collect()
    }

    #[test]
    fn replaces_the_word_before_the_cursor_and_adds_a_space() {
        let s = suggestions(&q("documen", ""), cands(&["documents"]), true);
        assert_eq!(
            s[0].edit,
            TextEdit {
                delete_before: 7,
                delete_after: 0,
                insert: "documents ".into()
            }
        );
    }

    #[test]
    fn mid_word_cursor_replaces_the_whole_word() {
        let s = suggestions(&q("do", "cumen"), cands(&["documents"]), true);
        assert_eq!((s[0].edit.delete_before, s[0].edit.delete_after), (2, 5));
    }

    #[test]
    fn lengths_are_bytes() {
        let s = suggestions(&q("caf", "é"), cands(&["cafés"]), false);
        assert_eq!((s[0].edit.delete_before, s[0].edit.delete_after), (3, 2));
        assert_eq!(s[0].edit.insert, "cafés");
    }

    #[test]
    fn no_space_when_not_asked() {
        let s = suggestions(&q("exam", ""), cands(&["example"]), false);
        assert_eq!(s[0].edit.insert, "example");
    }

    #[test]
    fn casing_follows_the_typed_word() {
        assert_eq!(
            labels(&suggestions(&q("Teh", ""), cands(&["the"]), true)),
            ["The"]
        );
        assert_eq!(
            labels(&suggestions(&q("TEH", ""), cands(&["the"]), true)),
            ["THE"]
        );
        assert_eq!(
            labels(&suggestions(&q("teh", ""), cands(&["the"]), true)),
            ["the"]
        );
        // A single capital is a capitalised word, not all caps.
        assert_eq!(
            labels(&suggestions(&q("T", ""), cands(&["the"]), true)),
            ["The"]
        );
        // Capitals the source has stay.
        assert_eq!(
            labels(&suggestions(&q("i", ""), cands(&["I"]), true)),
            ["I"]
        );
    }

    fn sugg(words: &[&str]) -> Vec<Suggestion> {
        suggestions(&q("", ""), cands(words), true)
    }

    /// Each slot's label, `""` for an empty one.
    fn slots(s: &[Option<Suggestion>]) -> Vec<&str> {
        s.iter()
            .map(|s| s.as_ref().map_or("", |s| s.label.as_str()))
            .collect()
    }

    #[test]
    fn center_out_starts_in_the_middle() {
        assert_eq!(center_out(0), Vec::<usize>::new());
        assert_eq!(center_out(1), [0]);
        assert_eq!(center_out(2), [0, 1]);
        assert_eq!(center_out(3), [1, 0, 2]);
        assert_eq!(center_out(4), [1, 2, 0, 3]);
        assert_eq!(center_out(5), [2, 1, 3, 0, 4]);
    }

    #[test]
    fn stabilize_centres_the_best_word() {
        let s = stabilize(&[], sugg(&["a", "b", "c"]), 3);
        assert_eq!(slots(&s), ["b", "a", "c"]);
        let s = stabilize(&[], sugg(&["a"]), 3);
        assert_eq!(slots(&s), ["", "a", ""]);
        let s = stabilize(&[], sugg(&["a", "b"]), 3);
        assert_eq!(slots(&s), ["b", "a", ""]);
    }

    #[test]
    fn stabilize_empty_leaves_the_bar_blank() {
        let prev = stabilize(&[], sugg(&["a"]), 3);
        assert!(stabilize(&prev, Vec::new(), 3).is_empty());
    }

    #[test]
    fn stabilize_keeps_words_in_their_slots() {
        // Shown as [that, this, they]. "that" stays left though it now
        // ranks first; "the" takes the free middle.
        let prev = stabilize(&[], sugg(&["this", "that", "they"]), 3);
        assert_eq!(slots(&prev), ["that", "this", "they"]);
        let s = stabilize(&prev, sugg(&["that", "the", "then"]), 3);
        assert_eq!(slots(&s), ["that", "the", "then"]);
        // A word kept off-centre stays there as the others come and go.
        let s = stabilize(&s, sugg(&["that"]), 3);
        assert_eq!(slots(&s), ["that", "", ""]);
        let s = stabilize(&s, sugg(&["those", "that"]), 3);
        assert_eq!(slots(&s), ["that", "those", ""]);
    }

    #[test]
    fn stabilize_matches_words_in_any_casing() {
        // "Then" (capitalised now) was "then" on the left.
        let prev = stabilize(&[], sugg(&["than", "then"]), 3);
        assert_eq!(slots(&prev), ["then", "than", ""]);
        let s = stabilize(&prev, sugg(&["The", "Then"]), 3);
        assert_eq!(slots(&s), ["Then", "The", ""]);
    }

    #[test]
    fn drops_the_typed_word_repeats_and_extras() {
        let s = suggestions(
            &q("Th", ""),
            cands(&["th", "the", "The", "that", "this", "they"]),
            true,
        );
        assert_eq!(labels(&s), ["The", "That", "This"]);
    }
}
