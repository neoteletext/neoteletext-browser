//! A channel's data, read from a file on this computer: a T42 recording or
//! a TTI page file, or a folder of page files read in name order, the only
//! formats the browser opens (DEC-1762). The trusted core never parses these
//! bytes; it hands them to the channel's decoder at once, in pieces within
//! the protocol's bound, as a fetcher hands over what it received.

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use confine::{FileIdentity, file_identity};
use neoteletext_formats::detect::{DETECTION_BYTES, Detection, Format, detect};
use neoteletext_formats::t42::T42Variant;

/// The largest channel read from a file: far above a day's recording of a
/// teletext service, which is a few megabytes.
pub const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
/// The most page files read from one folder: a channel has at most 800
/// display pages, with sub-pages in further files.
const MAX_FILES: usize = 4096;
// DEC-1762: what the broker reads is what the decoder is built to read.
const _: () = assert!(Format::T42(T42Variant::Plain).is_compiled() && Format::Tti.is_compiled());

pub struct Source {
    bytes: Vec<u8>,
    sent: usize,
    ended: bool,
}

/// What the loader does next with a channel's data.
pub enum Next<'a> {
    Data(&'a [u8]),
    End,
    Done,
}

/// What the broker answers for every file it does not read for the trusted
/// core, whatever the reason: the same words for a missing file, a secret
/// and a file that is not teletext, so no answer says which (decision 88).
pub const REFUSAL: &str = "not a channel this browser can open";

/// The places no file channel is read from (decision 88): every entry
/// beginning with a dot under the home folder, and the folders named, each
/// with all beneath it. The browser's own folders are among them, the only
/// ones the confined trusted core writes, so no link or hard link it makes
/// is read.
pub struct Places {
    home: Option<PathBuf>,
    refused: Vec<PathBuf>,
}

impl Places {
    /// The places beneath `home` and `refused`, each by its canonical path
    /// when it exists, as the paths it is compared with are.
    #[must_use]
    pub fn new(home: Option<PathBuf>, refused: Vec<PathBuf>) -> Self {
        let canonical = |path: PathBuf| path.canonicalize().unwrap_or(path);
        Self { home: home.map(canonical), refused: refused.into_iter().map(canonical).collect() }
    }

    /// Whether a canonical path lies in a refused place.
    #[must_use]
    pub fn refuses(&self, canonical: &Path) -> bool {
        let dot_entry = self.home.as_deref().and_then(|home| canonical.strip_prefix(home).ok()).is_some_and(|inside| {
            inside.components().any(|component| matches!(component, Component::Normal(name) if name.as_encoded_bytes().starts_with(b".")))
        });
        dot_entry || self.refused.iter().any(|place| canonical.starts_with(place))
    }
}

/// Opens a file without following a link anywhere in its path, and checks
/// the descriptor, not the path, so that nothing changed between the check
/// and the open is read (decision 88).
fn open_no_follow(canonical: &Path) -> Option<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    // A link at any step of the path refuses the open.
    // Without blocking, so that a pipe put in the file's place cannot hold
    // the broker at its open; it is then refused by its type.
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW_ANY | libc::O_NONBLOCK);
    }
    // A link at the last step refuses the open; one at an earlier step is
    // caught by the descriptor's own path below.
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    // A reparse point (a link or a junction) is opened as itself, and then
    // refused below by its type.
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        options.custom_flags(windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT);
    }
    let file = options.open(canonical).ok()?;
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::os::fd::AsRawFd as _;
        // The broker is not confined, so `/proc` is there.
        let opened = fs::read_link(format!("/proc/self/fd/{}", file.as_raw_fd())).ok()?;
        if opened != canonical {
            return None;
        }
    }
    let metadata = file.metadata().ok()?;
    // Only a file: a pipe would hold the broker at its read, and a device is
    // not a recording.
    (metadata.is_file() && !metadata.file_type().is_symlink()).then_some(file)
}

/// Opens a folder as [`open_no_follow`] opens a file, for its identity alone:
/// its files are read by name, each without following a link.
fn open_folder(canonical: &Path) -> Option<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW_ANY | libc::O_DIRECTORY);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY);
    }
    // A folder opens on Windows only by this flag.
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt as _;
        use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT};
        options.custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    }
    options.open(canonical).ok().filter(|folder| folder.metadata().is_ok_and(|metadata| metadata.is_dir()))
}

/// Whether the engine recognises the bytes as teletext in a format the
/// browser opens, T42 or TTI (requirement 2.1, decision 88, DEC-1762).
fn teletext(bytes: &[u8]) -> bool {
    let prefix = bytes.get(..DETECTION_BYTES).unwrap_or(bytes);
    matches!(detect(prefix, prefix.len() == bytes.len()), Detection::Found { format: Format::T42(_) | Format::Tti, .. })
}

/// Whether the bytes are a bookmarks file the trusted core may import:
/// another browser's (decision 71) or this browser's own.
fn bookmarks(bytes: &[u8]) -> bool {
    protocol::decoder::is_netscape_bookmarks(bytes) || neoteletext::bookmarks::is_bookmarks_file(bytes)
}

/// A file's bytes, at most `room` of them, when the file may be read, and
/// its identity, from the descriptor they were read through.
fn read_file(canonical: &Path, places: &Places, room: u64) -> Option<(Vec<u8>, FileIdentity)> {
    if places.refuses(canonical) {
        return None;
    }
    let mut bytes = Vec::new();
    let file = open_no_follow(canonical)?;
    let identity = file_identity(&file).ok()?;
    file.take(room).read_to_end(&mut bytes).ok()?;
    Some((bytes, identity))
}

impl Source {
    /// Reads a file, or a folder's files in name order, up to the limit,
    /// for the trusted core (decision 88): nothing in a refused place,
    /// nothing through a link, and only what the engine recognises as
    /// teletext, or a bookmarks file. A folder's other files are left out;
    /// a folder with none that is teletext is refused.
    ///
    /// # Errors
    /// [`REFUSAL`], whatever the reason.
    #[cfg(test)]
    pub fn read(path: &Path, places: &Places) -> Result<Self, &'static str> {
        let canonical = path.canonicalize().map_err(|_| REFUSAL)?;
        Self::read_as(&canonical, places, None).map(|(source, _)| source)
    }

    /// As [`Source::read`], for a canonical path: and when `expected` names a
    /// grant's file, only that file (DEC-1551 item 1). With what was read,
    /// the file's or folder's identity, from what was opened, for its grant.
    ///
    /// # Errors
    /// [`REFUSAL`], whatever the reason.
    pub fn read_as(canonical: &Path, places: &Places, expected: Option<FileIdentity>) -> Result<(Self, FileIdentity), &'static str> {
        if places.refuses(canonical) {
            return Err(REFUSAL);
        }
        let metadata = fs::symlink_metadata(canonical).map_err(|_| REFUSAL)?;
        let is_expected = |identity: FileIdentity| expected.is_none_or(|expected| expected == identity);
        if !metadata.is_dir() {
            if !metadata.is_file() || metadata.len() > MAX_SOURCE_BYTES {
                return Err(REFUSAL);
            }
            let (bytes, identity) = read_file(canonical, places, MAX_SOURCE_BYTES)
                .filter(|(bytes, identity)| is_expected(*identity) && (teletext(bytes) || bookmarks(bytes)))
                .ok_or(REFUSAL)?;
            return Ok((Self::from_bytes(bytes), identity));
        }
        let identity = open_folder(canonical).and_then(|folder| file_identity(&folder).ok()).filter(|identity| is_expected(*identity)).ok_or(REFUSAL)?;
        let mut names: Vec<_> = fs::read_dir(canonical)
            .map_err(|_| REFUSAL)?
            .filter_map(Result::ok)
            // The entry's own type: a link is never followed.
            .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
            .map(|entry| entry.file_name())
            .take(MAX_FILES)
            .collect();
        names.sort();
        let mut bytes = Vec::new();
        for name in names {
            let room = MAX_SOURCE_BYTES.saturating_sub(bytes.len() as u64);
            let Some((file, _)) = read_file(&canonical.join(name), places, room).filter(|(file, _)| teletext(file)) else { continue };
            bytes.extend(file);
            // Page files end their last line; a missing end of line
            // would join two files' lines.
            bytes.push(b'\n');
        }
        if bytes.is_empty() {
            return Err(REFUSAL);
        }
        Ok((Self::from_bytes(bytes), identity))
    }

    /// The channel's bytes, for the broker to send the core (decision 56).
    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// A channel of these bytes, written by the trusted core itself: the
    /// start page.
    #[must_use]
    pub fn from_bytes(bytes: Vec<u8>) -> Self {
        Self { bytes, sent: 0, ended: false }
    }

    /// A channel with no data, for addresses that cannot be read yet.
    #[must_use]
    pub fn empty() -> Self {
        Self { bytes: Vec::new(), sent: 0, ended: true }
    }

    /// Starts again from the first byte, for a new decoder: the file itself
    /// is what its last decoder was given. A channel with no data stays
    /// ended, so a web channel's decoder is never told its data ended.
    pub fn rewind(&mut self) {
        self.sent = 0;
        self.ended = self.ended && self.bytes.is_empty();
    }

    /// Whether [`Source::next`] has anything left to give.
    #[must_use]
    pub fn has_more(&self) -> bool {
        self.sent < self.bytes.len() || !self.ended
    }

    /// The next piece of at most `size` bytes, then the end, then nothing.
    pub fn next(&mut self, size: usize) -> Next<'_> {
        if self.sent < self.bytes.len() {
            let start = self.sent;
            self.sent = start.saturating_add(size).min(self.bytes.len());
            return Next::Data(self.bytes.get(start..self.sent).unwrap_or_default());
        }
        if self.ended {
            return Next::Done;
        }
        self.ended = true;
        Next::End
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::arithmetic_side_effects, reason = "a test")]
    use super::*;

    #[test]
    fn data_comes_in_pieces_then_ends_once() {
        let mut source = Source { bytes: vec![1, 2, 3, 4, 5], sent: 0, ended: false };
        assert!(matches!(source.next(2), Next::Data([1, 2])));
        assert!(matches!(source.next(2), Next::Data([3, 4])));
        assert!(matches!(source.next(2), Next::Data([5])));
        assert!(matches!(source.next(2), Next::End));
        assert!(matches!(source.next(2), Next::Done));
        assert!(matches!(Source::empty().next(2), Next::Done));
    }

    /// DEC-1762: a new decoder is given the file from its start, however
    /// much the last one had, and a channel with no data gives nothing.
    #[test]
    fn a_rewound_source_gives_everything_again() {
        let mut source = Source::from_bytes(vec![1, 2, 3]);
        assert!(matches!(source.next(2), Next::Data([1, 2])));
        source.rewind();
        assert!(matches!(source.next(3), Next::Data([1, 2, 3])));
        assert!(matches!(source.next(3), Next::End));
        source.rewind();
        assert!(source.has_more());
        assert!(matches!(source.next(3), Next::Data([1, 2, 3])));
        let mut empty = Source::empty();
        empty.rewind();
        assert!(!empty.has_more());
    }

    /// `bytes` bytes that look like no packet, as a recording's other
    /// contents do: zeros would read as packets whose addresses decode.
    fn noise(bytes: usize) -> Vec<u8> {
        (0..bytes)
            .scan(0x2F_u32, |state, _| {
                *state = state.wrapping_mul(1_103_515_245).wrapping_add(12_345);
                Some(state.to_be_bytes()[0])
            })
            .collect()
    }

    /// `count` records of `unit` bytes each holding, at `at`, a packet
    /// address that decodes: Hamming 8/4 of 0 twice, magazine 8, row 0.
    fn with_addresses(count: usize, unit: usize, at: usize) -> Vec<u8> {
        const ADDRESS: [u8; 2] = [0x15, 0x15];
        let mut bytes = noise(count * unit);
        for record in bytes.chunks_exact_mut(unit) {
            record[at..at + ADDRESS.len()].copy_from_slice(&ADDRESS);
        }
        bytes
    }

    /// DEC-1762: the formats the engine's `detect` knows beyond T42 and TTI
    /// are refused as anything else that is not teletext is. On the code
    /// before, the first two were read: the broker asked only whether the
    /// decoder was built with the format.
    #[test]
    fn only_t42_and_tti_are_channels() {
        /// An MPEG-2 transport stream's packet, and the byte that starts it.
        const TS_PACKET: usize = 188;
        const TS_SYNC: u8 = 0x47;
        /// A Linux sliced VBI record: its service, line and reserved words,
        /// then its data.
        const SLICED_RECORD: usize = 64;
        const SLICED_HEADER: usize = 16;
        /// `V4L2_SLICED_TELETEXT_B`, the service word of a teletext record.
        const SLICED_TELETEXT: [u8; 4] = [1, 0, 0, 0];
        let refused = |name: &str, bytes: &[u8]| {
            let folder = std::env::temp_dir().join(format!("neoteletext-source-formats-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&folder);
            fs::create_dir_all(&folder).unwrap();
            let file = folder.join(name);
            fs::write(&file, bytes).unwrap();
            let refused = Source::read(&file, &Places::new(None, Vec::new())).err();
            fs::remove_dir_all(&folder).unwrap();
            refused == Some(REFUSAL)
        };
        let mut transport = noise(TS_PACKET * 8);
        transport.iter_mut().step_by(TS_PACKET).for_each(|byte| *byte = TS_SYNC);
        assert!(refused("recording.ts", &transport), "a transport stream");
        let mut sliced = with_addresses(300, SLICED_RECORD, SLICED_HEADER);
        for record in sliced.as_chunks_mut::<SLICED_RECORD>().0 {
            record[..SLICED_TELETEXT.len()].copy_from_slice(&SLICED_TELETEXT);
        }
        assert!(refused("recording.vbi", &sliced), "sliced records");
        assert!(!refused("recording.t42", &with_addresses(300, 42, 0)), "T42");
        assert!(!refused("page.tti", b"PN,10000\nOL,1,A\n"), "TTI");
    }

    #[test]
    fn a_missing_file_is_refused() {
        assert_eq!(Source::read(Path::new("/nonexistent/teletext.t42"), &Places::new(None, Vec::new())).err(), Some(REFUSAL));
    }

    /// Only a file or a folder is a channel: a socket, like a pipe or a
    /// device, is refused before it is opened, so it can hold nothing up.
    /// Before this, the trusted core opened whatever the path named.
    #[cfg(unix)]
    #[test]
    fn a_socket_or_pipe_is_not_a_channel() {
        let folder = std::env::temp_dir().join(format!("neoteletext-source-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(&folder).unwrap();
        let socket = folder.join("teletext.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        fs::write(folder.join("100.tti"), b"PN,10000\n").unwrap();
        let places = Places::new(None, Vec::new());
        assert_eq!(Source::read(&socket, &places).err(), Some(REFUSAL));
        assert!(Source::read(&folder, &places).is_ok(), "the folder itself is a channel, with the socket skipped");
        fs::remove_dir_all(&folder).unwrap();
    }

    /// DEC-1551 item 1: a grant is of the file, not its path. A file put at
    /// the path since is refused, by what was opened, not what was looked
    /// at; so is a folder.
    #[test]
    fn a_grant_is_of_the_file_and_not_of_its_path() {
        let folder = std::env::temp_dir().join(format!("neoteletext-source-grant-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(folder.join("pages")).unwrap();
        let folder = folder.canonicalize().unwrap();
        let places = Places::new(None, Vec::new());
        let file = folder.join("100.tti");
        fs::write(&file, b"PN,10000\n").unwrap();
        fs::write(folder.join("pages/100.tti"), b"PN,10000\n").unwrap();
        let (_, identity) = Source::read_as(&file, &places, None).unwrap();
        assert!(Source::read_as(&file, &places, Some(identity)).is_ok());
        let (_, of_folder) = Source::read_as(&folder.join("pages"), &places, None).unwrap();
        assert!(Source::read_as(&folder.join("pages"), &places, Some(of_folder)).is_ok());
        assert_eq!(Source::read_as(&folder.join("pages"), &places, Some(identity)).err(), Some(REFUSAL));
        fs::remove_file(&file).unwrap();
        fs::write(folder.join("other"), b"PN,10000\n").unwrap();
        fs::write(&file, b"PN,10000\n").unwrap();
        assert_eq!(Source::read_as(&file, &places, Some(identity)).err(), Some(REFUSAL), "another file at its path");
        fs::remove_dir_all(&folder).unwrap();
    }

    /// Decision 88: a link at any step of the path is not followed by the
    /// open itself, so a folder swapped for a link after the path was
    /// resolved is not read through.
    #[cfg(unix)]
    #[test]
    fn a_link_inside_the_path_refuses_the_open() {
        let folder = std::env::temp_dir().join(format!("neoteletext-source-link-{}", std::process::id()));
        let _ = fs::remove_dir_all(&folder);
        fs::create_dir_all(folder.join("real")).unwrap();
        fs::write(folder.join("real/100.tti"), b"PN,10000\n").unwrap();
        std::os::unix::fs::symlink(folder.join("real"), folder.join("linked")).unwrap();
        let folder = folder.canonicalize().unwrap();
        assert!(open_no_follow(&folder.join("real/100.tti")).is_some());
        assert!(open_no_follow(&folder.join("linked/100.tti")).is_none());
        fs::remove_dir_all(&folder).unwrap();
    }
}
