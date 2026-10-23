//! A bookmark file read by a decoder (decision 71): the bounds of what it
//! reports, what it found, in the file's order, and the Netscape file's
//! first line, by which the trusted core knows the file.

use super::kinds::{KIND_BOOKMARK_FOLDER, KIND_BOOKMARK_FOLDER_END, KIND_BOOKMARK_LINK};
use crate::{Fields, ProtocolError, push_raw_text, raw_text};

/// The largest bookmark file a decoder reads (decision 71): the trusted
/// core's own bookmark file's bound.
pub const MAX_BOOKMARK_FILE: usize = 24 * 1024 * 1024;
/// The most entries a decoder reports from a bookmark file: the trusted
/// core's bound on bookmarks and folders.
pub const MAX_BOOKMARK_ENTRIES: usize = 10_000;
/// A bookmark's name as a decoder reports it, before the trusted core
/// cleans it: the name's bound, with room for what cleaning removes.
pub const MAX_BOOKMARK_NAME: usize = 4 * interface_core::library::MAX_NAME;
/// The first line of the Netscape bookmark file, which every browser
/// writes and reads (decision 71).
pub const NETSCAPE_DOCTYPE: &str = "<!DOCTYPE NETSCAPE-Bookmark-file-1>";

/// One thing a decoder found in a bookmark file, in the file's order
/// (decision 71): untrusted, for the trusted core to check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BookmarkEntry {
    /// A folder opens, by this name; what follows is in it until its end.
    Folder(String),
    /// The folder last opened ends.
    FolderEnd,
    /// A bookmark.
    Link {
        /// Its name.
        name: String,
        /// Its address, as written.
        address: String,
    },
}

/// Whether `bytes` are a Netscape bookmark file: its first line, after
/// any byte order mark and white space, in any case. Only the first line
/// is looked at; the file is read by a decoder (decision 71).
#[must_use]
pub fn is_netscape_bookmarks(bytes: &[u8]) -> bool {
    let bytes = bytes.strip_prefix("\u{feff}".as_bytes()).unwrap_or(bytes);
    let start = bytes.iter().position(|byte| !byte.is_ascii_whitespace()).unwrap_or(bytes.len());
    bytes.get(start..).and_then(|rest| rest.get(..NETSCAPE_DOCTYPE.len())).is_some_and(|first| first.eq_ignore_ascii_case(NETSCAPE_DOCTYPE.as_bytes()))
}

/// The payload of a bookmark entry, and its kind.
pub(super) fn push_entry(payload: &mut Vec<u8>, entry: &BookmarkEntry) -> u8 {
    match entry {
        BookmarkEntry::Folder(name) => {
            push_raw_text(payload, name, MAX_BOOKMARK_NAME);
            KIND_BOOKMARK_FOLDER
        }
        BookmarkEntry::FolderEnd => KIND_BOOKMARK_FOLDER_END,
        BookmarkEntry::Link { name, address } => {
            push_raw_text(payload, name, MAX_BOOKMARK_NAME);
            push_raw_text(payload, address, crate::MAX_ADDRESS);
            KIND_BOOKMARK_LINK
        }
    }
}

/// Reads a bookmark entry of `kind`.
pub(super) fn read_entry(kind: u8, fields: &mut Fields<'_>) -> Result<BookmarkEntry, ProtocolError> {
    Ok(match kind {
        KIND_BOOKMARK_FOLDER => BookmarkEntry::Folder(raw_text(fields, MAX_BOOKMARK_NAME, "name too long")?),
        KIND_BOOKMARK_FOLDER_END => BookmarkEntry::FolderEnd,
        KIND_BOOKMARK_LINK => {
            let name = raw_text(fields, MAX_BOOKMARK_NAME, "name too long")?;
            let address = raw_text(fields, crate::MAX_ADDRESS, "address too long")?;
            BookmarkEntry::Link { name, address }
        }
        other => return Err(ProtocolError::UnknownKind(other)),
    })
}
