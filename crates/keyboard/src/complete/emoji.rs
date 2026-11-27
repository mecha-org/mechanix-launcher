//! Emoji by shortcode: typing `:smil` puts 😄 and its neighbours in the
//! suggestion bar, and a tap replaces the `:smil` with the emoji. The names
//! come from an [`EmojiList`] (the bundled one is GitHub's gemoji).

use super::{Kind, Suggestion, TextEdit};
use crate::dictionaries::EmojiList;

/// Fewest characters after the `:` that bring up emoji, so a lone `:` (or
/// `:` and one letter, as in a smiley `:D`) leaves the word suggestions.
const MIN_QUERY: usize = 2;

/// Most characters a shortcode query runs to; longer is not one.
const MAX_QUERY: usize = 32;

/// The shortcode being typed at the end of `before` (the text before the
/// cursor): the characters after a `:` that starts a token, if there are
/// [`MIN_QUERY`] to [`MAX_QUERY`] of them and all may appear in a
/// shortcode (ASCII letters, digits, `_`, `+`, `-`). A `:` right after a
/// letter, digit or other `:` starts none, so `10:30`, `a:b` and `http://`
/// don't bring up emoji.
pub(crate) fn shortcode_query(before: &str) -> Option<&str> {
    let colon = before.rfind(':')?;
    let query = &before[colon + 1..];
    let len = query.chars().count();
    if !(MIN_QUERY..=MAX_QUERY).contains(&len) || !query.chars().all(is_shortcode_char) {
        return None;
    }
    let starts_token = before[..colon]
        .chars()
        .next_back()
        .is_none_or(|c| !(c.is_alphanumeric() || c == ':'));
    starts_token.then_some(query)
}

fn is_shortcode_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, '_' | '+' | '-')
}

/// The suggestions for the shortcode `query` from `list`, best first, at
/// most `max`: each replaces the `:` and `query` right before the cursor
/// with its emoji.
pub(crate) fn suggestions(list: &EmojiList, query: &str, max: usize) -> Vec<Suggestion> {
    list.search(query, max)
        .into_iter()
        .map(|emoji| Suggestion {
            label: emoji.to_string(),
            kind: Kind::Emoji,
            edit: TextEdit {
                delete_before: (query.len() + 1) as u32,
                delete_after: 0,
                insert: emoji.to_string(),
            },
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_colon_starting_a_token_begins_a_query() {
        assert_eq!(shortcode_query(":smil"), Some("smil"));
        assert_eq!(shortcode_query("hi :smil"), Some("smil"));
        assert_eq!(shortcode_query("(:smil"), Some("smil"));
        assert_eq!(shortcode_query(":+1"), Some("+1"));
        assert_eq!(shortcode_query(":thumbs_up"), Some("thumbs_up"));
        assert_eq!(shortcode_query("say :Joy"), Some("Joy"));
    }

    #[test]
    fn what_is_not_a_query() {
        for before in [
            "", ":", ":s", "smile", "10:30", "a:b", "http://x", "::smile", ":smile ", ":smile:",
            ":smi le", ":café",
        ] {
            assert_eq!(shortcode_query(before), None, "{before:?}");
        }
        let long = format!(":{}", "a".repeat(MAX_QUERY + 1));
        assert_eq!(shortcode_query(&long), None);
    }

    fn bundled() -> EmojiList {
        EmojiList::bundled("gemoji").unwrap()
    }

    fn labels(s: &[Suggestion]) -> Vec<&str> {
        s.iter().map(|s| s.label.as_str()).collect()
    }

    #[test]
    fn real_shortcodes_rank_well() {
        let list = bundled();
        assert_eq!(labels(&suggestions(&list, "smile", 3))[0], "😄");
        assert_eq!(labels(&suggestions(&list, "thumbsup", 3))[0], "👍");
        // "smile", then "smiley".
        assert_eq!(labels(&suggestions(&list, "smil", 3))[..2], ["😄", "😃"]);
        // "laugh" is a keyword of 😄 and the start of 😆's "laughing".
        let s = suggestions(&list, "laugh", 3);
        assert_eq!(labels(&s)[0], "😆");
        assert!(labels(&s).contains(&"😄"), "{s:?}");
        assert!(suggestions(&list, "zzzqx", 3).is_empty());
    }

    #[test]
    fn a_suggestion_replaces_the_colon_and_query() {
        let s = suggestions(&bundled(), "smil", 3);
        assert_eq!(
            s[0].edit,
            TextEdit {
                delete_before: 5,
                delete_after: 0,
                insert: "😄".into()
            }
        );
        assert_eq!(s[0].kind, Kind::Emoji);
    }
}
