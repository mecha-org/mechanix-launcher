//! Tracks the word being typed, so the predictor knows its prefix. Fed either
//! by the client's surrounding text (when the input method reports it) or by
//! the letters committed locally.

use mecha_wayland::prelude::*;

/// A backspace held at least this long deletes more than one character, so the
/// word being tracked can no longer be trusted and is dropped.
pub(crate) const HOLD_CLEAR_MS: u32 = 400;

/// Whether `c` belongs inside a word.
fn is_word_char(c: char) -> bool {
    c.is_alphabetic() || c == '\''
}

/// The word currently being typed.
#[derive(Default)]
pub(crate) struct TypingContext {
    /// Word part before the cursor, as reported by the client.
    surrounding: String,
    /// Letters committed locally since the last surrounding update.
    local: String,
    /// Word part after the cursor, as reported by the client (the caret was
    /// moved into a word).
    after: String,
    /// Bumped on every change, so a reader can tell the word changed.
    rev: u64,
}
impl Resource for TypingContext {}

impl TypingContext {
    /// The word prefix: surrounding part followed by the local part.
    pub(crate) fn prefix(&self) -> String {
        format!("{}{}", self.surrounding, self.local)
    }

    /// The word part after the cursor; empty unless the client reported one.
    pub(crate) fn word_after(&self) -> &str {
        &self.after
    }

    /// A number that changes whenever the word does.
    pub(crate) fn rev(&self) -> u64 {
        self.rev
    }

    /// Take the client's surrounding text; the prefix becomes the word run
    /// immediately before byte offset `cursor`, the word after it the run
    /// right after. An invalid cursor gives an empty word. Drops the local
    /// part, which the client's text supersedes.
    pub(crate) fn set_surrounding(&mut self, text: &str, cursor: u32) {
        self.rev += 1;
        self.local.clear();
        self.after = match text.get(cursor as usize..) {
            Some(rest) if text.is_char_boundary(cursor as usize) => {
                rest.chars().take_while(|&c| is_word_char(c)).collect()
            }
            _ => String::new(),
        };
        self.surrounding = match text.get(..cursor as usize) {
            Some(before) => {
                let run: String = before
                    .chars()
                    .rev()
                    .take_while(|&c| is_word_char(c))
                    .collect();
                run.chars().rev().collect()
            }
            None => String::new(),
        };
    }

    /// Record committed text; returns the words it finished, in order.
    pub(crate) fn committed(&mut self, text: &str) -> Vec<String> {
        self.rev += 1;
        let mut done = Vec::new();
        for c in text.chars() {
            if is_word_char(c) {
                self.local.push(c);
            } else {
                // The word now ends here; what was after the cursor follows
                // the separator, not this word.
                self.after.clear();
                if let Some(word) = self.end_word() {
                    done.push(word);
                }
            }
        }
        done
    }

    /// A single character was deleted before the cursor.
    pub(crate) fn backspace(&mut self) {
        self.rev += 1;
        if self.local.pop().is_none() {
            self.surrounding.pop();
        }
    }

    /// A held backspace ended after `held_ms`; long holds drop the word.
    pub(crate) fn backspace_held(&mut self, held_ms: u32) {
        if held_ms >= HOLD_CLEAR_MS {
            self.clear();
        }
    }

    /// Finish the current word, returning it if non-empty.
    pub(crate) fn end_word(&mut self) -> Option<String> {
        let word = self.prefix();
        self.clear();
        (!word.is_empty()).then_some(word)
    }

    /// A suggestion replaced the whole word around the cursor with
    /// `insert`: record that, returning the words it finished.
    pub(crate) fn replaced_word(&mut self, insert: &str) -> Vec<String> {
        self.clear();
        self.committed(insert)
    }

    /// Drop the word without returning it.
    pub(crate) fn clear(&mut self) {
        self.rev += 1;
        self.surrounding.clear();
        self.local.clear();
        self.after.clear();
    }

    /// Forget everything (a field was activated, deactivated or changed).
    pub(crate) fn reset(&mut self) {
        self.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_letters_build_prefix() {
        let mut c = TypingContext::default();
        c.committed("t");
        c.committed("h");
        assert_eq!(c.prefix(), "th");
    }

    #[test]
    fn space_ends_word_and_returns_it() {
        let mut c = TypingContext::default();
        assert_eq!(c.committed("the "), vec!["the".to_string()]);
        assert_eq!(c.prefix(), "");
    }

    #[test]
    fn backspace_pops_local() {
        let mut c = TypingContext::default();
        c.committed("the");
        c.backspace();
        assert_eq!(c.prefix(), "th");
    }

    #[test]
    fn apostrophe_stays_in_word() {
        let mut c = TypingContext::default();
        c.committed("don'");
        assert_eq!(c.prefix(), "don'");
    }

    #[test]
    fn surrounding_sets_prefix_and_local_appends() {
        let mut c = TypingContext::default();
        c.set_surrounding("hello wor", 9);
        assert_eq!(c.prefix(), "wor");
        c.committed("l");
        assert_eq!(c.prefix(), "worl");
    }

    #[test]
    fn surrounding_update_replaces_local() {
        let mut c = TypingContext::default();
        c.set_surrounding("hello wor", 9);
        c.committed("l");
        c.set_surrounding("hello worl", 10);
        assert_eq!(c.prefix(), "worl");
    }

    #[test]
    fn backspace_eats_surrounding_when_local_empty() {
        let mut c = TypingContext::default();
        c.set_surrounding("hello wor", 9);
        c.backspace();
        assert_eq!(c.prefix(), "wo");
    }

    #[test]
    fn surrounding_cursor_out_of_range_gives_empty_prefix() {
        let mut c = TypingContext::default();
        c.set_surrounding("hello", 99);
        assert_eq!(c.prefix(), "");
    }

    #[test]
    fn surrounding_cursor_mid_char_gives_empty_prefix() {
        let mut c = TypingContext::default();
        c.set_surrounding("é", 1);
        assert_eq!(c.prefix(), "");
    }

    #[test]
    fn non_letter_before_cursor_gives_empty_prefix() {
        let mut c = TypingContext::default();
        c.set_surrounding("abc ", 4);
        assert_eq!(c.prefix(), "");
    }

    #[test]
    fn long_backspace_hold_clears_context() {
        let mut c = TypingContext::default();
        c.committed("word");
        c.backspace_held(100);
        assert_eq!(c.prefix(), "word");
        c.backspace_held(HOLD_CLEAR_MS);
        assert_eq!(c.prefix(), "");
    }

    #[test]
    fn reset_forgets_surrounding_and_local() {
        let mut c = TypingContext::default();
        c.set_surrounding("hello wor", 9);
        c.committed("l");
        c.reset();
        assert_eq!(c.prefix(), "");
    }

    #[test]
    fn surrounding_sets_the_word_after_the_cursor() {
        let mut c = TypingContext::default();
        c.set_surrounding("hello wor ld", 8);
        assert_eq!((c.prefix().as_str(), c.word_after()), ("wo", "r"));
        c.set_surrounding("é", 1);
        assert_eq!(c.word_after(), "");
    }

    #[test]
    fn a_separator_ends_the_word_after_too() {
        let mut c = TypingContext::default();
        c.set_surrounding("abcd", 2);
        c.committed(" ");
        assert_eq!((c.prefix().as_str(), c.word_after()), ("", ""));
    }

    #[test]
    fn replacing_the_word_finishes_the_new_one() {
        let mut c = TypingContext::default();
        c.set_surrounding("documen", 7);
        assert_eq!(c.replaced_word("documents "), vec!["documents".to_string()]);
        assert_eq!(c.prefix(), "");
    }

    #[test]
    fn every_change_bumps_the_revision() {
        let mut c = TypingContext::default();
        let mut last = c.rev();
        let mut step = |c: &mut TypingContext| {
            assert_ne!(c.rev(), last);
            last = c.rev();
        };
        c.committed("a");
        step(&mut c);
        c.backspace();
        step(&mut c);
        c.set_surrounding("x", 1);
        step(&mut c);
        c.clear();
        step(&mut c);
    }

    #[test]
    fn end_word_returns_word_once() {
        let mut c = TypingContext::default();
        c.committed("hi");
        assert_eq!(c.end_word(), Some("hi".to_string()));
        assert_eq!(c.end_word(), None);
    }
}
