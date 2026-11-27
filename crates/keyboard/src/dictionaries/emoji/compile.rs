//! Compiling an emoji list in GitHub gemoji's `db/emoji.json` format into
//! the compact, sorted form [`EmojiList`](super::EmojiList) searches in
//! place. `build.rs` runs this on the bundled list, so the device embeds it
//! ready to search instead of parsing JSON and sorting at startup; a list
//! loaded from a path is compiled at runtime. Shared with `build.rs`, so
//! std and serde only.
//!
//! The format: a JSON array of objects, one per emoji, in Unicode's order.
//! Each has the `emoji` itself, a `description` ("grinning face with
//! smiling eyes"), its shortcodes (`aliases`: `smile`) and `tags` (`happy`,
//! `joy`); other fields are ignored.
//!
//! Each emoji is found by names of three kinds ([`SHORTCODE`],
//! [`SHORTCODE_WORD`], [`KEYWORD`]), all lowercase. The names go in one
//! index sorted by name, so the names starting with what was typed are one
//! binary search away.

use serde::Deserialize;

/// Bytes per emoji: its offset in the text (`u32`, little endian), its
/// length in bytes (`u8`) and three bytes of padding.
pub const EMOJI_LEN: usize = 8;
/// Bytes per name: its offset in the text (`u32`, little endian), its
/// length in bytes (`u8`), its kind (`u8`) and the emoji's index (`u16`,
/// little endian).
pub const NAME_LEN: usize = 8;

/// Name kind: a shortcode (`slightly_smiling_face`).
pub const SHORTCODE: u8 = 0;
/// Name kind: a later word of a shortcode (`smiling`, `face`).
pub const SHORTCODE_WORD: u8 = 1;
/// Name kind: a tag or a word of the description (`happy`).
pub const KEYWORD: u8 = 2;

/// Description words that say nothing about an emoji.
const STOPWORDS: &[&str] = &[
    "with", "and", "of", "the", "in", "on", "for", "at", "or", "to", "an",
];

/// A compiled emoji list.
pub struct Compiled {
    /// Every emoji and name, back to back.
    pub text: String,
    /// One [`EMOJI_LEN`]-byte entry per emoji, in the list's order.
    pub emoji: Vec<u8>,
    /// One [`NAME_LEN`]-byte entry per name of an emoji, sorted by name,
    /// then kind, then emoji. An emoji has each name once, of its best
    /// kind.
    pub names: Vec<u8>,
}

/// One entry of the JSON list.
#[derive(Deserialize)]
struct Entry {
    emoji: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    aliases: Vec<String>,
    #[serde(default)]
    tags: Vec<String>,
}

/// Compile `json`. `None` if it is not a gemoji list. Entries without an
/// emoji or a shortcode, emoji over 255 bytes, names over 255 bytes and
/// emoji past the 65536th are skipped.
pub fn compile(json: &str) -> Option<Compiled> {
    let entries: Vec<Entry> = serde_json::from_str(json).ok()?;
    let mut text = String::new();
    let mut emoji = Vec::new();
    // (name, kind, emoji index) per name.
    let mut names: Vec<(String, u8, u16)> = Vec::new();
    for e in entries {
        let (Ok(start), Ok(len), Ok(index)) = (
            u32::try_from(text.len()),
            u8::try_from(e.emoji.len()),
            u16::try_from(emoji.len() / EMOJI_LEN),
        ) else {
            continue;
        };
        if e.emoji.is_empty() || e.aliases.is_empty() {
            continue;
        }
        text.push_str(&e.emoji);
        emoji.extend_from_slice(&start.to_le_bytes());
        emoji.extend_from_slice(&[len, 0, 0, 0]);
        for (name, kind) in names_of(&e) {
            names.push((name, kind, index));
        }
    }
    // Each name once per emoji, of its best kind: sorting by (name, emoji,
    // kind) puts it first.
    names.sort_unstable_by(|a, b| (&a.0, a.2, a.1).cmp(&(&b.0, b.2, b.1)));
    names.dedup_by(|dup, kept| dup.0 == kept.0 && dup.2 == kept.2);
    names.sort_unstable();
    let mut bytes = Vec::with_capacity(names.len() * NAME_LEN);
    // The text holds each name once: sorted, the entries of a name shared
    // by several emoji ("face") are neighbours and point at one copy.
    let mut last: Option<(String, u32)> = None;
    for (name, kind, index) in names {
        let (Ok(here), Ok(len)) = (u32::try_from(text.len()), u8::try_from(name.len())) else {
            continue;
        };
        let start = match &last {
            Some((prev, start)) if *prev == name => *start,
            _ => {
                text.push_str(&name);
                last = Some((name, here));
                here
            }
        };
        bytes.extend_from_slice(&start.to_le_bytes());
        bytes.push(len);
        bytes.push(kind);
        bytes.extend_from_slice(&index.to_le_bytes());
    }
    Some(Compiled {
        text,
        emoji,
        names: bytes,
    })
}

/// Every name `e` is found by, lowercase, with its kind: its shortcodes,
/// their later words, its tags (spaces as `_`, like a shortcode) and the
/// words of its description. One-letter words and [`STOPWORDS`] are left
/// out; a shortcode is kept whatever it is (`a`, `+1`).
fn names_of(e: &Entry) -> Vec<(String, u8)> {
    let mut out = Vec::new();
    for alias in &e.aliases {
        let alias = alias.to_lowercase();
        for word in alias.split('_').skip(1) {
            push_word(&mut out, word, SHORTCODE_WORD);
        }
        if !alias.is_empty() {
            out.push((alias, SHORTCODE));
        }
    }
    for tag in &e.tags {
        push_word(&mut out, &tag.to_lowercase().replace(' ', "_"), KEYWORD);
    }
    let description = e.description.to_lowercase();
    for word in description.split(|c: char| !(c.is_ascii_alphanumeric() || c == '+')) {
        push_word(&mut out, word, KEYWORD);
    }
    out
}

fn push_word(out: &mut Vec<(String, u8)>, word: &str, kind: u8) {
    if word.chars().count() > 1 && !STOPWORDS.contains(&word) {
        out.push((word.to_string(), kind));
    }
}
