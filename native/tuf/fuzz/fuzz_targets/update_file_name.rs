//! A file name the update client asks the update host for, joined to the
//! repository's two folders as it stands, unescaped (TODO-7390 step 5
//! item 4; `updater::asked_for`, over the transport's own check of an
//! address, `updater::transport::asked_of`). The names come from signed
//! metadata, but the check must hold whatever they are: the answer is a
//! refusal, or a path of the update host inside one of its two folders,
//! with no part that climbs out of it.
#![no_main]

use libfuzzer_sys::fuzz_target;
use updater::asked_for;
use updater::transport::Folder;

/// The update host, at its port.
const HOST: &str = "updates.example.test";
const PORT: u16 = 443;

fuzz_target!(|data: &[u8]| {
    let Some((&choice, name)) = data.split_first() else { return };
    let Ok(name) = std::str::from_utf8(name) else { return };
    let folder = if choice & 1 == 0 { Folder::Metadata } else { Folder::Targets };
    let Ok((path, landed)) = asked_for(HOST, PORT, folder, name) else { return };
    let inside = format!("/{}/", landed.name());
    assert!(path.starts_with(&inside), "{name:?} in {folder:?} asked for {path:?}");
    let path_only = path.split('?').next().unwrap_or_default();
    assert!(
        !path_only.split('/').any(|part| part == ".." || part == "." || part.eq_ignore_ascii_case("%2e%2e") || part.eq_ignore_ascii_case("%2e")),
        "{name:?} asked for {path:?}"
    );
    // The query is the update host's to read as it likes; the path is
    // what names a file, and it holds no separator but the slash. A name
    // joined unescaped may make a query (`?`); the update library escapes
    // its names, so that only this opening's harder case makes one.
    assert!(!path_only.contains('\\') && !path.contains(['#', '\r', '\n', '\0']), "{name:?} asked for {path:?}");
});
