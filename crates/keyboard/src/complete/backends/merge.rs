//! Several sources as one: e.g. word-list completions with hunspell's
//! corrections.

use crate::complete::{Candidate, Completer, Kind, WordQuery};

/// Interleaves its sources' candidates (each source's best, in source
/// order, then each one's second, ...), skipping repeats. A source's
/// autocorrection stands only if no source takes the typed word as spelled
/// right (one list may lack a word another has). Every source learns every
/// word.
pub(crate) struct Merge(Vec<Box<dyn Completer>>);

impl Merge {
    pub(crate) fn new(sources: Vec<Box<dyn Completer>>) -> Merge {
        Merge(sources)
    }
}

impl Completer for Merge {
    fn suggest(&mut self, q: &WordQuery) -> Vec<Candidate> {
        let lists: Vec<Vec<Candidate>> = self.0.iter_mut().map(|s| s.suggest(q)).collect();
        let mut iters: Vec<_> = lists.into_iter().map(Vec::into_iter).collect();
        let mut out: Vec<Candidate> = Vec::new();
        loop {
            let mut any = false;
            for it in &mut iters {
                if let Some(c) = it.next() {
                    any = true;
                    if !out.iter().any(|o| o.text.eq_ignore_ascii_case(&c.text)) {
                        out.push(c);
                    }
                }
            }
            if !any {
                break;
            }
        }
        let word = format!("{}{}", q.word, q.word_after);
        if let Some(auto) = out.iter_mut().find(|c| c.kind == Kind::Autocorrection)
            && self.accepts(&word)
        {
            auto.kind = Kind::Correction;
        }
        out
    }

    fn accepts(&mut self, word: &str) -> bool {
        self.0.iter_mut().any(|s| s.accepts(word))
    }

    fn learn(&mut self, word: &str) {
        for s in &mut self.0 {
            s.learn(word);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Vec<&'static str>);
    impl Completer for Fixed {
        fn suggest(&mut self, _q: &WordQuery) -> Vec<Candidate> {
            self.0
                .iter()
                .map(|t| Candidate {
                    text: t.to_string(),
                    kind: Kind::Completion,
                })
                .collect()
        }
    }

    /// Autocorrects every word to `fix`.
    struct Fixer(&'static str);
    impl Completer for Fixer {
        fn suggest(&mut self, _q: &WordQuery) -> Vec<Candidate> {
            vec![Candidate {
                text: self.0.to_string(),
                kind: Kind::Autocorrection,
            }]
        }
    }

    /// Takes `word` as spelled right; suggests nothing.
    struct Knows(&'static str);
    impl Completer for Knows {
        fn suggest(&mut self, _q: &WordQuery) -> Vec<Candidate> {
            Vec::new()
        }
        fn accepts(&mut self, word: &str) -> bool {
            word == self.0
        }
    }

    #[test]
    fn a_source_that_knows_the_word_vetoes_autocorrection() {
        let q = |word| WordQuery {
            word,
            word_after: "",
            max: 3,
        };
        let mut m = Merge::new(vec![Box::new(Fixer("mocha")), Box::new(Knows("mecha"))]);
        assert_eq!(m.suggest(&q("mecha"))[0].kind, Kind::Correction);
        assert_eq!(m.suggest(&q("mechq"))[0].kind, Kind::Autocorrection);
        assert!(m.accepts("mecha") && !m.accepts("mechq"));
    }

    fn texts(c: &[Candidate]) -> Vec<&str> {
        c.iter().map(|c| c.text.as_str()).collect()
    }

    #[test]
    fn interleaves_in_source_order_without_repeats() {
        let mut m = Merge::new(vec![
            Box::new(Fixed(vec!["a", "b", "c"])),
            Box::new(Fixed(vec!["B", "x"])),
        ]);
        let q = WordQuery {
            word: "",
            word_after: "",
            max: 3,
        };
        assert_eq!(texts(&m.suggest(&q)), ["a", "B", "x", "c"]);
    }

    #[test]
    fn empty_merge_suggests_nothing() {
        let q = WordQuery {
            word: "x",
            word_after: "",
            max: 3,
        };
        assert!(Merge::new(Vec::new()).suggest(&q).is_empty());
    }
}
