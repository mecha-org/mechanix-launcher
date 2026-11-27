//! Compiling a word list in AOSP LatinIME's "combined" text format into the
//! compact, sorted form [`Wordlist`](super::Wordlist) searches in place.
//! `build.rs` runs this on the bundled lists, so the device embeds them
//! ready to search instead of parsing and sorting at startup (as LatinIME
//! compiles its lists to binary `.dict` files); a list loaded from a path
//! is compiled at runtime. Shared with `build.rs`, so std only.
//!
//! The format, as LatinIME's `dictionaries/sample.combined` documents it: a
//! header line, then one ` word=…,f=…` line per word (`f` a log-scale
//! frequency, 0-255), each optionally followed by indented `  shortcut=…`
//! and `  bigram=…` lines. `f=0` marks a word that is real but never
//! suggested; `not_a_word=true` an entry that only carries a shortcut; a
//! shortcut with `f=whitelist` replaces its word outright.

/// Bytes per entry: the word's offset in the text (`u32`, little endian),
/// its length in bytes (`u8`), its frequency (`u8`), its flags (`u8`) and
/// one byte of padding.
pub const ENTRY_LEN: usize = 8;
/// Entry flag: the word may be suggested (not profanity, not offensive,
/// not a shortcut-only entry).
pub const SUGGEST: u8 = 1;
/// Entry flag: the entry is a word at all (not a shortcut-only entry).
pub const REAL: u8 = 2;

/// A compiled word list.
pub struct Compiled {
    /// Every word, back to back.
    pub text: String,
    /// One [`ENTRY_LEN`]-byte entry per word, sorted by word [`folded`],
    /// then most frequent first.
    pub entries: Vec<u8>,
    /// Whitelisted replacements, one `from<TAB>to` line each, `from`
    /// lowercased ([`folded`]).
    pub replacements: String,
}

/// Compile combined text. Comments, the header, bigrams, shortcuts that are
/// not whitelisted, malformed lines and words over 255 bytes are skipped.
pub fn compile(combined: &str) -> Compiled {
    let mut text = String::new();
    // (start, len, f, flags) per word.
    let mut entries: Vec<(u32, u8, u8, u8)> = Vec::new();
    let mut replacements = String::new();
    // The word the shortcut lines that follow belong to.
    let mut last: Option<&str> = None;
    for line in combined.lines() {
        if let Some(rest) = line.strip_prefix(" word=") {
            last = None;
            let mut fields = rest.split(',');
            let Some(word) = fields.next().filter(|w| !w.is_empty()) else {
                continue;
            };
            let (mut f, mut real, mut offensive) = (None, true, false);
            for (key, value) in fields.filter_map(|field| field.split_once('=')) {
                match key {
                    "f" => f = value.parse::<u8>().ok(),
                    "not_a_word" => real = value != "true",
                    "possibly_offensive" => offensive |= value == "true",
                    "flags" => offensive |= value.split(':').any(|f| f == "offensive"),
                    _ => {}
                }
            }
            let (Some(f), Ok(start), Ok(len)) =
                (f, u32::try_from(text.len()), u8::try_from(word.len()))
            else {
                continue;
            };
            let suggest = real && f > 0 && !offensive;
            let flags = if suggest { SUGGEST } else { 0 } | if real { REAL } else { 0 };
            text.push_str(word);
            entries.push((start, len, f, flags));
            last = Some(word);
        } else if let Some(rest) = line.strip_prefix("  shortcut=")
            && let Some(from) = last
        {
            let mut fields = rest.split(',');
            if let Some(to) = fields.next()
                && fields.any(|field| field == "f=whitelist")
            {
                replacements.extend(folded(from));
                replacements.push('\t');
                replacements.push_str(to);
                replacements.push('\n');
            }
        }
    }
    // Folded once per word, not at every comparison.
    entries.sort_by_cached_key(|&(start, len, f, _)| {
        let word = &text[start as usize..start as usize + len as usize];
        (folded(word).collect::<String>(), std::cmp::Reverse(f))
    });
    let mut bytes = Vec::with_capacity(entries.len() * ENTRY_LEN);
    for (start, len, f, flags) in entries {
        bytes.extend_from_slice(&start.to_le_bytes());
        bytes.extend_from_slice(&[len, f, flags, 0]);
    }
    Compiled {
        text,
        entries: bytes,
        replacements,
    }
}

/// `s` lowercased, a char at a time, without allocating.
pub fn folded(s: &str) -> impl Iterator<Item = char> + Clone + '_ {
    s.chars().flat_map(char::to_lowercase)
}
