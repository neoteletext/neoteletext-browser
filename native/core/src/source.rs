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

    /// A channel with no data, for addresses that cannot be read yet.
    #[must_use]
    pub fn empty() -> Self {
        Self { bytes: Vec::new(), sent: 0, ended: true }
    }

    /// The next piece of at most `size` bytes, then the end, then nothing.
    pub fn next(&mut self, size: usize) -> Next<'_> {
        if self.sent < self.bytes.len() {
            let start = self.sent;
            self.sent = (start + size).min(self.bytes.len());
            return Next::Data(&self.bytes[start..self.sent]);
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
}
