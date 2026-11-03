//! A [`Predictor`] backed by a word-frequency list.

use super::{FLAT, LetterPrior, Predictor};

/// Share of probability mass spread evenly over all letters, so no letter's
/// prior is ever zero.
pub(crate) const SMOOTHING: f32 = 0.01;

/// Word frequencies, sorted by word, with precomputed priors for the empty
/// prefix and each single-letter prefix.
pub(crate) struct Dictionary {
    words: Vec<(Box<str>, u32)>,
    empty: LetterPrior,
    single: [LetterPrior; 26],
}

impl Dictionary {
    /// Parse `word<TAB>count` lines. Blank and malformed lines and words with
    /// characters outside `[a-z']` (after lowercasing) are skipped; duplicate
    /// words have their counts summed.
    pub(crate) fn parse(tsv: &str) -> Dictionary {
        let mut words: Vec<(Box<str>, u32)> = Vec::new();
        for line in tsv.lines() {
            let Some((w, c)) = line.split_once('\t') else { continue };
            let w = w.trim().to_lowercase();
            let Ok(c) = c.trim().parse::<u32>() else { continue };
            if w.is_empty() || !w.bytes().all(|b| b.is_ascii_lowercase() || b == b'\'') {
                continue;
            }
            words.push((w.into_boxed_str(), c));
        }
        words.sort_by(|a, b| a.0.cmp(&b.0));
        words.dedup_by(|dup, kept| {
            if dup.0 == kept.0 {
                kept.1 = kept.1.saturating_add(dup.1);
                true
            } else {
                false
            }
        });
        let mut dict = Dictionary { words, empty: FLAT, single: [FLAT; 26] };
        dict.empty = dict.compute("");
        for i in 0..26 {
            let p = ((b'a' + i as u8) as char).to_string();
            dict.single[i] = dict.compute(&p);
        }
        dict
    }

    /// A word list shipped in the binary, by name.
    pub(crate) fn bundled(name: &str) -> Option<Dictionary> {
        match name {
            "en" => Some(Self::parse(include_str!("../../resources/dict/en.tsv"))),
            _ => None,
        }
    }

    /// Load `spec` as a bundled name, else as a file path; if neither works,
    /// warn and return an empty dictionary (every prior is [`FLAT`]).
    pub(crate) fn load(spec: &str) -> Dictionary {
        if let Some(d) = Self::bundled(spec) {
            return d;
        }
        match std::fs::read_to_string(spec) {
            Ok(text) => Self::parse(&text),
            Err(e) => {
                tracing::warn!("cannot load dictionary {spec:?}: {e}; prediction disabled");
                Self::parse("")
            }
        }
    }

    /// Number of distinct words.
    pub(crate) fn len(&self) -> usize {
        self.words.len()
    }

    /// Prior for an already-normalised (lowercase `[a-z']`) prefix.
    fn compute(&self, prefix: &str) -> LetterPrior {
        let start = self.words.partition_point(|(w, _)| &**w < prefix);
        let mut mass = [0f64; 26];
        for (w, c) in &self.words[start..] {
            let Some(rest) = w.strip_prefix(prefix) else { break };
            if let Some(&b) = rest.as_bytes().first()
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

impl Predictor for Dictionary {
    fn prior(&self, prefix: &str) -> LetterPrior {
        let p = prefix.to_lowercase();
        if !p.bytes().all(|b| b.is_ascii_lowercase() || b == b'\'') {
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

    fn idx(c: char) -> usize {
        (c as u8 - b'a') as usize
    }

    #[test]
    fn prior_after_th_favours_e_over_q() {
        let p = Dictionary::parse(TINY).prior("th");
        assert!(p[idx('e')] > 10.0 * p[idx('q')]);
        assert!(p[idx('a')] > p[idx('q')]);
    }

    #[test]
    fn prior_sums_to_one_and_is_never_zero() {
        let d = Dictionary::parse(TINY);
        for prefix in ["", "th", "zz"] {
            let p = d.prior(prefix);
            let sum: f32 = p.iter().sum();
            assert!((sum - 1.0).abs() < 1e-4, "{prefix:?} sums to {sum}");
            assert!(p.iter().all(|&v| v > 0.0), "{prefix:?} has a zero");
        }
    }

    #[test]
    fn empty_prefix_is_start_letter_distribution() {
        let p = Dictionary::parse(TINY).prior("");
        assert!(p[idx('t')] > 10.0 * p[idx('z')]);
    }

    #[test]
    fn unknown_prefix_is_flat() {
        assert_eq!(Dictionary::parse(TINY).prior("qx"), FLAT);
    }

    #[test]
    fn prefix_is_case_insensitive() {
        let d = Dictionary::parse(TINY);
        assert_eq!(d.prior("TH"), d.prior("th"));
    }

    #[test]
    fn non_ascii_prefix_is_flat() {
        assert_eq!(Dictionary::parse(TINY).prior("thé"), FLAT);
    }

    #[test]
    fn parse_skips_bad_lines() {
        let d = Dictionary::parse("x\n\tq\nok\tnotanumber\nfine\t3\nÉcole\t5");
        assert_eq!(d.len(), 1);
        assert_eq!(&*d.words[0].0, "fine");
    }

    #[test]
    fn duplicates_sum() {
        let d = Dictionary::parse("ab\t1\nAB\t2\n");
        assert_eq!((d.len(), d.words[0].1), (1, 3));
    }

    #[test]
    fn load_falls_back_to_empty_dictionary() {
        let d = Dictionary::load("/nonexistent/dictionary.tsv");
        assert_eq!(d.len(), 0);
        assert_eq!(d.prior("th"), FLAT);
    }

    #[test]
    fn bundled_en_loads_with_enough_words() {
        let d = Dictionary::bundled("en").expect("en is bundled");
        assert!(d.len() >= 20_000, "only {} words", d.len());
        assert!(Dictionary::bundled("xx").is_none());
    }

    #[test]
    fn prior_lookup_is_fast() {
        let d = Dictionary::bundled("en").unwrap();
        let start = std::time::Instant::now();
        for _ in 0..2_000 {
            for p in ["", "t", "th", "con", "inter"] {
                std::hint::black_box(d.prior(p));
            }
        }
        assert!(start.elapsed() < std::time::Duration::from_secs(1), "{:?}", start.elapsed());
    }
}
