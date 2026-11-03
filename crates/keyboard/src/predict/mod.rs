//! Next-letter prediction: given the text typed so far in the current word,
//! estimate how likely each letter is to come next. Used to bias touch targets.

mod context;
mod dictionary;
mod profile;

#[allow(unused_imports)]
pub(crate) use context::{HOLD_CLEAR_MS, TypingContext};
#[allow(unused_imports)]
pub(crate) use dictionary::{Dictionary, SMOOTHING};
#[allow(unused_imports)]
pub(crate) use profile::{NoopRecorder, Prediction, Profile, Profiles, WordRecorder};

/// Probability of each letter a-z being typed next; index `c as u8 - b'a'`.
/// Every value is > 0 and the values sum to 1.
pub(crate) type LetterPrior = [f32; 26];

/// The uninformative prior: every letter equally likely.
pub(crate) const FLAT: LetterPrior = [1.0 / 26.0; 26];

/// Estimates the next-letter distribution for a word prefix.
pub(crate) trait Predictor: Send + Sync {
    /// Prior over the next letter after `prefix`. The prefix is
    /// case-insensitive and may contain any characters; when nothing is known
    /// about it the result is [`FLAT`].
    fn prior(&self, prefix: &str) -> LetterPrior;
}
