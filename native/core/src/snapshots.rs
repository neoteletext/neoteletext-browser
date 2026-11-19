//! Tab thumbnails' snapshots (neoteletext-common's `docs/tab-thumbnails.md`
//! sections 8 and 11; REQ-6883, REQ-6886): the store of snapshots kept
//! between runs, one versioned file per place, the user's alone, bounded,
//! read back as untrusted; the misses remembered in memory; and the pacing
//! of fetching and drawing them.
//!
//! The store holds only what a snapshot decoder gave back after drawing a
//! real page: the sub-page's source with every concealed character blanked
//! (REQ-6885 item 4). The trusted core never parses a body: it goes unread
//! to a snapshot decoder.

use std::collections::VecDeque;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::address::Address;

pub use protocol::snapshot::MAX_SNAPSHOT_BYTES;

/// The folder in the settings folder that holds the store.
pub const FOLDER: &str = "snapshots";
/// The first line of a snapshot file: its format and version.
pub const VERSION_LINE: &str = "neoteletext-snapshot 1";
/// The end of a snapshot file's name, after the key's hash.
const SUFFIX: &str = ".snapshot";
/// Lowercase hexadecimal digits, for a file's name.
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
/// The digits of a SHA-256 in hexadecimal.
const HASH_DIGITS: usize = 64;
/// A file's name's length.
const FILE_NAME_BYTES: usize = HASH_DIGITS + SUFFIX.len();
/// The temporary a snapshot is written through before its rename.
const TEMPORARY: &str = "snapshot.new";
/// The page number a place has when its address names none: the
/// channel's initial page (section 11).
pub const INITIAL_PAGE: u16 = 0x000;
/// The page a snapshot of the channel's initial page asks for: page 100,
/// the page a receiver shows first when a service names none (EN 300 706
/// clause 9.3.1.1 names 100 the lowest display page; the engine's
/// navigation opens it). The key stays 000, so that a later address
/// naming 100 is a place of its own.
pub const INITIAL_PAGE_ASKED: u16 = 0x100;
/// The sub-code of the page as a whole, 3F7F (EN 300 706 Annex A.1).
pub const WHOLE_PAGE: u16 = 0x3F7F;
/// The bytes of a teletext packet: a body is whole packets.
const PACKET_BYTES: usize = neoteletext_core::consts::PACKET_BYTES;
/// The most digits of a time fetched (seconds since 1970 as an `i64`).
const MAX_TIME_DIGITS: usize = 19;
/// The digits of the longest body's length.
const MAX_LENGTH_DIGITS: usize = 7;
/// The first length with more digits than [`MAX_LENGTH_DIGITS`].
const LENGTH_DIGITS_PASSED: usize = 10_000_000;
const _: () = assert!(MAX_SNAPSHOT_BYTES < LENGTH_DIGITS_PASSED, "a body's length fits its digits");
/// The key line at its longest: page, tab, sub-code, tab, address, line feed.
const MAX_KEY_LINE: usize = 3 + 1 + 4 + 1 + protocol::MAX_ADDRESS + 1;
/// The third line at its longest: time, tab, sub-code, tab, length, line feed.
const MAX_THIRD_LINE: usize = MAX_TIME_DIGITS + 1 + 4 + 1 + MAX_LENGTH_DIGITS + 1;
/// The longest head: the three lines.
const MAX_HEAD: usize = VERSION_LINE.len() + 1 + MAX_KEY_LINE + MAX_THIRD_LINE;
/// The longest snapshot file: the body's bound and the three lines at
/// their longest (REQ-6886 item 4).
pub const MAX_SNAPSHOT_FILE: usize = MAX_HEAD + MAX_SNAPSHOT_BYTES;
/// The most snapshots kept: one for each of the session's tabs.
pub const MAX_SNAPSHOTS: usize = protocol::MAX_TABS;
/// The most bytes the store's files take, all together: 32 MiB, the
/// specification's figure, about sixteen of the largest pictured sub-pages
/// and thousands of plain ones (provisional, DEC-6875 item 7).
pub const MAX_SNAPSHOT_STORE_BYTES: u64 = 32 * 1024 * 1024;
/// Snapshot fetches at once in the browser (section 8), and to any one
/// host: a few at a time, so no server is asked for many pages at once.
pub const SNAPSHOT_FETCHES_AT_ONCE: usize = 2;
/// See [`SNAPSHOT_FETCHES_AT_ONCE`].
pub const SNAPSHOT_FETCHES_PER_HOST: usize = 1;
/// Snapshot decoders at once.
pub const SNAPSHOT_DRAWS_AT_ONCE: usize = 2;
/// How old a stored snapshot is before it is fetched again while its tab
/// is in view: one hour.
pub const SNAPSHOT_REFRESH: Duration = Duration::from_hours(1);
/// How long a miss is remembered before its place is asked again: fifteen
/// minutes, in memory only.
pub const SNAPSHOT_MISS_RETRY: Duration = Duration::from_mins(15);
/// The most misses remembered; the oldest is forgotten first.
pub const MAX_MISSES_REMEMBERED: usize = 256;
/// How long a snapshot fetcher has before it is ended, a miss: above the
/// engine's `SNAPSHOT_TIME_LIMIT_US` of 15 seconds, with the process's
/// start.
pub const SNAPSHOT_FETCH_LIMIT: Duration = Duration::from_secs(20);
const _: () = assert!(SNAPSHOT_FETCH_LIMIT.as_micros() > neoteletext_web::consts::SNAPSHOT_TIME_LIMIT_US as u128, "the fetcher ends itself first");
/// How long a snapshot decoder has before it is ended, a miss.
pub const SNAPSHOT_DRAW_LIMIT: Duration = Duration::from_secs(5);

/// A snapshot's place: the channel, the page asked for (000 for the
/// channel's initial page) and the sub-code asked for (3F7F for the page
/// as a whole), always together (DEC-6599).
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Place {
    /// The channel's address, as the address rules print it.
    pub channel: String,
    /// The page, or [`INITIAL_PAGE`].
    pub page: u16,
    /// The sub-code, or [`WHOLE_PAGE`].
    pub sub: u16,
}

impl Place {
    /// The place a tab at `address` shows, when a snapshot can be had of
    /// it: a web channel's, never a file's or a built-in page's.
    #[must_use]
    pub fn of(address: &Address) -> Option<Self> {
        if address.file.is_some() || address.is_built_in() {
            return None;
        }
        let page = address.page.unwrap_or(INITIAL_PAGE);
        let sub = address.page.and(address.subpage).unwrap_or(WHOLE_PAGE);
        Some(Self { channel: address.channel_text().to_owned(), page, sub })
    }

    /// The page a snapshot of the place asks for.
    #[must_use]
    pub const fn page_asked(&self) -> u16 {
        if self.page == INITIAL_PAGE { INITIAL_PAGE_ASKED } else { self.page }
    }

    /// The key line, without its line feed.
    #[must_use]
    pub fn key_line(&self) -> String {
        format!("{:03X}\t{:04X}\t{}", self.page, self.sub, self.channel)
    }

    /// The file's name: the key line's SHA-256, so that no address
    /// appears in a file name.
    #[must_use]
    pub fn file_name(&self) -> String {
        let digest = ring::digest::digest(&ring::digest::SHA256, self.key_line().as_bytes());
        let mut name = String::with_capacity(FILE_NAME_BYTES);
        for byte in digest.as_ref() {
            name.push(char::from(HEX_DIGITS.get(usize::from(byte >> 4)).copied().unwrap_or(b'0')));
            name.push(char::from(HEX_DIGITS.get(usize::from(byte & 0x0F)).copied().unwrap_or(b'0')));
        }
        name.push_str(SUFFIX);
        name
    }
}

/// A snapshot as stored.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// When it was fetched, in seconds since 1970 (UTC).
    pub fetched: i64,
    /// The sub-code drawn.
    pub drawn: u16,
    /// The blanked source, whole packets.
    pub body: Vec<u8>,
}

/// Why a snapshot file was refused (each is refused whole and deleted).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// The version line is not format 1's.
    Version,
    /// The key line is not the place asked for, or not an address the
    /// address rules accept.
    Key,
    /// The time, the sub-code drawn or the length is malformed.
    Head,
    /// The body is not the length said, not whole packets, or past the bound.
    Body,
}

/// A number of exactly `digits` hexadecimal capitals or digits.
fn hexadecimal(text: &str, digits: usize) -> Option<u16> {
    (text.len() == digits && text.bytes().all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte)))
        .then(|| u16::from_str_radix(text, 16).ok())
        .flatten()
}

/// A decimal number of 1 to `digits` digits, without a sign or a leading
/// zero (but 0 itself).
fn decimal<T: core::str::FromStr>(text: &str, digits: usize) -> Option<T> {
    let well_formed = !text.is_empty() && text.len() <= digits && text.bytes().all(|byte| byte.is_ascii_digit()) && (text == "0" || !text.starts_with('0'));
    if well_formed { text.parse().ok() } else { None }
}

/// A snapshot's file, format 1 (section 11).
///
/// # Errors
/// When the body is past the bound or not whole packets.
pub fn encode(place: &Place, snapshot: &Snapshot) -> Result<Vec<u8>, Refused> {
    if snapshot.body.len() > MAX_SNAPSHOT_BYTES || !snapshot.body.len().is_multiple_of(PACKET_BYTES) || snapshot.body.is_empty() {
        return Err(Refused::Body);
    }
    if !names_one_subpage(snapshot.drawn) || snapshot.fetched < 0 {
        return Err(Refused::Head);
    }
    let mut bytes = format!("{VERSION_LINE}\n{}\n{}\t{:04X}\t{}\n", place.key_line(), snapshot.fetched, snapshot.drawn, snapshot.body.len()).into_bytes();
    bytes.extend_from_slice(&snapshot.body);
    Ok(bytes)
}

/// A sub-code drawn: one sub-page's, 0000 to 3F7E.
const fn names_one_subpage(sub: u16) -> bool {
    sub < WHOLE_PAGE && neoteletext_core::types::SubCode::new(sub).value() == sub
}

/// The three lines of a file and where its body starts.
fn split_head(bytes: &[u8]) -> Option<([&str; 3], usize)> {
    let mut lines = [""; 3];
    let mut at = 0_usize;
    for line in &mut lines {
        let rest = bytes.get(at..)?;
        let end = rest.iter().take(MAX_KEY_LINE).position(|&byte| byte == b'\n')?;
        *line = core::str::from_utf8(rest.get(..end)?).ok()?;
        at = at.checked_add(end)?.checked_add(1)?;
    }
    Some((lines, at))
}

/// Reads a snapshot file of `place`, strictly.
///
/// # Errors
/// What is wrong with it: the whole file is then refused.
pub fn decode(place: &Place, bytes: &[u8]) -> Result<Snapshot, Refused> {
    if bytes.len() > MAX_SNAPSHOT_FILE {
        return Err(Refused::Body);
    }
    let Some(([version, key, third], body_at)) = split_head(bytes) else { return Err(Refused::Head) };
    if version != VERSION_LINE {
        return Err(Refused::Version);
    }
    if key != place.key_line() || crate::address::parse(&place.channel).map_or(true, |parsed| parsed.channel_text() != place.channel) {
        return Err(Refused::Key);
    }
    let mut fields = third.split('\t');
    let (Some(fetched), Some(drawn), Some(length), None) = (fields.next(), fields.next(), fields.next(), fields.next()) else { return Err(Refused::Head) };
    let fetched: i64 = decimal(fetched, MAX_TIME_DIGITS).ok_or(Refused::Head)?;
    let drawn = hexadecimal(drawn, 4).filter(|&drawn| names_one_subpage(drawn)).ok_or(Refused::Head)?;
    let length: usize = decimal(length, MAX_LENGTH_DIGITS).ok_or(Refused::Head)?;
    let body = bytes.get(body_at..).ok_or(Refused::Body)?;
    if length == 0 || length > MAX_SNAPSHOT_BYTES || !length.is_multiple_of(PACKET_BYTES) || body.len() != length {
        return Err(Refused::Body);
    }
    Ok(Snapshot { fetched, drawn, body: body.to_vec() })
}

/// Opens a kept file to read, never through a link (finding F-35).
fn open_private(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW);
    }
    options.open(path)
}

/// Reads at most `max` bytes of the file at `path`; `None` when it is not
/// there, cannot be read, or is longer.
fn read_within(path: &Path, max: usize) -> Option<Vec<u8>> {
    let file = open_private(path).ok()?;
    let mut bytes = Vec::new();
    file.take(u64::try_from(max).ok()?.saturating_add(1)).read_to_end(&mut bytes).ok()?;
    (bytes.len() <= max).then_some(bytes)
}

/// Whether `name` is a snapshot file's: 64 lowercase hexadecimal digits
/// and the suffix.
fn is_snapshot_name(name: &str) -> bool {
    name.strip_suffix(SUFFIX).is_some_and(|hash| hash.len() == HASH_DIGITS && hash.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
}

/// A file of the store, as its head says.
struct Kept {
    name: String,
    fetched: i64,
    bytes: u64,
}

/// The store (REQ-6886): the folder `snapshots` in the settings folder,
/// or nothing kept when there is none.
#[derive(Debug, Default)]
pub struct Store {
    folder: Option<PathBuf>,
}

impl Store {
    /// The store in `settings_folder`; none kept without one.
    #[must_use]
    pub fn new(settings_folder: Option<&Path>) -> Self {
        Self { folder: settings_folder.map(|folder| folder.join(FOLDER)) }
    }

    /// Its folder.
    #[must_use]
    pub fn folder(&self) -> Option<&Path> {
        self.folder.as_deref()
    }

    /// The snapshot of `place`, if one is kept and passes every check; a
    /// file that fails one is deleted and is no snapshot.
    #[must_use]
    pub fn read(&self, place: &Place) -> Option<Snapshot> {
        let path = self.folder.as_ref()?.join(place.file_name());
        let bytes = read_within(&path, MAX_SNAPSHOT_FILE);
        if let Some(Ok(snapshot)) = bytes.as_deref().map(|bytes| decode(place, bytes)) {
            return Some(snapshot);
        }
        let _ = std::fs::remove_file(&path);
        None
    }

    /// Keeps `snapshot` of `place`, deleting the snapshots fetched longest
    /// ago while the store would pass its bounds, never this one.
    ///
    /// # Errors
    /// When the snapshot is malformed or cannot be written.
    pub fn write(&self, place: &Place, snapshot: &Snapshot) -> std::io::Result<()> {
        let Some(folder) = &self.folder else { return Ok(()) };
        let bytes = encode(place, snapshot).map_err(|refused| std::io::Error::new(std::io::ErrorKind::InvalidData, format!("{refused:?}")))?;
        let name = place.file_name();
        let mut kept: Vec<Kept> = self.kept().into_iter().filter(|kept| kept.name != name).collect();
        kept.sort_by_key(|kept| kept.fetched);
        let new = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let mut total: u64 = kept.iter().map(|kept| kept.bytes).sum();
        let mut oldest = kept.into_iter();
        let mut count = oldest.len();
        while count >= MAX_SNAPSHOTS || total.saturating_add(new) > MAX_SNAPSHOT_STORE_BYTES {
            let Some(gone) = oldest.next() else { break };
            let _ = std::fs::remove_file(folder.join(&gone.name));
            total = total.saturating_sub(gone.bytes);
            count = count.saturating_sub(1);
        }
        crate::private_file::write_whole(folder, &name, TEMPORARY, bytes)
    }

    /// Every snapshot file kept, with its head's time; anything else in
    /// the folder, or a file whose head is malformed, is deleted.
    fn kept(&self) -> Vec<Kept> {
        let Some(folder) = &self.folder else { return Vec::new() };
        let Ok(entries) = std::fs::read_dir(folder) else { return Vec::new() };
        let mut kept = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_str().map(str::to_owned);
            let file = entry.file_type().is_ok_and(|kind| kind.is_file());
            let head = name.as_deref().filter(|name| file && is_snapshot_name(name)).and_then(|_| read_within_head(&path));
            match (name, head) {
                (Some(name), Some((fetched, bytes))) => kept.push(Kept { name, fetched, bytes }),
                _ => remove(&path),
            }
        }
        kept
    }

    /// Deletes every file but those of `places`, and anything that is not
    /// a snapshot file: at start, and whenever tabs change.
    pub fn keep_only<'a>(&self, places: impl IntoIterator<Item = &'a Place>) {
        let Some(folder) = &self.folder else { return };
        let names: std::collections::BTreeSet<String> = places.into_iter().map(Place::file_name).collect();
        for kept in self.kept() {
            if !names.contains(&kept.name) {
                let _ = std::fs::remove_file(folder.join(&kept.name));
            }
        }
    }

    /// Deletes every snapshot fetched at or after `since` (history
    /// cleared in that time); all of them with `None`.
    pub fn clear(&self, since: Option<i64>) {
        let Some(folder) = &self.folder else { return };
        for kept in self.kept() {
            if since.is_none_or(|since| kept.fetched >= since) {
                let _ = std::fs::remove_file(folder.join(&kept.name));
            }
        }
    }
}

/// Removes a stray: a file, or a folder and what it holds.
fn remove(path: &Path) {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_dir() => {
            let _ = std::fs::remove_dir_all(path);
        }
        Ok(_) => {
            let _ = std::fs::remove_file(path);
        }
        Err(_) => {}
    }
}

/// A file's time fetched and size, from its head, read strictly: its
/// version line and its third line; `None` when either is malformed.
fn read_within_head(path: &Path) -> Option<(i64, u64)> {
    let size = std::fs::symlink_metadata(path).ok()?.len();
    if size > u64::try_from(MAX_SNAPSHOT_FILE).ok()? {
        return None;
    }
    let mut head = Vec::new();
    open_private(path).ok()?.take(u64::try_from(MAX_HEAD).ok()?).read_to_end(&mut head).ok()?;
    let ([version, _, third], _) = split_head(&head)?;
    if version != VERSION_LINE {
        return None;
    }
    let fetched = decimal(third.split('\t').next()?, MAX_TIME_DIGITS)?;
    Some((fetched, size))
}

/// The places that missed recently (REQ-6883), in memory only, the oldest
/// first, at most [`MAX_MISSES_REMEMBERED`].
#[derive(Debug, Default)]
pub struct Misses {
    missed: VecDeque<(Place, Instant)>,
}

impl Misses {
    /// `place` missed at `now`.
    pub fn missed(&mut self, place: &Place, now: Instant) {
        self.missed.retain(|(missed, _)| missed != place);
        self.missed.push_back((place.clone(), now));
        while self.missed.len() > MAX_MISSES_REMEMBERED {
            self.missed.pop_front();
        }
    }

    /// Whether `place` missed within [`SNAPSHOT_MISS_RETRY`] of `now`.
    #[must_use]
    pub fn recent(&self, place: &Place, now: Instant) -> bool {
        self.missed.iter().any(|(missed, at)| missed == place && now.saturating_duration_since(*at) < SNAPSHOT_MISS_RETRY)
    }

    /// Forgets every miss (history cleared).
    pub fn clear(&mut self) {
        self.missed.clear();
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]
mod tests;
