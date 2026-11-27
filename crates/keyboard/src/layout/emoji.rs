//! Colour emoji from the bundled Noto Color Emoji font. fontdue draws only
//! outlines, so each emoji the layout uses is cut out of the font's `SVG `
//! table at load and inserted into the atlas as a full-colour SVG image,
//! which the atlas re-rasterizes on every scale change.
//!
//! The font's SVG documents are nanoemoji output: some hold one glyph, others
//! are shared by thousands (one is 14 MB). A glyph is a `<g id="glyphN">` of
//! `<path>`s and `<use>`s referring by id to top-level children of the one
//! `<defs>`. Cutting one out keeps that `<g>` and only the defs it reaches.
//! A sequence (keycap, flag, skin tone, ZWJ) is found through its GSUB
//! ligature.

use mecha_wayland::prelude::{Atlas, Resource, SpriteId};
use std::borrow::Cow;
use std::collections::{HashMap, HashSet};
use tracing::warn;
use ttf_parser::gsub::SubstitutionSubtable;
use ttf_parser::{Face, GlyphId};

static NOTO_COLOR_EMOJI: &[u8] = include_bytes!("../../resources/fonts/NotoColorEmoji-Regular.ttf");

/// Variation selector 16: asks for emoji presentation; draws nothing itself.
const VS16: char = '\u{FE0F}';
/// Zero-width joiner: glues emoji into one sequence (`👨‍💻`).
const ZWJ: char = '\u{200D}';
/// 🤝: a modifier base by itself, but left untoned inside a multi-person
/// sequence (`🧑‍🤝‍🧑`), whose people take the tone.
const HANDSHAKE: char = '\u{1F91D}';

/// The five Fitzpatrick skin-tone modifiers, light to dark.
pub(crate) const SKIN_TONES: [char; 5] = [
    '\u{1F3FB}',
    '\u{1F3FC}',
    '\u{1F3FD}',
    '\u{1F3FE}',
    '\u{1F3FF}',
];

/// One loaded emoji: its sprite and the logical size it draws at.
#[derive(Debug, Clone, Copy)]
pub struct EmojiSprite {
    pub sprite: SpriteId,
    pub width: f32,
    pub height: f32,
}

/// The emoji sprites, keyed by the emoji string: those the layout uses,
/// loaded up front, and any others [`ensure`](Emoji::ensure)d since (the
/// suggestion bar's).
#[derive(Default)]
pub struct Emoji {
    sprites: HashMap<String, EmojiSprite>,
    /// Logical px of each sprite's longer side.
    size: u32,
    /// The font, kept (with the SVG documents it has indexed) to load
    /// more; `None` until something needs it.
    font: Option<EmojiFont>,
    /// Emoji the font can't draw, so they are not tried again.
    missing: HashSet<String>,
    /// The skin tone picked last (an index into [`SKIN_TONES`]), applied
    /// to every emoji that takes one; `None` for the default yellow. Kept in
    /// memory only.
    skin_tone: Option<usize>,
    /// Per emoji asked about: its five toned forms, or `None` if it takes
    /// no skin tone.
    tones: HashMap<String, Option<[String; 5]>>,
}

impl std::fmt::Debug for Emoji {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Emoji")
            .field("sprites", &self.sprites)
            .field("size", &self.size)
            .finish_non_exhaustive()
    }
}

impl Resource for Emoji {}

impl Emoji {
    /// Insert every emoji in `emoji` into the atlas, its longer side `size`
    /// logical px. One the font can't draw warns and is left out.
    pub fn load<'a>(
        atlas: &mut Atlas,
        emoji: impl IntoIterator<Item = &'a str>,
        size: u32,
    ) -> Self {
        let mut this = Self {
            size,
            ..Self::default()
        };
        for e in emoji {
            if this.ensure(atlas, e).is_none() {
                warn!("no colour glyph for emoji {e:?}; its key draws empty");
            }
        }
        this
    }

    /// The sprite for `emoji`, inserting it into the atlas first if it is
    /// not there yet. `None` if the font can't draw it. The first emoji
    /// from one of the font's large shared SVG documents takes a while (the
    /// document is indexed once); the rest are quick.
    pub fn ensure(&mut self, atlas: &mut Atlas, emoji: &str) -> Option<EmojiSprite> {
        if let Some(s) = self.sprites.get(emoji) {
            return Some(*s);
        }
        if self.missing.contains(emoji) {
            return None;
        }
        let svg = self.font.get_or_insert_with(EmojiFont::new).svg(emoji);
        let loaded =
            match svg.map(|svg| atlas.insert_svg_image(Cow::Owned(svg.into_bytes()), self.size)) {
                Some(Ok(sprite)) => {
                    let s = atlas.sprite(sprite).size;
                    Some(EmojiSprite {
                        sprite,
                        width: s.width,
                        height: s.height,
                    })
                }
                Some(Err(err)) => {
                    warn!("emoji {emoji:?} does not load: {err:?}");
                    None
                }
                None => None,
            };
        match loaded {
            Some(s) => {
                self.sprites.insert(emoji.to_owned(), s);
            }
            None => {
                self.missing.insert(emoji.to_owned());
            }
        }
        loaded
    }

    /// The sprite for `emoji`, or `None` if it wasn't loaded.
    pub fn get(&self, emoji: &str) -> Option<EmojiSprite> {
        self.sprites.get(emoji).copied()
    }

    /// The skin tone picked last, as an index into [`SKIN_TONES`]; `None`
    /// for the default.
    pub(crate) fn skin_tone(&self) -> Option<usize> {
        self.skin_tone
    }

    /// Pick skin tone `tone` (an index into [`SKIN_TONES`], `None` for the
    /// default) for every emoji that takes one.
    pub(crate) fn set_skin_tone(&mut self, tone: Option<usize>) {
        self.skin_tone = tone.filter(|&t| t < SKIN_TONES.len());
    }

    /// The five toned forms of `emoji`, light to dark, if it takes a skin
    /// tone and the font draws every form. Worked out once per emoji.
    pub(crate) fn tones(&mut self, emoji: &str) -> Option<[String; 5]> {
        if let Some(t) = self.tones.get(emoji) {
            return t.clone();
        }
        let font = self.font.get_or_insert_with(EmojiFont::new);
        let forms: Option<Vec<String>> = SKIN_TONES.iter().map(|&t| font.toned(emoji, t)).collect();
        let forms = forms.and_then(|f| <[String; 5]>::try_from(f).ok());
        self.tones.insert(emoji.to_owned(), forms.clone());
        forms
    }

    /// `emoji` in the skin tone picked last, if one is and `emoji` takes
    /// it; else `emoji` as it is.
    pub(crate) fn toned(&mut self, emoji: &str) -> String {
        match (self.skin_tone, self.tones(emoji)) {
            (Some(t), Some(forms)) => forms[t].clone(),
            _ => emoji.to_owned(),
        }
    }
}

/// One SVG document's top-level elements: `<defs>` children by id, and
/// glyph `<g>`s by glyph id.
#[derive(Default)]
struct DocIndex {
    defs: HashMap<&'static str, &'static str>,
    glyphs: HashMap<u16, &'static str>,
}

/// The parsed font, with each shared SVG document indexed once.
struct EmojiFont {
    face: Face<'static>,
    /// Per SVG document, keyed by its start address.
    docs: HashMap<usize, DocIndex>,
}

impl EmojiFont {
    fn new() -> Self {
        Self {
            face: Face::parse(NOTO_COLOR_EMOJI, 0).expect("bundled Noto Color Emoji parses"),
            docs: HashMap::new(),
        }
    }

    /// A standalone SVG for `emoji`: one codepoint, or a sequence (keycap,
    /// flag, skin tone, ZWJ) the font ligates to a single glyph.
    fn svg(&mut self, emoji: &str) -> Option<String> {
        let glyph = self.glyph(emoji)?;
        self.glyph_svg(glyph)
    }

    /// The one glyph `emoji` draws as. A VS16 only selects presentation, so
    /// it is tried with and then without; the whole sequence must ligate.
    fn glyph(&self, emoji: &str) -> Option<GlyphId> {
        let all: Option<Vec<GlyphId>> = emoji.chars().map(|c| self.face.glyph_index(c)).collect();
        let bare: Option<Vec<GlyphId>> = emoji
            .chars()
            .filter(|&c| c != VS16)
            .map(|c| self.face.glyph_index(c))
            .collect();
        [all, bare]
            .into_iter()
            .flatten()
            .find_map(|glyphs| match glyphs[..] {
                [] => None,
                [glyph] => Some(glyph),
                _ => self.ligature(&glyphs),
            })
    }

    /// `emoji` with skin tone `tone` after each of its modifier bases (any
    /// VS16 after one dropped), if it has one, has no tone yet, and the
    /// font draws the result. A base is a codepoint the font ligates with
    /// a tone; in a ZWJ sequence 🤝 is not one (its people take the tone).
    fn toned(&self, emoji: &str, tone: char) -> Option<String> {
        if emoji.chars().any(|c| SKIN_TONES.contains(&c)) {
            return None;
        }
        let joined = emoji.contains(ZWJ);
        let mut out = String::with_capacity(emoji.len() + 8);
        let mut any = false;
        let mut chars = emoji.chars().peekable();
        while let Some(c) = chars.next() {
            out.push(c);
            if !(joined && c == HANDSHAKE) && self.takes_tone(c, tone) {
                out.push(tone);
                any = true;
                chars.next_if_eq(&VS16);
            }
        }
        (any && self.glyph(&out).is_some()).then_some(out)
    }

    /// Whether the font ligates `c` followed by skin tone `tone`.
    fn takes_tone(&self, c: char, tone: char) -> bool {
        match (self.face.glyph_index(c), self.face.glyph_index(tone)) {
            (Some(a), Some(b)) => self.ligature(&[a, b]).is_some(),
            _ => false,
        }
    }

    /// The glyph a GSUB ligature substitutes for exactly `glyphs`.
    fn ligature(&self, glyphs: &[GlyphId]) -> Option<GlyphId> {
        let (first, rest) = glyphs.split_first()?;
        let gsub = self.face.tables().gsub?;
        for lookup in gsub.lookups {
            for i in 0..lookup.subtables.len() {
                let Some(SubstitutionSubtable::Ligature(lig)) = lookup.subtables.get(i) else {
                    continue;
                };
                let Some(set) = lig
                    .coverage
                    .get(*first)
                    .and_then(|i| lig.ligature_sets.get(i))
                else {
                    continue;
                };
                for l in set {
                    if l.components.into_iter().eq(rest.iter().copied()) {
                        return Some(l.glyph);
                    }
                }
            }
        }
        None
    }

    fn glyph_svg(&mut self, glyph: GlyphId) -> Option<String> {
        let doc = self.face.glyph_svg_image(glyph)?.data;
        // A gzip-compressed document; Noto doesn't use them.
        if doc.starts_with(&[0x1f, 0x8b]) {
            return None;
        }
        let doc: &'static str = std::str::from_utf8(doc).ok()?;
        let index = self
            .docs
            .entry(doc.as_ptr() as usize)
            .or_insert_with(|| index_doc(doc));
        let defs = &index.defs;
        let body = *index.glyphs.get(&glyph.0)?;

        // Every def the glyph reaches, in document order.
        let mut seen = HashSet::new();
        let mut stack: Vec<&str> = refs(body).collect();
        while let Some(id) = stack.pop() {
            if let Some(def) = defs.get(id)
                && seen.insert(id)
            {
                stack.extend(refs(def));
            }
        }
        let mut used: Vec<&str> = seen.iter().filter_map(|id| defs.get(id).copied()).collect();
        used.sort_by_key(|d| d.as_ptr() as usize);

        // The glyph's em box: x across its advance, y from the ascender
        // down to the descender (SVG glyphs draw y-down from the baseline).
        let advance = self.face.glyph_hor_advance(glyph)?;
        let ascender = self.face.ascender();
        let height = ascender - self.face.descender();
        Some(format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" \
             xmlns:xlink=\"http://www.w3.org/1999/xlink\" version=\"1.1\" \
             viewBox=\"0 {} {advance} {height}\"><defs>{}</defs>{body}</svg>",
            -ascender,
            used.concat(),
        ))
    }
}

/// Index `doc` in one pass: the children of its `<defs>`, then the glyphs
/// after it, both top-level and each read whole.
fn index_doc(doc: &'static str) -> DocIndex {
    let mut index = DocIndex::default();
    let Some(open) = doc.find('>') else {
        return index;
    };
    // Past the `<svg …>` start tag.
    let mut rest = &doc[open + 1..];
    let mut in_defs = false;
    while let Some(lt) = rest.find('<') {
        rest = &rest[lt..];
        if rest.starts_with("<defs>") {
            in_defs = true;
            rest = &rest["<defs>".len()..];
            continue;
        }
        if rest.starts_with("</defs>") {
            in_defs = false;
            rest = &rest["</defs>".len()..];
            continue;
        }
        if rest.starts_with("</") {
            break;
        }
        let Some(el) = element_at(rest) else { break };
        if let Some(id) = attr(el, "id") {
            if in_defs {
                index.defs.insert(id, el);
            } else if let Some(gid) = id.strip_prefix("glyph").and_then(|n| n.parse().ok()) {
                index.glyphs.insert(gid, el);
            }
        }
        rest = &rest[el.len()..];
    }
    index
}

/// The element `s` starts with: self-closing, or up to its matching end tag.
fn element_at(s: &str) -> Option<&str> {
    let name_end = s[1..].find([' ', '>', '/'])? + 1;
    let name = &s[1..name_end];
    let tag_end = s.find('>')? + 1;
    if s[..tag_end].ends_with("/>") {
        return Some(&s[..tag_end]);
    }
    let (open, close) = (format!("<{name}"), format!("</{name}>"));
    let mut depth = 1;
    let mut i = tag_end;
    loop {
        let next_close = s[i..].find(&close)? + i;
        let next_open = s[i..next_close]
            .match_indices(&open)
            .map(|(j, _)| j + i)
            // `<g` must not match `<glyph…`-like names.
            .find(|&j| matches!(s.as_bytes().get(j + open.len()), Some(b' ' | b'>' | b'/')));
        match next_open {
            Some(j) => {
                // A nested element of the same name: self-closing or not.
                let end = s[j..].find('>')? + j;
                if !s[j..=end].ends_with("/>") {
                    depth += 1;
                }
                i = end + 1;
            }
            None => {
                depth -= 1;
                i = next_close + close.len();
                if depth == 0 {
                    return Some(&s[..i]);
                }
            }
        }
    }
}

/// The value of attribute `name` in `el`'s start tag.
fn attr<'a>(el: &'a str, name: &str) -> Option<&'a str> {
    let tag = &el[..el.find('>')?];
    let pat = format!(" {name}=\"");
    let start = tag.find(&pat)? + pat.len();
    Some(&tag[start..start + tag[start..].find('"')?])
}

/// The ids `s` refers to: `href="#id"` and `url(#id)`.
fn refs(s: &str) -> impl Iterator<Item = &str> {
    s.match_indices("#").filter_map(|(i, _)| {
        let before = &s[..i];
        let end = if before.ends_with("href=\"") {
            '"'
        } else if before.ends_with("url(") {
            ')'
        } else {
            return None;
        };
        let rest = &s[i + 1..];
        Some(&rest[..rest.find(end)?])
    })
}

/// Whether the bundled font can draw `emoji`.
#[cfg(test)]
pub(crate) fn can_draw(emoji: &str) -> bool {
    EmojiFont::new().svg(emoji).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn element_matches_nested_tags() {
        let doc = r#"<g id="a"><g x="1"><path/></g><g/></g><g id="b"/>"#;
        assert_eq!(element_at(doc), Some(&doc[..38]));
        assert_eq!(element_at(&doc[38..]), Some("<g id=\"b\"/>"));
    }

    #[test]
    fn refs_finds_hrefs_and_urls() {
        let s = r##"<use xlink:href="#p1" fill="url(#g2)"/><path fill="#FFF"/>"##;
        assert_eq!(refs(s).collect::<Vec<_>>(), ["p1", "g2"]);
    }

    #[test]
    fn single_codepoint_emoji_draw() {
        assert!(can_draw("😂"), "from a shared document");
        assert!(can_draw("🔥"), "from its own document");
        assert!(can_draw("❤️"), "a trailing VS16 is fine");
        assert!(!can_draw("a"), "letters have no colour glyph");
    }

    #[test]
    fn sequences_ligate_to_one_glyph() {
        assert!(can_draw("1️⃣"), "keycap");
        assert!(can_draw("👍🏽"), "skin tone");
        assert!(can_draw("🇮🇳"), "flag");
        assert!(can_draw("👨‍👩‍👧"), "ZWJ family");
        assert!(!can_draw("😂😂"), "two emoji are not one glyph");
    }

    /// A cut-out from the 14 MB shared document keeps only what it uses,
    /// and every reference in it resolves.
    #[test]
    fn cut_out_glyph_is_small_and_self_contained() {
        let svg = EmojiFont::new().svg("😂").unwrap();
        assert!(svg.len() < 64 * 1024, "{} bytes", svg.len());
        for id in refs(&svg) {
            assert!(svg.contains(&format!("id=\"{id}\"")), "dangling #{id}");
        }
    }

    /// Every emoji key in the bundled layout has a colour glyph.
    #[test]
    fn bundled_layout_emoji_all_draw() {
        let keymap = crate::layout::load_keymap();
        let emoji: Vec<_> = keymap
            .views
            .iter()
            .flat_map(|v| v.keys())
            .filter_map(|k| match &k.face {
                crate::layout::KeyFace::Emoji(e) => Some(e.as_str()),
                _ => None,
            })
            .collect();
        assert!(emoji.len() >= 20, "the emoji page has its emoji");
        let mut font = EmojiFont::new();
        let missing: Vec<_> = emoji.iter().filter(|e| font.svg(e).is_none()).collect();
        assert!(missing.is_empty(), "no colour glyph for {missing:?}");
    }

    #[test]
    fn ensure_loads_emoji_not_in_the_layout_once() {
        let mut atlas = Atlas::new();
        let mut emoji = Emoji::load(&mut atlas, ["😂"], 28);
        assert!(emoji.get("🍕").is_none());
        let pizza = emoji.ensure(&mut atlas, "🍕").unwrap();
        assert_eq!(pizza.width.max(pizza.height), 28.0);
        assert_eq!(emoji.get("🍕").unwrap().sprite, pizza.sprite);
        assert_eq!(emoji.ensure(&mut atlas, "🍕").unwrap().sprite, pizza.sprite);
        assert!(emoji.ensure(&mut atlas, "a").is_none());
        assert!(emoji.missing.contains("a"));
    }

    /// Every emoji the suggestion bar can offer (the bundled gemoji list)
    /// has a colour glyph in the bundled font.
    #[test]
    fn every_suggestable_emoji_draws() {
        let list = crate::dictionaries::EmojiList::bundled("gemoji").unwrap();
        let mut font = EmojiFont::new();
        let missing: Vec<_> = list.all().filter(|e| font.svg(e).is_none()).collect();
        assert!(missing.is_empty(), "no colour glyph for {missing:?}");
    }

    #[test]
    fn skin_tones_apply_to_modifier_bases_only() {
        let mut emoji = Emoji::default();
        let thumbs = emoji.tones("👍").expect("👍 takes a tone");
        assert_eq!(thumbs[2], "👍🏽");
        assert_eq!(thumbs[4], "👍🏿");
        assert_eq!(emoji.tones("☝️").unwrap()[0], "☝🏻", "VS16 dropped");
        assert_eq!(emoji.tones("🏃‍♀️").unwrap()[2], "🏃🏽‍♀️", "ZWJ sequence");
        assert!(emoji.tones("😀").is_none(), "faces take none");
        assert!(emoji.tones("❤️").is_none());
        assert!(emoji.tones("👍🏽").is_none(), "already toned");
        assert!(emoji.tones("a").is_none());
    }

    #[test]
    fn toned_follows_the_picked_skin_tone() {
        let mut emoji = Emoji::default();
        assert_eq!(emoji.toned("👍"), "👍");
        emoji.set_skin_tone(Some(3));
        assert_eq!(emoji.toned("👍"), "👍🏾");
        assert_eq!(emoji.toned("😀"), "😀");
        emoji.set_skin_tone(Some(9));
        assert_eq!(emoji.skin_tone(), None, "out of range is the default");
        assert_eq!(emoji.toned("👍"), "👍");
    }

    /// Every toneable emoji in the bundled layout and gemoji list draws in
    /// all five tones (what `tones` checks), and the people page has some.
    #[test]
    fn bundled_toneable_emoji_draw_in_every_tone() {
        let mut emoji = Emoji::default();
        let keymap = crate::layout::load_keymap();
        let toneable = keymap
            .views
            .iter()
            .flat_map(|v| v.keys())
            .filter(
                |k| matches!(&k.face, crate::layout::KeyFace::Emoji(e) if emoji.tones(e).is_some()),
            )
            .count();
        assert!(toneable >= 20, "{toneable} toneable layout emoji");
        let list = crate::dictionaries::EmojiList::bundled("gemoji").unwrap();
        let toneable = list.all().filter(|e| emoji.tones(e).is_some()).count();
        assert!(toneable >= 250, "{toneable} toneable gemoji");
    }

    #[test]
    fn loaded_emoji_are_sized_and_deduplicated() {
        let mut atlas = Atlas::new();
        let emoji = Emoji::load(&mut atlas, ["😂", "😂", "🔥", "a"], 28);
        assert_eq!(emoji.sprites.len(), 2);
        let e = emoji.get("😂").unwrap();
        assert_eq!(e.width.max(e.height), 28.0);
        assert!(e.width.min(e.height) > 20.0);
        assert!(emoji.get("a").is_none());
    }
}
