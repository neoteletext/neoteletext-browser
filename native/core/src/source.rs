//! A channel's data, read from a file on this computer: a recording (T42,
//! a transport stream, sliced records) or page files (TTI), or a folder of
//! page files read in name order. The trusted core never parses these
//! bytes; it hands them to the channel's decoder a piece at a time, at a
//! fixed pace, as a fetcher would hand over what it received.

use std::fs;
use std::io::Read;
use std::path::Path;

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

impl Source {
    /// Reads a file, or a folder's files in name order, up to the limit.
    ///
    /// # Errors
    /// A sentence for the log, when nothing could be read.
    pub fn read(path: &Path) -> Result<Self, String> {
        let metadata = fs::metadata(path).map_err(|error| format!("{}: {error}", path.display()))?;
        // Only files and folders: a pipe would hold the trusted core at
        // its open, and a device is not a recording.
        if !metadata.is_dir() && !metadata.is_file() {
            return Err(format!("{}: not a file or a folder", path.display()));
        }
        let mut bytes = Vec::new();
        let take = |file: &Path, bytes: &mut Vec<u8>| -> Result<(), String> {
            let room = MAX_SOURCE_BYTES.saturating_sub(bytes.len() as u64);
            let handle = fs::File::open(file).map_err(|error| format!("{}: {error}", file.display()))?;
            handle.take(room).read_to_end(bytes).map_err(|error| format!("{}: {error}", file.display()))?;
            Ok(())
        };
        if metadata.is_dir() {
            let mut files: Vec<_> = fs::read_dir(path)
                .map_err(|error| format!("{}: {error}", path.display()))?
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|file| file.is_file())
                .take(MAX_FILES)
                .collect();
            files.sort();
            for file in files {
                take(&file, &mut bytes)?;
                // Page files end their last line; a missing end of line
                // would join two files' lines.
                bytes.push(b'\n');
            }
        } else {
            take(path, &mut bytes)?;
        }
        Ok(Self { bytes, sent: 0, ended: false })
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
    fn a_missing_file_is_reported() {
        assert!(Source::read(Path::new("/nonexistent/teletext.t42")).is_err());
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
        let refused = Source::read(&socket).err().unwrap();
        assert!(refused.ends_with("not a file or a folder"), "{refused}");
        assert!(Source::read(&folder).is_ok(), "the folder itself is a channel, with the socket skipped");
        fs::remove_dir_all(&folder).unwrap();
    }
}
