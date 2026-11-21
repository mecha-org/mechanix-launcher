//! A [`Predictor`] backed by a word-frequency [`Dictionary`].

use std::sync::Arc;

use super::{FLAT, LetterPrior, Predictor};
use crate::dictionaries::{Dictionary, is_normalised};

/// Share of probability mass spread evenly over all letters, so no letter's
/// prior is ever zero.
pub(crate) const SMOOTHING: f32 = 0.01;

/// Next-letter priors from a dictionary, with the priors for the empty
/// prefix and each single-letter prefix precomputed.
pub(crate) struct DictionaryPredictor {
    dict: Arc<Dictionary>,
    empty: LetterPrior,
    single: [LetterPrior; 26],
}

impl DictionaryPredictor {
    pub(crate) fn new(dict: Arc<Dictionary>) -> DictionaryPredictor {
        let mut p = DictionaryPredictor {
            dict,
            empty: FLAT,
            single: [FLAT; 26],
        };
        p.empty = p.compute("");
        for i in 0..26 {
            let prefix = ((b'a' + i as u8) as char).to_string();
            p.single[i] = p.compute(&prefix);
        }
        p
    }

    /// Prior for an already-normalised (lowercase `[a-z']`) prefix.
    fn compute(&self, prefix: &str) -> LetterPrior {
        let mut mass = [0f64; 26];
        for (w, c) in self.dict.prefixed(prefix) {
            if let Some(&b) = w.as_bytes().get(prefix.len())
                && b.is_ascii_lowercase()
            {
                mass[(b - b'a') as usize] += f64::from(*c);
            }
        }
        let total: f64 = mass.iter().sum();
        if total <= 0.0 {
            return FLAT;
        }
        let s = f64::from(SMOOTHING);
        let mut out = [0f32; 26];
        for (o, m) in out.iter_mut().zip(mass) {
            *o = ((m + s * total / 26.0) / (total * (1.0 + s))) as f32;
        }
        out
    }
}

impl Predictor for DictionaryPredictor {
    fn prior(&self, prefix: &str) -> LetterPrior {
        let p = prefix.to_lowercase();
        if !is_normalised(&p) {
            return FLAT;
        }
        match p.as_bytes() {
            [] => self.empty,
            [b @ b'a'..=b'z'] => self.single[(b - b'a') as usize],
            _ => self.compute(&p),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TINY: &str = "the\t100\nthat\t50\nthis\t30\nto\t80\nzoo\t1\n";

    fn tiny() -> DictionaryPredictor {
        DictionaryPredictor::new(Arc::new(Dictionary::parse(TINY)))
    }

    fn bundled(name: &str) -> DictionaryPredictor {
        DictionaryPredictor::new(Arc::new(Dictionary::bundled(name).unwrap()))
    }

    fn idx(c: char) -> usize {
        (c as u8 - b'a') as usize
    }

    #[test]
    fn prior_after_th_favours_e_over_q() {
        let p = tiny().prior("th");
        assert!(p[idx('e')] > 10.0 * p[idx('q')]);
        assert!(p[idx('a')] > p[idx('q')]);
    }

    #[test]
    fn prior_sums_to_one_and_is_never_zero() {
        let d = tiny();
        for prefix in ["", "th", "zz"] {
            let p = d.prior(prefix);
            let sum: f32 = p.iter().sum();
            assert!((sum - 1.0).abs() < 1e-4, "{prefix:?} sums to {sum}");
            assert!(p.iter().all(|&v| v > 0.0), "{prefix:?} has a zero");
        }
    }

    #[test]
    fn empty_prefix_is_start_letter_distribution() {
        let p = tiny().prior("");
        assert!(p[idx('t')] > 10.0 * p[idx('z')]);
    }

    #[test]
    fn unknown_prefix_is_flat() {
        assert_eq!(tiny().prior("qx"), FLAT);
    }

    #[test]
    fn prefix_is_case_insensitive() {
        let d = tiny();
        assert_eq!(d.prior("TH"), d.prior("th"));
    }

    #[test]
    fn non_ascii_prefix_is_flat() {
        assert_eq!(tiny().prior("thé"), FLAT);
    }

    #[test]
    fn empty_dictionary_is_flat() {
        let d = DictionaryPredictor::new(Arc::new(Dictionary::parse("")));
        assert_eq!(d.prior("th"), FLAT);
    }

    #[test]
    fn bundled_terminal_biases_commands() {
        let prior = bundled("terminal").prior("sys");
        assert!(prior[idx('t')] > prior[idx('q')]);
        assert!(prior[idx('t')] > FLAT[idx('t')]);
    }

    #[test]
    fn prior_lookup_is_fast() {
        let d = bundled("en");
        let start = std::time::Instant::now();
        for _ in 0..2_000 {
            for p in ["", "t", "th", "con", "inter"] {
                std::hint::black_box(d.prior(p));
            }
        }
        assert!(
            start.elapsed() < std::time::Duration::from_secs(1),
            "{:?}",
            start.elapsed()
        );
    }
}
