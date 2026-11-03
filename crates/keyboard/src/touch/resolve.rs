//! Resolve a touch point to a key, biasing letter keys by a next-letter prior.
//!
//! A touch inside a key's core, or inside a non-letter key, always hits that
//! key. Anywhere else (key edges and gaps) the nearby letter keys compete on
//! a Gaussian falloff in distance weighted by how likely each letter is next.

use mecha_wayland::prelude::*;

use crate::predict::LetterPrior;

/// Fraction of a letter key (width and height, about its centre) that always
/// hits that key regardless of the prior.
pub(crate) const CORE: f32 = 0.5;
/// Gaussian sigma as a fraction of the median letter key width.
pub(crate) const SIGMA_RATIO: f32 = 0.3;
/// Farthest a touch may be pulled to another key, as a fraction of the median
/// letter key width.
pub(crate) const MAX_SHIFT_RATIO: f32 = 0.5;

/// A key's hit rectangle and, for `a`-`z` keys, its lowercase letter.
#[derive(Debug, Clone, Copy)]
pub(crate) struct KeyGeom {
    pub rect: Rect,
    /// Lowercase `a`-`z` only; `None` for every other key.
    pub letter: Option<char>,
}

/// Tunables for [`resolve`].
#[derive(Debug, Clone, Copy)]
pub(crate) struct ResolveParams {
    /// Exponent applied to the prior; 0 disables prediction.
    pub strength: f32,
    /// Fraction of a letter key that always hits it.
    pub core: f32,
    /// Gaussian sigma in pixels.
    pub sigma: f32,
    /// Largest distance in pixels a touch may be pulled to another key.
    pub max_shift: f32,
}

impl ResolveParams {
    /// Parameters scaled to the median letter key width of `keys` (all keys if
    /// none is a letter, 1.0 if there are no keys).
    pub fn for_keys(keys: &[KeyGeom], strength: f32) -> Self {
        let mut widths: Vec<f32> = keys
            .iter()
            .filter(|k| k.letter.is_some())
            .map(|k| k.rect.width())
            .collect();
        if widths.is_empty() {
            widths = keys.iter().map(|k| k.rect.width()).collect();
        }
        let width = if widths.is_empty() {
            1.0
        } else {
            widths.sort_by(|a, b| a.total_cmp(b));
            let n = widths.len();
            if n % 2 == 1 {
                widths[n / 2]
            } else {
                (widths[n / 2 - 1] + widths[n / 2]) / 2.0
            }
        };
        Self {
            strength,
            core: CORE,
            sigma: SIGMA_RATIO * width,
            max_shift: MAX_SHIFT_RATIO * width,
        }
    }
}

/// Euclidean distance from `p` to `r`; 0 when `p` is on or inside it.
fn distance(r: Rect, p: Point) -> f32 {
    let dx = (r.x() - p.x).max(0.0).max(p.x - r.right());
    let dy = (r.y() - p.y).max(0.0).max(p.y - r.bottom());
    dx.hypot(dy)
}

/// `r` scaled by `factor` about its centre.
fn scaled(r: Rect, factor: f32) -> Rect {
    let w = r.width() * factor;
    let h = r.height() * factor;
    Rect::new(
        r.x() + (r.width() - w) / 2.0,
        r.y() + (r.height() - h) / 2.0,
        w,
        h,
    )
}

/// The index of the key a touch at `p` means, or `None` if `keys` is empty.
pub(crate) fn resolve(
    keys: &[KeyGeom],
    p: Point,
    prior: &LetterPrior,
    params: &ResolveParams,
) -> Option<usize> {
    if keys.is_empty() {
        return None;
    }
    if let Some(i) = keys
        .iter()
        .position(|k| k.letter.is_none() && k.rect.contains(p))
    {
        return Some(i);
    }
    if let Some(i) = keys
        .iter()
        .position(|k| k.letter.is_some() && scaled(k.rect, params.core).contains(p))
    {
        return Some(i);
    }

    // Nearest key; strict `<` keeps the lower index on ties.
    let mut nearest = 0;
    let mut nearest_d = f32::INFINITY;
    for (i, k) in keys.iter().enumerate() {
        let d = distance(k.rect, p);
        if d < nearest_d {
            nearest_d = d;
            nearest = i;
        }
    }
    if keys[nearest].letter.is_none() || params.strength == 0.0 {
        return Some(nearest);
    }

    let two_sigma_sq = 2.0 * params.sigma * params.sigma;
    let mut best: Option<(usize, f32)> = None;
    for (i, k) in keys.iter().enumerate() {
        let Some(letter) = k.letter else { continue };
        let d = distance(k.rect, p);
        if d > params.max_shift {
            continue;
        }
        let score = (-d * d / two_sigma_sq).exp()
            * prior[(letter as u8 - b'a') as usize].powf(params.strength);
        if best.is_none_or(|(_, s)| score > s) {
            best = Some((i, score));
        }
    }
    Some(best.map_or(nearest, |(i, _)| i))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::predict::FLAT;

    const W: f32 = 40.0;
    const H: f32 = 44.0;
    const GAP: f32 = 8.0;

    fn letter_index(c: char) -> usize {
        (c as u8 - b'a') as usize
    }

    /// `q w e r t` then a BackSpace key, 40x44 with 8 px gaps.
    fn row() -> Vec<KeyGeom> {
        let mut keys: Vec<KeyGeom> = "qwert"
            .chars()
            .enumerate()
            .map(|(i, c)| KeyGeom {
                rect: Rect::new(i as f32 * (W + GAP), 0.0, W, H),
                letter: Some(c),
            })
            .collect();
        keys.push(KeyGeom {
            rect: Rect::new(5.0 * (W + GAP), 0.0, W, H),
            letter: None,
        });
        keys
    }

    fn params(keys: &[KeyGeom]) -> ResolveParams {
        ResolveParams::for_keys(keys, 1.0)
    }

    #[test]
    fn inside_non_letter_key_is_exact() {
        let keys = row();
        let mut prior = [0.001; 26];
        prior[letter_index('t')] = 1.0;
        // Just inside BackSpace's left edge, 8 px from t.
        let p = Point::new(5.0 * (W + GAP) + 0.5, H / 2.0);
        assert_eq!(resolve(&keys, p, &prior, &params(&keys)), Some(5));
    }

    #[test]
    fn edge_tap_leans_to_likely_letter() {
        let keys = row();
        let mut prior = [0.02; 26];
        prior[letter_index('e')] = 0.5;
        prior[letter_index('r')] = 0.001;
        // 2 px inside r's left edge, next to e.
        let p = Point::new(3.0 * (W + GAP) + 2.0, H / 2.0);
        assert_eq!(resolve(&keys, p, &prior, &params(&keys)), Some(2));
    }

    #[test]
    fn centre_tap_always_wins_against_strong_prior() {
        let keys = row();
        let mut prior = [1.0; 26];
        prior[letter_index('q')] = 1e-6;
        let p = Point::new(W / 2.0, H / 2.0);
        assert_eq!(resolve(&keys, p, &prior, &params(&keys)), Some(0));
    }

    #[test]
    fn strength_zero_is_plain_geometry() {
        let keys = row();
        let mut prior = [0.001; 26];
        prior[letter_index('t')] = 1.0;
        let params = ResolveParams::for_keys(&keys, 0.0);
        for x in -10..=300 {
            for y in -10..=54 {
                let p = Point::new(x as f32, y as f32);
                let mut best = 0;
                let mut best_d = f32::INFINITY;
                for (i, k) in keys.iter().enumerate() {
                    let dx = (k.rect.x() - p.x).max(0.0).max(p.x - k.rect.right());
                    let dy = (k.rect.y() - p.y).max(0.0).max(p.y - k.rect.bottom());
                    let d = dx.hypot(dy);
                    if d < best_d {
                        best_d = d;
                        best = i;
                    }
                }
                assert_eq!(
                    resolve(&keys, p, &prior, &params),
                    Some(best),
                    "at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn gap_tap_goes_somewhere() {
        let keys = row();
        // Midpoint of the e-r gap.
        let p = Point::new(3.0 * (W + GAP) - GAP / 2.0, H / 2.0);
        let got = resolve(&keys, p, &FLAT, &params(&keys));
        assert!(matches!(got, Some(2) | Some(3)), "got {got:?}");
    }

    #[test]
    fn gap_near_backspace_goes_to_backspace() {
        let keys = row();
        let mut prior = [0.001; 26];
        prior[letter_index('t')] = 1.0;
        // 5 px from t, 3 px from BackSpace.
        let p = Point::new(5.0 * (W + GAP) - 3.0, H / 2.0);
        assert_eq!(resolve(&keys, p, &prior, &params(&keys)), Some(5));
    }

    #[test]
    fn shift_is_bounded() {
        let keys = row();
        let mut prior = [1e-6; 26];
        prior[letter_index('t')] = 1.0;
        // 1 px inside w's left edge.
        let p = Point::new(W + GAP + 1.0, H / 2.0);
        let got = resolve(&keys, p, &prior, &params(&keys));
        assert_ne!(got, Some(4));
        assert_eq!(got, Some(1));
    }

    #[test]
    fn empty_keys_is_none() {
        let p = ResolveParams::for_keys(&[], 1.0);
        assert_eq!(resolve(&[], Point::ZERO, &FLAT, &p), None);
    }
}
