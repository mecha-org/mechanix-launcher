//! Compiles the bundled word lists (`resources/dict/<locale>_wordlist.combined.gz`,
//! kept exactly as AOSP LatinIME ships them) into the form the keyboard
//! searches in place, so the device neither inflates, parses nor sorts them
//! at startup. See `src/dictionaries/wordlist/compile.rs`.

use std::io::Read;
use std::path::PathBuf;

#[path = "src/dictionaries/wordlist/compile.rs"]
mod compile;

/// The bundled lists, by locale.
const LISTS: &[&str] = &["en_US"];

fn main() {
    println!("cargo::rerun-if-changed=src/dictionaries/wordlist/compile.rs");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR"));
    for name in LISTS {
        let src = format!("resources/dict/{name}_wordlist.combined.gz");
        println!("cargo::rerun-if-changed={src}");
        let gz = std::fs::read(&src).unwrap_or_else(|e| panic!("reading {src}: {e}"));
        let mut text = String::new();
        flate2::read::GzDecoder::new(&gz[..])
            .read_to_string(&mut text)
            .unwrap_or_else(|e| panic!("inflating {src}: {e}"));
        let compiled = compile::compile(&text);
        let write = |ext: &str, bytes: &[u8]| {
            let path = out.join(format!("{name}.{ext}"));
            std::fs::write(&path, bytes)
                .unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
        };
        write("words", compiled.text.as_bytes());
        write("entries", &compiled.entries);
        write("replacements", compiled.replacements.as_bytes());
    }
}
