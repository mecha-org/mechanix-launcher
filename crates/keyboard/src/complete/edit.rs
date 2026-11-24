//! Turning a source's candidate words into [`Suggestion`]s: what each one
//! shows and the exact edit a tap on it makes. Every source shares this, so
//! the byte arithmetic and casing live in one place.

use super::{Candidate, Kind, Query};

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
    q: &Query,
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

/// `next` reordered to keep the bar steady: a word `prev` also showed (in
/// any casing) keeps its slot when that slot still exists, so the word under
/// a finger about to tap stays put; the rest fill the free slots in `next`'s
/// order.
pub(crate) fn stabilize(prev: &[Suggestion], next: Vec<Suggestion>) -> Vec<Suggestion> {
    let mut slots: Vec<Option<Suggestion>> = (0..next.len()).map(|_| None).collect();
    let mut rest = Vec::new();
    for s in next {
        let was = prev
            .iter()
            .position(|p| p.label.to_lowercase() == s.label.to_lowercase());
        match was {
            Some(i) if slots.get(i).is_some_and(Option::is_none) => slots[i] = Some(s),
            _ => rest.push(s),
        }
    }
    // As many slots are free as words are left.
    let mut rest = rest.into_iter();
    slots
        .into_iter()
        .filter_map(|slot| slot.or_else(|| rest.next()))
        .collect()
}

/// `candidate` cased to match `typed`: all capitals if `typed` is (two or
/// more letters, all uppercase), a capital first letter if `typed` starts
/// with one, else as the source gave it (keeping capitals it has, like
/// "I" or a name).
fn recase(typed: &str, candidate: &str) -> String {
    let letters: Vec<char> = typed.chars().filter(|c| c.is_alphabetic()).collect();
    if letters.len() >= 2 && letters.iter().all(|c| c.is_uppercase()) {
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

    fn q<'a>(word: &'a str, word_after: &'a str) -> Query<'a> {
        Query {
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

    #[test]
    fn stabilize_keeps_words_in_their_slots() {
        // "that" stays second though it now ranks first; "the" takes the
        // free slot.
        let s = stabilize(
            &sugg(&["this", "that", "they"]),
            sugg(&["that", "the", "this"]),
        );
        assert_eq!(labels(&s), ["this", "that", "the"]);
    }

    #[test]
    fn stabilize_matches_words_in_any_casing() {
        // "Then" (capitalised now) was "then" in the first slot.
        let s = stabilize(&sugg(&["then", "than"]), sugg(&["The", "Then"]));
        assert_eq!(labels(&s), ["Then", "The"]);
    }

    #[test]
    fn stabilize_ranks_words_whose_slot_is_gone() {
        // "then" was third, but only two slots remain: it goes by rank.
        let s = stabilize(&sugg(&["a", "b", "then"]), sugg(&["the", "then"]));
        assert_eq!(labels(&s), ["the", "then"]);
        // A word in the middle slot stays there among new ones.
        let s = stabilize(&sugg(&["a", "the"]), sugg(&["x", "y", "The"]));
        assert_eq!(labels(&s), ["x", "The", "y"]);
    }

    #[test]
    fn stabilize_with_nothing_before_keeps_the_ranking() {
        let next = sugg(&["a", "b", "c"]);
        assert_eq!(stabilize(&[], next.clone()), next);
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
