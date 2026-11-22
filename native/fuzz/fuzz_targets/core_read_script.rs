//! The broker's reading of a page file's script for the trusted core
//! (script-api.md section 12.7; DEC-7315 version 2; the broker's request
//! kind 26; `neoteletext::source::read_script`, opened by TODO-7439): an
//! arbitrary name beside a page file or a folder that was given to the
//! browser. The target makes, once, a folder of every kind of file a
//! name could reach: the one ordinary script, a symbolic link out of the
//! folder, a second hard link to a file outside it, a folder, an empty
//! file, a file one byte past a script's length, and a page and script
//! in a refused place. Whatever the name, what is read is the folder's
//! own ordinary script or page file, never anything else.
#![no_main]

use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use libfuzzer_sys::fuzz_target;
use neoteletext::source::{Places, read_script};

/// The bytes of the folder's one ordinary script and of its page file.
const SCRIPT: &[u8] = b"/*! nt {\"api\":1} */\n";
const PAGE: &[u8] = b"PN,10000\nOL,1,HELLO\n";
/// Bytes that must never be read.
const SECRET: &[u8] = b"secret: outside the folder";
/// A script's length (the engine's `MAX_SCRIPT_BYTES`).
const SCRIPT_BYTES: usize = neoteletext_core::consts::MAX_SCRIPT_BYTES;

struct Made {
    page: PathBuf,
    folder: PathBuf,
    refused_page: PathBuf,
    places: Places,
}

/// The folders, made under the fuzz crate's own ignored `target`.
fn made() -> &'static Made {
    static MADE: OnceLock<Made> = OnceLock::new();
    MADE.get_or_init(|| {
        let root = Path::new("target/fuzz-scratch/core_read_script");
        let _ = fs::remove_dir_all(root);
        let (folder, refused) = (root.join("channel"), root.join("refused"));
        for made in [&folder, &refused, &folder.join("sub.js")] {
            fs::create_dir_all(made).expect("the scratch folder");
        }
        let write = |path: PathBuf, bytes: &[u8]| fs::write(path, bytes).expect("a scratch file");
        write(folder.join("100.tti"), PAGE);
        write(folder.join("boat.js"), SCRIPT);
        write(folder.join("empty.js"), b"");
        write(folder.join("big.js"), &vec![b' '; SCRIPT_BYTES + 1]);
        write(root.join("outside.js"), SECRET);
        write(root.join("hard-source.js"), SECRET);
        fs::hard_link(root.join("hard-source.js"), folder.join("hard.js")).expect("a hard link");
        symlink("../outside.js", folder.join("link.js")).expect("a symbolic link");
        write(refused.join("100.tti"), PAGE);
        write(refused.join("boat.js"), SECRET);
        let canonical = |path: PathBuf| path.canonicalize().expect("a canonical path");
        let places = Places::new(None, vec![canonical(refused.clone())]);
        Made { page: canonical(folder.join("100.tti")), folder: canonical(folder), refused_page: canonical(refused.join("100.tti")), places }
    })
}

fuzz_target!(init: { made(); }, |data: &[u8]| {
    let Some((&choice, name)) = data.split_first() else { return };
    let Ok(name) = std::str::from_utf8(name) else { return };
    let made = made();
    let given = match choice % 3 {
        0 => &made.page,
        1 => &made.folder,
        _ => &made.refused_page,
    };
    let Ok(bytes) = read_script(given, name, &made.places) else { return };
    assert!(choice % 3 != 2, "{name:?} was read in a refused place");
    assert!(bytes == SCRIPT || bytes == PAGE, "{name:?} beside {} read {} bytes that are not the folder's own", given.display(), bytes.len());
});
