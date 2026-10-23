//! A reader of the Netscape bookmark file, which every browser writes and
//! reads (decision 71). It runs in a decoder process, which opens no file
//! and no network connection, because the trusted core holds no reader of
//! outside formats (requirement 3.2).
//!
//! It is not an HTML parser: it knows the file's few tags, `DL` for a
//! folder's contents, `H3` for a folder's name and `A` for a bookmark, and
//! passes over every other tag and its attributes. Text is kept only
//! inside `H3` and `A`, and only `HREF` of the attributes; every length,
//! the folders' depth and the count of what is found are bounded.

use interface_core::library::MAX_FOLDER_DEPTH;
use protocol::decoder::{BookmarkEntry, MAX_BOOKMARK_ENTRIES, MAX_BOOKMARK_FILE, MAX_BOOKMARK_NAME, is_netscape_bookmarks};

#[cfg(test)]
mod tests;

/// The longest attribute value kept: an address's bound. Longer values,
/// such as an `ICON`'s picture written into the file, are passed over.
const MAX_ATTRIBUTE: usize = protocol::MAX_ADDRESS;
/// The longest entity name read after `&`, `#x10FFFF` being the longest
/// numeric one.
const MAX_ENTITY: usize = 8;
/// The radix of a numeric character reference: decimal, or hexadecimal
/// after `x`.
const DECIMAL: u32 = 10;
const HEXADECIMAL: u32 = 16;

/// What the file held: its folders and bookmarks in order, and how many
/// were left out.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Found {
    /// The folders and bookmarks, each folder's end after its contents.
    pub entries: Vec<BookmarkEntry>,
    /// Left out: a bookmark with no address or too long a one, a folder
    /// deeper than the bookmarks keep, or anything past their count.
    pub skipped: u32,
}

/// Why a file was not read.
#[derive(Debug, PartialEq, Eq)]
pub enum NotRead {
    /// It does not begin as a Netscape bookmark file.
    NotNetscape,
    /// It is longer than [`MAX_BOOKMARK_FILE`].
    TooLarge,
}

/// What text is being kept for.
enum Capture {
    /// An `H3`: a folder's name.
    Folder,
    /// An `A`: a bookmark, with its address, none when it had none that
    /// could be kept.
    Link(Option<String>),
}

#[derive(Default)]
struct Reader {
    found: Found,
    /// Whether each `DL` open opened a folder, innermost last.
    lists: Vec<bool>,
    /// Folders open.
    depth: usize,
    /// A folder's name, waiting for its `DL`.
    pending: Option<String>,
    capture: Option<Capture>,
    text: String,
    /// Folders and bookmarks reported, against [`MAX_BOOKMARK_ENTRIES`].
    counted: usize,
}

impl Reader {
    fn skip(&mut self) {
        self.found.skipped = self.found.skipped.saturating_add(1);
    }

    /// Room for one more folder or bookmark.
    fn room(&mut self) -> bool {
        if self.counted >= MAX_BOOKMARK_ENTRIES {
            self.skip();
            return false;
        }
        self.counted = self.counted.saturating_add(1);
        true
    }

    fn text(&mut self, raw: &str) {
        if self.capture.is_some() {
            decode_into(raw, &mut self.text, MAX_BOOKMARK_NAME);
        }
    }

    fn tag(&mut self, tag: &str) {
        let (closing, tag) = tag.strip_prefix('/').map_or((false, tag), |rest| (true, rest));
        let name_end = tag.find(|character: char| character.is_ascii_whitespace() || character == '/').unwrap_or(tag.len());
        let (name, attributes) = tag.split_at_checked(name_end).unwrap_or((tag, ""));
        let is = |wanted: &str| name.eq_ignore_ascii_case(wanted);
        match (closing, name) {
            (false, _) if is("DL") => self.open_list(),
            (true, _) if is("DL") => self.close_list(),
            (false, _) if is("H3") => self.begin(Capture::Folder),
            (true, _) if is("H3") => {
                if matches!(self.capture, Some(Capture::Folder)) {
                    self.capture = None;
                    self.pending = Some(tidy(&self.text));
                }
            }
            (false, _) if is("A") => {
                let address = attribute(attributes, "HREF").map(|address| tidy(&address)).filter(|address| !address.is_empty());
                self.begin(Capture::Link(address));
            }
            (true, _) if is("A") => {
                if let Some(Capture::Link(address)) = self.capture.take() {
                    match address {
                        Some(address) if self.room() => self.found.entries.push(BookmarkEntry::Link { name: tidy(&self.text), address }),
                        Some(_) => {}
                        None => self.skip(),
                    }
                }
            }
            _ => {}
        }
    }

    fn begin(&mut self, capture: Capture) {
        // A folder's name not followed by its list was an empty folder.
        self.pending = None;
        self.capture = Some(capture);
        self.text.clear();
    }

    fn open_list(&mut self) {
        let opens = match self.pending.take() {
            Some(name) if self.depth < MAX_FOLDER_DEPTH => {
                if self.room() {
                    self.found.entries.push(BookmarkEntry::Folder(name));
                    self.depth = self.depth.saturating_add(1);
                    true
                } else {
                    false
                }
            }
            // Too deep: its contents join the folder around it.
            Some(_) => {
                self.skip();
                false
            }
            None => false,
        };
        self.lists.push(opens);
    }

    fn close_list(&mut self) {
        if self.lists.pop() == Some(true) {
            self.found.entries.push(BookmarkEntry::FolderEnd);
            self.depth = self.depth.saturating_sub(1);
        }
    }

    fn finish(mut self) -> Found {
        // A file cut short: every folder still open ends.
        while !self.lists.is_empty() {
            self.close_list();
        }
        if matches!(self.capture, Some(Capture::Link(_))) {
            self.skip();
        }
        self.found
    }
}

/// Reads a Netscape bookmark file.
///
/// # Errors
/// When it does not begin as one, or is too large.
pub fn read(bytes: &[u8]) -> Result<Found, NotRead> {
    if bytes.len() > MAX_BOOKMARK_FILE {
        return Err(NotRead::TooLarge);
    }
    if !is_netscape_bookmarks(bytes) {
        return Err(NotRead::NotNetscape);
    }
    // Every browser writes it in UTF-8; anything else is replaced.
    let text = String::from_utf8_lossy(bytes);
    let mut reader = Reader::default();
    let mut rest: &str = &text;
    while let Some(open) = rest.find('<') {
        let (before, after) = rest.split_at_checked(open).unwrap_or((rest, ""));
        reader.text(before);
        let after = after.strip_prefix('<').unwrap_or(after);
        if let Some(comment) = after.strip_prefix("!--") {
            rest = comment.find("-->").and_then(|end| comment.get(end.saturating_add("-->".len())..)).unwrap_or("");
            continue;
        }
        let (tag, next) = split_tag(after);
        reader.tag(tag);
        rest = next;
    }
    reader.text(rest);
    Ok(reader.finish())
}

/// The tag up to its `>`, which a quoted attribute value may hold, and
/// what follows.
fn split_tag(text: &str) -> (&str, &str) {
    let mut quote: Option<char> = None;
    for (at, character) in text.char_indices() {
        match (quote, character) {
            (Some(open), _) if character == open => quote = None,
            (None, '"' | '\'') => quote = Some(character),
            (None, '>') => return (text.get(..at).unwrap_or(""), text.get(at.saturating_add(1)..).unwrap_or("")),
            _ => {}
        }
    }
    (text, "")
}

/// The value of the attribute `wanted` among a tag's `attributes`, its
/// entities decoded; none when it is absent or longer than
/// [`MAX_ATTRIBUTE`].
fn attribute(attributes: &str, wanted: &str) -> Option<String> {
    let mut rest = attributes;
    loop {
        rest = rest.trim_start_matches(|character: char| character.is_ascii_whitespace() || character == '/');
        if rest.is_empty() {
            return None;
        }
        let name_end = rest.find(|character: char| character.is_ascii_whitespace() || character == '=').unwrap_or(rest.len());
        let (name, after) = rest.split_at_checked(name_end).unwrap_or((rest, ""));
        let after = after.trim_start();
        let (value, next) = if let Some(after) = after.strip_prefix('=') {
            let after = after.trim_start();
            if let Some(quote @ ('"' | '\'')) = after.chars().next() {
                let inside = after.get(quote.len_utf8()..).unwrap_or("");
                let end = inside.find(quote).unwrap_or(inside.len());
                (inside.get(..end).unwrap_or(""), inside.get(end.saturating_add(quote.len_utf8())..).unwrap_or(""))
            } else {
                let end = after.find(|character: char| character.is_ascii_whitespace()).unwrap_or(after.len());
                after.split_at_checked(end).unwrap_or((after, ""))
            }
        } else {
            ("", after)
        };
        if name.eq_ignore_ascii_case(wanted) {
            if value.len() > MAX_ATTRIBUTE {
                return None;
            }
            let mut decoded = String::new();
            decode_into(value, &mut decoded, MAX_ATTRIBUTE);
            return Some(decoded);
        }
        if next.len() >= rest.len() {
            // Nothing was read: a stray character, passed over.
            rest = rest.get(rest.chars().next().map_or(1, char::len_utf8)..).unwrap_or("");
        } else {
            rest = next;
        }
    }
}

/// Appends `raw` to `out` with its character references decoded, while
/// `out` is shorter than `limit` bytes.
fn decode_into(raw: &str, out: &mut String, limit: usize) {
    let mut rest = raw;
    while !rest.is_empty() && out.len() < limit {
        let Some(amp) = rest.find('&') else {
            push_within(out, rest, limit);
            return;
        };
        let (plain, after) = rest.split_at_checked(amp).unwrap_or((rest, ""));
        push_within(out, plain, limit);
        let after = after.strip_prefix('&').unwrap_or(after);
        let semicolon = after.get(..after.len().min(MAX_ENTITY.saturating_add(1))).and_then(|head| head.find(';'));
        if let Some((character, end)) = semicolon.and_then(|end| Some((entity(after.get(..end)?)?, end))) {
            push_within(out, character.encode_utf8(&mut [0; 4]), limit);
            rest = after.get(end.saturating_add(1)..).unwrap_or("");
        } else {
            push_within(out, "&", limit);
            rest = after;
        }
    }
}

/// The character an entity names: the five XML ones, `nbsp`, and numeric
/// references.
fn entity(name: &str) -> Option<char> {
    let numeric = |digits: &str, radix: u32| u32::from_str_radix(digits, radix).ok().and_then(char::from_u32);
    match name {
        "amp" => Some('&'),
        "lt" => Some('<'),
        "gt" => Some('>'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "nbsp" => Some(' '),
        _ => {
            let number = name.strip_prefix('#')?;
            match number.strip_prefix(['x', 'X']) {
                Some(hexadecimal) => numeric(hexadecimal, HEXADECIMAL),
                None => numeric(number, DECIMAL),
            }
        }
    }
}

/// Appends as much of `text` as keeps `out` within `limit` bytes, on a
/// character boundary.
fn push_within(out: &mut String, text: &str, limit: usize) {
    let room = limit.saturating_sub(out.len());
    let end = (0..=text.len().min(room)).rev().find(|&end| text.is_char_boundary(end)).unwrap_or(0);
    out.push_str(text.get(..end).unwrap_or(""));
}

/// White space run together and trimmed, as a browser shows a name.
fn tidy(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
