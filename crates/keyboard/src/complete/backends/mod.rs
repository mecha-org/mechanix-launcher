//! The completion sources.

#[cfg(feature = "hunspell")]
mod hunspell;
mod merge;
mod wordlist;

#[cfg(feature = "hunspell")]
pub(crate) use hunspell::HunspellCompleter;
pub(crate) use merge::Merge;
pub(crate) use wordlist::WordlistCompleter;
