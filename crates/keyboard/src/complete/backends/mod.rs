//! The completion sources.

mod dictionary;
#[cfg(feature = "hunspell")]
mod hunspell;
mod merge;

pub(crate) use dictionary::DictionaryCompleter;
#[cfg(feature = "hunspell")]
pub(crate) use hunspell::HunspellCompleter;
pub(crate) use merge::Merge;
