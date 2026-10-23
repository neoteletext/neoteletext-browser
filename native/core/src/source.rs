//! A channel's data, read from a file on this computer: a recording (T42,
//! a transport stream, sliced records) or page files (TTI), or a folder of
//! page files read in name order. The trusted core never parses these
//! bytes; it hands them to the channel's decoder a piece at a time, at a
//! fixed pace, as a fetcher would hand over what it received.

use std::fs;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use neoteletext_formats::detect::{DETECTION_BYTES, Detection, detect};

/// The largest channel read from a file: far above a day's recording of a
/// teletext service, which is a few megabytes.
pub const MAX_SOURCE_BYTES: u64 = 64 * 1024 * 1024;
/// The most page files read from one folder: a channel has at most 800
/// display pages, with sub-pages in further files.
const MAX_FILES: usize = 4096;

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

/// Whether the engine recognises the bytes as teletext in a format the
/// decoder is built with (requirement 2.1, decision 88).
fn teletext(bytes: &[u8]) -> bool {
    let prefix = bytes.get(..DETECTION_BYTES).unwrap_or(bytes);
    matches!(detect(prefix, prefix.len() == bytes.len()), Detection::Found { format, .. } if format.is_compiled())
}

/// Whether the bytes are a bookmarks file the trusted core may import:
/// another browser's (decision 71) or this browser's own.
fn bookmarks(bytes: &[u8]) -> bool {
    protocol::decoder::is_netscape_bookmarks(bytes) || neoteletext::bookmarks::is_bookmarks_file(bytes)
}

/// A file's bytes, at most `room` of them, when the file may be read.
fn read_file(canonical: &Path, places: &Places, room: u64) -> Option<Vec<u8>> {
    if places.refuses(canonical) {
        return None;
    }
    let mut bytes = Vec::new();
    open_no_follow(canonical)?.take(room).read_to_end(&mut bytes).ok()?;
    Some(bytes)
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
    pub fn read(path: &Path, places: &Places) -> Result<Self, &'static str> {
        let canonical = path.canonicalize().map_err(|_| REFUSAL)?;
        if places.refuses(&canonical) {
            return Err(REFUSAL);
        }
        let metadata = fs::symlink_metadata(&canonical).map_err(|_| REFUSAL)?;
        if !metadata.is_dir() {
            if !metadata.is_file() || metadata.len() > MAX_SOURCE_BYTES {
                return Err(REFUSAL);
            }
            let bytes = read_file(&canonical, places, MAX_SOURCE_BYTES).filter(|bytes| teletext(bytes) || bookmarks(bytes)).ok_or(REFUSAL)?;
            return Ok(Self::from_bytes(bytes));
        }
        let mut names: Vec<_> = fs::read_dir(&canonical)
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
            let Some(file) = read_file(&canonical.join(name), places, room).filter(|file| teletext(file)) else { continue };
            bytes.extend(file);
            // Page files end their last line; a missing end of line
            // would join two files' lines.
            bytes.push(b'\n');
        }
        if bytes.is_empty() {
            return Err(REFUSAL);
        }
        Ok(Self::from_bytes(bytes))
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
