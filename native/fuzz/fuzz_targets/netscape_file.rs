//! The Netscape bookmark file, which a decoder reads for an import
//! (decision 71): whatever it holds, reading it ends, keeps every bound,
//! and every folder it opens it ends.
#![no_main]

use decoder::netscape::read;
use interface_core::library::MAX_FOLDER_DEPTH;
use libfuzzer_sys::fuzz_target;
use protocol::decoder::{BookmarkEntry, MAX_BOOKMARK_ENTRIES, MAX_BOOKMARK_NAME};

fuzz_target!(|data: &[u8]| {
    let Ok(found) = read(data) else { return };
    let mut depth = 0_usize;
    let mut counted = 0_usize;
    for entry in &found.entries {
        match entry {
            BookmarkEntry::Folder(name) => {
                depth += 1;
                counted += 1;
                assert!(depth <= MAX_FOLDER_DEPTH && name.len() <= MAX_BOOKMARK_NAME);
            }
            BookmarkEntry::FolderEnd => depth = depth.checked_sub(1).expect("an end without its folder"),
            BookmarkEntry::Link { name, address } => {
                counted += 1;
                assert!(name.len() <= MAX_BOOKMARK_NAME && address.len() <= protocol::MAX_ADDRESS && !address.is_empty());
            }
        }
    }
    assert_eq!(depth, 0, "every folder ends");
    assert!(counted <= MAX_BOOKMARK_ENTRIES);
});
