//! The bookmarks file, which another program can write (requirement 20.20):
//! whatever it holds, reading it keeps every bound, makes no cycle, and
//! what was read writes a file that reads back the same.
#![no_main]

use libfuzzer_sys::fuzz_target;
use neoteletext::bookmarks::{Bookmarks, MAX_DEPTH, MAX_ITEMS};

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let (bookmarks, _) = Bookmarks::read(&text);
    assert!(bookmarks.len() <= MAX_ITEMS);
    let walked = bookmarks.walk();
    assert_eq!(walked.len(), bookmarks.len() + 2, "every item reached once from the two folders: no cycle");
    assert!(walked.iter().all(|&id| bookmarks.depth(id) <= MAX_DEPTH));
    let (again, dropped) = Bookmarks::read(&bookmarks.text());
    assert_eq!(again.text(), bookmarks.text());
    assert!(dropped.is_empty(), "{dropped:?}");
});
