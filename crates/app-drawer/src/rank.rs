//! Fuzzy ranking of app names with match positions, and fitting names to a width.

use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hit {
    pub index: usize,
    pub positions: Vec<u32>,
}

pub(crate) struct Ranker {
    matcher: Matcher,
}

impl Ranker {
    pub(crate) fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
        }
    }

    /// `names` arrive A-Z; a stable sort by score keeps that order for ties.
    pub(crate) fn rank(&mut self, names: &[&str], query: &str) -> Vec<Hit> {
        if query.trim().is_empty() {
            return (0..names.len())
                .map(|index| Hit {
                    index,
                    positions: Vec::new(),
                })
                .collect();
        }
        let pattern = Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart);
        let mut buf = Vec::new();
        let mut scored: Vec<(u32, Hit)> = Vec::new();
        for (index, name) in names.iter().enumerate() {
            let mut positions = Vec::new();
            let haystack = Utf32Str::new(name, &mut buf);
            if let Some(score) = pattern.indices(haystack, &mut self.matcher, &mut positions) {
                positions.sort_unstable();
                positions.dedup();
                scored.push((score, Hit { index, positions }));
            }
        }
        scored.sort_by(|a, b| b.0.cmp(&a.0));
        scored.into_iter().map(|(_, hit)| hit).collect()
    }
}

/// Shortens `name` to at most `max_chars` chars, ending in `…` when cut.
pub(crate) fn fit(name: &str, positions: &[u32], max_chars: usize) -> (String, Vec<u32>) {
    if name.chars().count() <= max_chars {
        return (name.to_string(), positions.to_vec());
    }
    if max_chars == 0 {
        return (String::new(), Vec::new());
    }
    let keep = max_chars - 1;
    let mut out: String = name.chars().take(keep).collect();
    out.push('…');
    let kept = positions
        .iter()
        .copied()
        .filter(|&p| (p as usize) < keep)
        .collect();
    (out, kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx(hits: &[Hit]) -> Vec<usize> {
        hits.iter().map(|h| h.index).collect()
    }

    #[test]
    fn prefix_match_ranks_first_with_positions() {
        let hits = Ranker::new().rank(&["Camera", "Chromium", "Clock"], "chr");
        assert_eq!(hits[0].index, 1);
        assert_eq!(hits[0].positions, vec![0, 1, 2]);
    }

    #[test]
    fn equal_scores_keep_input_order() {
        let hits = Ranker::new().rank(&["Abc", "Xbc"], "bc");
        assert_eq!(idx(&hits), vec![0, 1]);
    }

    #[test]
    fn non_matches_dropped_and_blank_returns_all() {
        let names = ["Camera", "Chromium", "Clock"];
        let mut r = Ranker::new();
        assert!(r.rank(&names, "zzz").is_empty());
        let all = r.rank(&names, "  ");
        assert_eq!(idx(&all), vec![0, 1, 2]);
        assert!(all.iter().all(|h| h.positions.is_empty()));
    }

    #[test]
    fn positions_are_char_indices() {
        let hits = Ranker::new().rank(&["Café Ölmühle"], "olm");
        assert_eq!(hits[0].positions, vec![5, 6, 7]);
    }

    #[test]
    fn fit_truncates_with_ellipsis() {
        let (s, p) = fit("Chromium Web Browser", &[0, 13], 10);
        assert_eq!(s, "Chromium …");
        assert_eq!(p, vec![0]);
    }

    #[test]
    fn fit_short_unchanged_and_zero_empty() {
        assert_eq!(fit("Clock", &[0], 10), ("Clock".to_string(), vec![0]));
        assert_eq!(fit("Clock", &[0], 0), (String::new(), vec![]));
    }
}
