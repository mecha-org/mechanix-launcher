//! Offline noisy-typing simulation used to tune the prediction defaults.
//!
//! Letters are tapped at key centres plus Gaussian noise; the resolver's
//! error rate with and without the dictionary prior shows what the prediction
//! strength buys on real words and costs on random strings.

use mecha_wayland::prelude::*;
use xkbcommon::xkb::Keysym;

use super::resolve::{KeyGeom, ResolveParams, SIGMA_RATIO, resolve};
use crate::layout::{DEFAULT_STRENGTH, KeyAction, Keymap, load_keymap};
use crate::predict::{Dictionary, Predictor};
use crate::protocol::virtual_keyboard::keysym_text;

/// Tap noise as a fraction of the median letter key width.
const NOISE: f32 = 0.35;
const EN_TSV: &str = include_str!("../../resources/dict/en.tsv");

/// Approximate flex layout of the `base` view: equal-height rows, key widths
/// proportional to `Key.width` after subtracting gaps.
fn base_geometry(keymap: &Keymap) -> Vec<KeyGeom> {
    let kb = &keymap.keyboard;
    let view = keymap.view("base").expect("base view");
    let rows = view.rows.len() as f32;
    let row_h = (kb.height - 2.0 * kb.padding - (rows - 1.0) * kb.row_gap) / rows;
    let row_w = kb.width.unwrap_or(540.0) - 2.0 * kb.padding;
    let mut out = Vec::new();
    for (r, row) in view.rows.iter().enumerate() {
        let n = row.keys().count() as f32;
        let total: f32 = row.keys().map(|k| k.width).sum();
        let avail = row_w - (n - 1.0) * kb.gap;
        let y = kb.padding + r as f32 * (row_h + kb.row_gap);
        let mut x = kb.padding;
        for key in row.keys() {
            let w = key.width / total * avail;
            let letter = match &key.action {
                KeyAction::EmitKeysym(ks) => letter_of(*ks),
                _ => None,
            };
            out.push(KeyGeom {
                rect: Rect::new(x, y, w, row_h),
                letter,
            });
            x += w + kb.gap;
        }
    }
    out
}

fn letter_of(ks: Keysym) -> Option<char> {
    let c = keysym_text(ks)?.chars().next()?;
    c.is_ascii_lowercase().then_some(c)
}

/// xorshift64* with Box-Muller normals.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    /// Uniform in (0, 1].
    fn unit(&mut self) -> f32 {
        ((self.next_u64() >> 40) as f32 + 1.0) / (1u64 << 24) as f32
    }
    fn normal(&mut self) -> f32 {
        let (u, v) = (self.unit(), self.unit());
        (-2.0 * u.ln()).sqrt() * (std::f32::consts::TAU * v).cos()
    }
}

/// Every 10th of the 5000 most frequent bundled words (no apostrophes).
fn words() -> Vec<&'static str> {
    EN_TSV
        .lines()
        .take(5000)
        .filter_map(|l| l.split('\t').next())
        .enumerate()
        .filter(|(i, w)| {
            i % 10 == 0 && !w.contains('\'') && w.chars().all(|c| c.is_ascii_lowercase())
        })
        .map(|(_, w)| w)
        .collect()
}

/// `n` seeded uniform random 5-letter strings.
fn random_strings(n: usize) -> Vec<String> {
    let mut rng = Rng::new(0xC0FFEE);
    (0..n)
        .map(|_| {
            (0..5)
                .map(|_| (b'a' + (rng.next_u64() >> 33) as u8 % 26) as char)
                .collect()
        })
        .collect()
}

/// Tunables varied by the table.
#[derive(Clone, Copy)]
struct Cfg {
    strength: f32,
    core: f32,
    sigma_ratio: f32,
}

impl Cfg {
    fn with_strength(strength: f32) -> Self {
        Self {
            strength,
            core: super::resolve::CORE,
            sigma_ratio: SIGMA_RATIO,
        }
    }
}

fn error_rate_cfg<S: AsRef<str>>(words: &[S], cfg: Cfg, noise: f32, seed: u64) -> f32 {
    let keys = base_geometry(&load_keymap());
    let mut params = ResolveParams::for_keys(&keys, cfg.strength);
    let median = params.sigma / SIGMA_RATIO;
    params.core = cfg.core;
    params.sigma = cfg.sigma_ratio * median;
    let dict = Dictionary::bundled("en").expect("bundled en");
    let mut rng = Rng::new(seed);
    let (mut wrong, mut total) = (0usize, 0usize);
    for word in words {
        let word = word.as_ref();
        for (i, c) in word.chars().enumerate() {
            let target = keys.iter().position(|k| k.letter == Some(c)).unwrap();
            let r = keys[target].rect;
            let p = Point {
                x: r.x() + r.width() / 2.0 + rng.normal() * noise * median,
                y: r.y() + r.height() / 2.0 + rng.normal() * noise * median,
            };
            let prior = dict.prior(&word[..i]);
            if resolve(&keys, p, &prior, &params) != Some(target) {
                wrong += 1;
            }
            total += 1;
        }
    }
    wrong as f32 / total as f32
}

/// Fraction of wrongly resolved letters over `words`.
fn error_rate(words: &[&str], strength: f32, noise: f32, seed: u64) -> f32 {
    error_rate_cfg(words, Cfg::with_strength(strength), noise, seed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base_geometry_tiles_the_keyboard() {
        let km = load_keymap();
        let keys = base_geometry(&km);
        let kb = &km.keyboard;
        let (x0, y0) = (kb.padding, kb.padding);
        let x1 = kb.width.unwrap_or(540.0) - kb.padding;
        let y1 = kb.height - kb.padding;
        for (i, a) in keys.iter().enumerate() {
            let r = a.rect;
            assert!(r.x() >= x0 - 0.01 && r.right() <= x1 + 0.01);
            assert!(r.y() >= y0 - 0.01 && r.bottom() <= y1 + 0.01);
            for b in &keys[i + 1..] {
                let o = b.rect;
                let overlap = r.x() < o.right() - 0.01
                    && o.x() < r.right() - 0.01
                    && r.y() < o.bottom() - 0.01
                    && o.y() < r.bottom() - 0.01;
                assert!(!overlap, "{r:?} overlaps {o:?}");
            }
        }
        let mut letters: Vec<char> = keys.iter().filter_map(|k| k.letter).collect();
        letters.sort();
        letters.dedup();
        assert_eq!(letters.len(), 26);
    }

    #[test]
    fn prediction_lowers_error_on_real_words() {
        let w = words();
        let on = error_rate(&w, DEFAULT_STRENGTH, NOISE, 1);
        let off = error_rate(&w, 0.0, NOISE, 1);
        assert!(on <= 0.85 * off, "on {on} off {off}");
    }

    #[test]
    fn prediction_does_not_hurt_random_strings() {
        let s = random_strings(500);
        let on = error_rate(
            &s.iter().map(String::as_str).collect::<Vec<_>>(),
            DEFAULT_STRENGTH,
            NOISE,
            2,
        );
        let off = error_rate(
            &s.iter().map(String::as_str).collect::<Vec<_>>(),
            0.0,
            NOISE,
            2,
        );
        assert!(on <= 1.10 * off, "on {on} off {off}");
    }

    #[test]
    fn out_of_list_words_typeable_at_centres() {
        let words = ["zbigniew", "qwxz", "mecha", "kubernetes", "jjjj"];
        assert_eq!(error_rate(&words, DEFAULT_STRENGTH * 3.0, 0.0, 3), 0.0);
    }

    #[test]
    #[ignore]
    fn print_strength_table() {
        let w = words();
        let s = random_strings(500);
        let s: Vec<&str> = s.iter().map(String::as_str).collect();
        println!("words: {}", w.len());
        println!("sigma  core strength  words  random");
        for sigma_ratio in [0.2, 0.3, 0.4] {
            for core in [0.4, 0.5, 0.6, 0.7] {
                for strength in [0.0, 0.1, 0.25, 0.5, 1.0, 1.5, 2.0, 3.0] {
                    let cfg = Cfg {
                        strength,
                        core,
                        sigma_ratio,
                    };
                    println!(
                        "{sigma_ratio:.1}  {core:.1}  {strength:.2}  {:.4}  {:.4}",
                        error_rate_cfg(&w, cfg, NOISE, 1),
                        error_rate_cfg(&s, cfg, NOISE, 2),
                    );
                }
            }
        }
    }
}
