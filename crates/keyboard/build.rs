//! Compiles the bundled word lists (`resources/dict/<locale>_wordlist.combined.gz`,
//! kept exactly as AOSP LatinIME ships them) and emoji lists
//! (`resources/emoji/<name>.json.gz`, kept exactly as GitHub gemoji ships
//! them) into the forms the keyboard searches in place, so the device
//! neither inflates, parses nor sorts them at startup. See
//! `src/dictionaries/wordlist/compile.rs` and
//! `src/dictionaries/emoji/compile.rs`.

use std::io::Read;
use std::path::{Path, PathBuf};

#[path = "src/dictionaries/wordlist/compile.rs"]
mod compile;

#[path = "src/dictionaries/emoji/compile.rs"]
mod emoji_compile;

/// The bundled word lists, by locale.
const LISTS: &[&str] = &["en_US"];

/// The bundled emoji lists, by name.
const EMOJI_LISTS: &[&str] = &["gemoji"];

fn main() {
    println!("cargo::rerun-if-changed=src/dictionaries/wordlist/compile.rs");
    println!("cargo::rerun-if-changed=src/dictionaries/emoji/compile.rs");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    for name in LISTS {
        let src = format!("resources/dict/{name}_wordlist.combined.gz");
        let compiled = compile::compile(&inflate(&src));
        write(&out, name, "words", compiled.text.as_bytes());
        write(&out, name, "entries", &compiled.entries);
        write(&out, name, "replacements", compiled.replacements.as_bytes());
    }
    for name in EMOJI_LISTS {
        let src = format!("resources/emoji/{name}.json.gz");
        let compiled = emoji_compile::compile(&inflate(&src))
            .unwrap_or_else(|| panic!("{src} is not a gemoji list"));
        write(&out, name, "text", compiled.text.as_bytes());
        write(&out, name, "emoji", &compiled.emoji);
        write(&out, name, "names", &compiled.names);
    }
}

/// The text of the gzipped file `src`, rebuilt when it changes.
fn inflate(src: &str) -> String {
    println!("cargo::rerun-if-changed={src}");
    let gz = std::fs::read(src).unwrap_or_else(|e| panic!("reading {src}: {e}"));
    let mut text = String::new();
    flate2::read::GzDecoder::new(&gz[..])
        .read_to_string(&mut text)
        .unwrap_or_else(|e| panic!("inflating {src}: {e}"));
    text
}

/// Write `bytes` to `<out>/<name>.<ext>`.
fn write(out: &Path, name: &str, ext: &str, bytes: &[u8]) {
    let path = out.join(format!("{name}.{ext}"));
    std::fs::write(&path, bytes).unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
}
