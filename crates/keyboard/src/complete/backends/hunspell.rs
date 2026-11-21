//! Spell-checking with hunspell: when the typed word is misspelled,
//! hunspell's suggestions for it, as corrections. A correctly spelled word
//! gets nothing here; completions come from another source (see
//! [`Merge`](super::Merge)).
//!
//! The system's libhunspell is opened at runtime, so the keyboard builds
//! without hunspell's headers (or libclang) and runs without the library:
//! a missing library or dictionary leaves this source out, with a warning.

use std::ffi::{CStr, CString, c_char, c_int};

use libloading::Library;

use crate::complete::{Candidate, Completer, Kind, Query};

/// Where system hunspell dictionaries live.
const DICT_DIR: &str = "/usr/share/hunspell";

/// The library names to try, newest first.
const LIBRARIES: &[&str] = &[
    "libhunspell-1.7.so.0",
    "libhunspell-1.7.so",
    "libhunspell-1.6.so.0",
    "libhunspell.so",
];

/// Words shorter than this (in chars) are not checked: too little to correct.
const MIN_LEN: usize = 2;

/// `Hunhandle`, opaque.
#[repr(C)]
struct Hunhandle {
    _private: [u8; 0],
}

// The C API from hunspell.h, stable across 1.x.
type CreateFn = unsafe extern "C" fn(*const c_char, *const c_char) -> *mut Hunhandle;
type DestroyFn = unsafe extern "C" fn(*mut Hunhandle);
type SpellFn = unsafe extern "C" fn(*mut Hunhandle, *const c_char) -> c_int;
type SuggestFn =
    unsafe extern "C" fn(*mut Hunhandle, *mut *mut *mut c_char, *const c_char) -> c_int;
type FreeListFn = unsafe extern "C" fn(*mut Hunhandle, *mut *mut *mut c_char, c_int);
type AddFn = unsafe extern "C" fn(*mut Hunhandle, *const c_char) -> c_int;
type EncodingFn = unsafe extern "C" fn(*mut Hunhandle) -> *mut c_char;

/// An open hunspell dictionary: the library, its functions and a handle.
struct Speller {
    handle: *mut Hunhandle,
    destroy: DestroyFn,
    spell: SpellFn,
    suggest: SuggestFn,
    free_list: FreeListFn,
    add: AddFn,
    /// Kept loaded while the function pointers above are in use; dropped
    /// after the handle is destroyed.
    _lib: Library,
}

// SAFETY: the handle is an owned C++ object with no thread affinity or
// shared state; it is only reached through `&mut HunspellCompleter`, so
// never from two threads at once.
unsafe impl Send for Speller {}

impl Speller {
    /// Open the library and the dictionary at `aff`/`dic`.
    fn open(aff: &str, dic: &str) -> Result<Speller, String> {
        let lib = LIBRARIES
            .iter()
            // SAFETY: loading libhunspell runs no initialisers with
            // preconditions.
            .find_map(|name| unsafe { Library::new(name) }.ok())
            .ok_or("libhunspell not found")?;
        // SAFETY: each symbol has the C signature declared above.
        unsafe {
            let create = *lib
                .get::<CreateFn>(b"Hunspell_create\0")
                .map_err(|e| e.to_string())?;
            let destroy = *lib
                .get::<DestroyFn>(b"Hunspell_destroy\0")
                .map_err(|e| e.to_string())?;
            let spell = *lib
                .get::<SpellFn>(b"Hunspell_spell\0")
                .map_err(|e| e.to_string())?;
            let suggest = *lib
                .get::<SuggestFn>(b"Hunspell_suggest\0")
                .map_err(|e| e.to_string())?;
            let free_list = *lib
                .get::<FreeListFn>(b"Hunspell_free_list\0")
                .map_err(|e| e.to_string())?;
            let add = *lib
                .get::<AddFn>(b"Hunspell_add\0")
                .map_err(|e| e.to_string())?;
            let encoding = *lib
                .get::<EncodingFn>(b"Hunspell_get_dic_encoding\0")
                .map_err(|e| e.to_string())?;
            let (aff_c, dic_c) = (cstring(aff)?, cstring(dic)?);
            let handle = create(aff_c.as_ptr(), dic_c.as_ptr());
            if handle.is_null() {
                return Err("Hunspell_create failed".into());
            }
            let speller = Speller {
                handle,
                destroy,
                spell,
                suggest,
                free_list,
                add,
                _lib: lib,
            };
            // Words cross as UTF-8; a dictionary in another encoding would
            // garble them.
            let enc = encoding(handle);
            let enc = if enc.is_null() {
                String::new()
            } else {
                CStr::from_ptr(enc).to_string_lossy().into_owned()
            };
            if !enc.eq_ignore_ascii_case("UTF-8") {
                return Err(format!("dictionary encoding {enc:?} is not UTF-8"));
            }
            Ok(speller)
        }
    }

    /// Whether `word` is spelled right.
    fn check(&self, word: &CStr) -> bool {
        // SAFETY: a live handle and a NUL-terminated string.
        unsafe { (self.spell)(self.handle, word.as_ptr()) != 0 }
    }

    /// Hunspell's suggestions for `word`, best first.
    fn suggest(&self, word: &CStr) -> Vec<String> {
        let mut list: *mut *mut c_char = std::ptr::null_mut();
        // SAFETY: a live handle; hunspell allocates `list` with `n` entries,
        // which are read and then handed back to it to free.
        unsafe {
            let n = (self.suggest)(self.handle, &mut list, word.as_ptr());
            if n <= 0 || list.is_null() {
                return Vec::new();
            }
            let out = (0..n as usize)
                .map(|i| *list.add(i))
                .filter(|p| !p.is_null())
                .map(|p| CStr::from_ptr(p).to_string_lossy().into_owned())
                .collect();
            (self.free_list)(self.handle, &mut list, n);
            out
        }
    }

    /// Accept `word` as spelled right from now on.
    fn add(&mut self, word: &CStr) {
        // SAFETY: a live handle and a NUL-terminated string.
        unsafe { (self.add)(self.handle, word.as_ptr()) };
    }
}

impl Drop for Speller {
    fn drop(&mut self) {
        // SAFETY: the handle is live and destroyed once, before `_lib`
        // unloads the code.
        unsafe { (self.destroy)(self.handle) };
    }
}

fn cstring(s: &str) -> Result<CString, String> {
    CString::new(s).map_err(|e| e.to_string())
}

/// Corrections from hunspell.
pub(crate) struct HunspellCompleter {
    speller: Speller,
    /// The last word asked about and its candidates: `suggest` is slow (tens
    /// of ms on a phone), and the same word is asked about again whenever
    /// the context changes without the word changing.
    last: Option<(String, Vec<Candidate>)>,
}

impl HunspellCompleter {
    /// Load `lang` (e.g. `en_US`) from the system dictionaries; `None`
    /// (warned) if the library or the dictionary is missing.
    pub(crate) fn load(lang: &str) -> Option<Self> {
        let aff = format!("{DICT_DIR}/{lang}.aff");
        let dic = format!("{DICT_DIR}/{lang}.dic");
        if !std::path::Path::new(&aff).exists() || !std::path::Path::new(&dic).exists() {
            tracing::warn!(lang, "no hunspell dictionary at {aff} / {dic}; left out");
            return None;
        }
        match Speller::open(&aff, &dic) {
            Ok(speller) => {
                tracing::info!(lang, "hunspell dictionary loaded");
                Some(Self {
                    speller,
                    last: None,
                })
            }
            Err(e) => {
                tracing::warn!(lang, "cannot use hunspell: {e}; left out");
                None
            }
        }
    }
}

impl Completer for HunspellCompleter {
    fn suggest(&mut self, q: &Query) -> Vec<Candidate> {
        let word = format!("{}{}", q.word, q.word_after);
        if word.chars().count() < MIN_LEN {
            return Vec::new();
        }
        if let Some((last, found)) = &self.last
            && *last == word
        {
            return found.clone();
        }
        let Ok(c_word) = CString::new(word.as_str()) else {
            return Vec::new();
        };
        let found = if self.speller.check(&c_word) {
            Vec::new()
        } else {
            self.speller
                .suggest(&c_word)
                .into_iter()
                .take(q.max)
                .map(|text| Candidate {
                    text,
                    kind: Kind::Correction,
                })
                .collect()
        };
        self.last = Some((word, found.clone()));
        found
    }

    fn learn(&mut self, word: &str) {
        // Words the user typed and kept are not misspellings: stop
        // correcting them this session.
        if let Ok(c_word) = CString::new(word) {
            self.speller.add(&c_word);
            self.last = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(word: &str) -> Query<'_> {
        Query {
            word,
            word_after: "",
            max: 3,
        }
    }

    /// Runs only where the system has libhunspell and the en_US dictionary.
    fn en() -> Option<HunspellCompleter> {
        HunspellCompleter::load("en_US")
    }

    #[test]
    fn corrects_misspellings_only() {
        let Some(mut h) = en() else {
            return;
        };
        let s = h.suggest(&q("recieve"));
        assert!(s.iter().any(|c| c.text == "receive"), "{s:?}");
        assert!(s.iter().all(|c| c.kind == Kind::Correction));
        assert!(h.suggest(&q("receive")).is_empty());
        assert!(h.suggest(&q("x")).is_empty());
    }

    #[test]
    fn learned_words_are_not_corrected() {
        let Some(mut h) = en() else {
            return;
        };
        assert!(!h.suggest(&q("mechanixy")).is_empty());
        h.learn("mechanixy");
        assert!(h.suggest(&q("mechanixy")).is_empty());
    }

    #[test]
    fn missing_dictionary_is_none() {
        assert!(HunspellCompleter::load("xx_NOPE").is_none());
    }
}
