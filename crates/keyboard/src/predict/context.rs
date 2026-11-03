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
    /// Whether the client has sent surrounding text since the last reset.
    surrounding_mode: bool,
}
impl Resource for TypingContext {}

impl TypingContext {
    /// The word prefix: surrounding part followed by the local part.
    pub(crate) fn prefix(&self) -> String {
        format!("{}{}", self.surrounding, self.local)
    }

    /// Take the client's surrounding text; the prefix becomes the word run
    /// immediately before byte offset `cursor`. An invalid cursor gives an
    /// empty prefix. Drops the local part, which the client's text supersedes.
    pub(crate) fn set_surrounding(&mut self, text: &str, cursor: u32) {
        self.surrounding_mode = true;
        self.local.clear();
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
        let mut done = Vec::new();
        for c in text.chars() {
            if is_word_char(c) {
                self.local.push(c);
            } else if let Some(word) = self.end_word() {
                done.push(word);
            }
        }
        done
    }

    /// A single character was deleted before the cursor.
    pub(crate) fn backspace(&mut self) {
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

    /// Drop the word without returning it; keeps surrounding mode.
    pub(crate) fn clear(&mut self) {
        self.surrounding.clear();
        self.local.clear();
    }

    /// Forget everything and leave surrounding mode (a field was activated).
    pub(crate) fn reset(&mut self) {
        self.clear();
        self.surrounding_mode = false;
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
    fn reset_leaves_surrounding_mode() {
        let mut c = TypingContext::default();
        c.set_surrounding("hello wor", 9);
        c.reset();
        assert_eq!(c.prefix(), "");
        assert!(!c.surrounding_mode);
    }

    #[test]
    fn end_word_returns_word_once() {
        let mut c = TypingContext::default();
        c.committed("hi");
        assert_eq!(c.end_word(), Some("hi".to_string()));
        assert_eq!(c.end_word(), None);
    }
}
