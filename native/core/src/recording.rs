//! A recording of one tab's traffic (REQ-6406, DEC-6405): every message
//! its fetcher gave the trusted core and every message the trusted core
//! gave its fetcher, in order, each with its time, written to a text
//! file while the developer console is open.
//!
//! The file is the user's alone, in the folder `recordings` of the
//! browser's data folder. A line beginning `#` is the header or a mark;
//! every other line is the milliseconds since the recording began, `<`
//! for received or `>` for sent, a kind, and the bytes in hexadecimal or
//! a report's text.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::calendar::{SECONDS_PER_DAY, civil_from_days};
use crate::private_file::{make_folder, private_options};

/// The size at which a recording stops by itself: 64 MiB, far more than a
/// session of pages and keys, and a bound on what an open console can
/// write to the user's disk.
pub const MAX_RECORDING_BYTES: u64 = 64 * 1024 * 1024;
/// The recordings kept: when one starts, the oldest beyond these go.
pub const KEPT_RECORDINGS: usize = 20;
/// The most recordings begun within one second that are given a name: far
/// more than a person can start, and a bound on the names tried.
const SAME_SECOND_RECORDINGS: u32 = 100;
/// The folder of recordings, in the browser's data folder.
pub const FOLDER: &str = "recordings";
/// A recording's file name ends in a dot and this.
pub const EXTENSION: &str = "ntrec";
/// The first line of every recording: the format and its version.
pub const FORMAT_LINE: &str = "# NeoTeletext recording 1";
/// The most characters of a report's text kept in one entry.
const MAX_TEXT: usize = 2_000;
/// The most characters of a host kept in a file's name.
const MAX_HOST: usize = 80;
const SECONDS_PER_HOUR: i64 = 3_600;
const SECONDS_PER_MINUTE: i64 = 60;

/// Which way an entry went.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    /// From the tab's fetcher to the trusted core.
    Received,
    /// From the trusted core to the tab's fetcher.
    Sent,
}

/// What an entry carries.
#[derive(Clone, Copy, Debug)]
pub enum Payload<'a> {
    /// Bytes, written in hexadecimal.
    Bytes(&'a [u8]),
    /// A report or a request, written as its text on one line.
    Text(&'a str),
}

/// An open recording.
#[derive(Debug)]
pub struct Recording {
    file: std::fs::File,
    path: PathBuf,
    /// The tab's channel.
    channel: u32,
    /// The trusted core's clock when it began, in milliseconds.
    began_ms: u64,
    /// The bytes written to the file so far.
    bytes: u64,
    paused: bool,
}

/// `utc` seconds since 1970 as `2026-11-16T02-10-11Z`: a time that sorts
/// as text and holds no character a file's name cannot.
#[must_use]
pub fn stamp(utc: i64) -> String {
    let days = utc.div_euclid(SECONDS_PER_DAY);
    let second = utc.rem_euclid(SECONDS_PER_DAY);
    let (year, month, day) = civil_from_days(days).unwrap_or((1970, 1, 1));
    let (hour, minute, second) =
        (second.div_euclid(SECONDS_PER_HOUR), second.rem_euclid(SECONDS_PER_HOUR).div_euclid(SECONDS_PER_MINUTE), second.rem_euclid(SECONDS_PER_MINUTE));
    format!("{year:04}-{month:02}-{day:02}T{hour:02}-{minute:02}-{second:02}Z")
}

/// A host as a file's name holds it: its letters, digits, dots and
/// hyphens, no more than [`MAX_HOST`], and never empty.
fn file_host(host: &str) -> String {
    let kept: String = host.chars().filter(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '-')).take(MAX_HOST).collect();
    let kept = kept.trim_matches('.');
    if kept.is_empty() { String::from("tab") } else { kept.to_owned() }
}

/// Text on one line: line breaks and other control characters become
/// spaces, and no more than [`MAX_TEXT`] characters are kept.
fn one_line(text: &str) -> String {
    text.chars().take(MAX_TEXT).map(|character| if character.is_control() { ' ' } else { character }).collect()
}

/// Deletes the oldest recordings in `folder` so that at most `keep`
/// remain; nothing else in the folder is touched. Names sort by time.
fn prune(folder: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    let mut recordings: Vec<PathBuf> =
        entries.filter_map(Result::ok).map(|entry| entry.path()).filter(|path| path.extension().is_some_and(|extension| extension == EXTENSION)).collect();
    recordings.sort();
    let excess = recordings.len().saturating_sub(keep);
    for path in recordings.iter().take(excess) {
        let _ = std::fs::remove_file(path);
    }
}

impl Recording {
    /// Starts a recording of `channel` in `data`'s folder of recordings:
    /// makes the folder, deletes the oldest recordings beyond those kept,
    /// and writes the header.
    ///
    /// # Errors
    /// When the folder or the file cannot be made or written.
    pub fn start(data: &Path, channel: u32, host: &str, address: &str, version: &str, now_ms: u64, utc: i64) -> std::io::Result<Self> {
        let folder = data.join(FOLDER);
        make_folder(&folder)?;
        prune(&folder, KEPT_RECORDINGS.saturating_sub(1));
        let began = stamp(utc);
        // A second recording begun within the second has the first one's
        // stamp, so it takes the next number after it: the user may stop one
        // and start another at once.
        let host = file_host(host);
        let mut made = None;
        for copy in 1..=SAME_SECOND_RECORDINGS {
            let name = if copy == 1 { format!("{began}-{host}.{EXTENSION}") } else { format!("{began}-{copy}-{host}.{EXTENSION}") };
            let path = folder.join(name);
            match private_options().create_new(true).open(&path) {
                Ok(file) => {
                    made = Some((file, path));
                    break;
                }
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(error),
            }
        }
        let (file, path) = made.ok_or_else(|| std::io::Error::from(std::io::ErrorKind::AlreadyExists))?;
        let mut recording = Self { file, path, channel, began_ms: now_ms, bytes: 0, paused: false };
        recording.write(&format!("{FORMAT_LINE}\n# browser {}\n# address {}\n# started {began}\n", one_line(version), one_line(address)))?;
        Ok(recording)
    }

    /// The tab's channel.
    #[must_use]
    pub const fn channel(&self) -> u32 {
        self.channel
    }

    /// The bytes written so far.
    #[must_use]
    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    /// Whether it is paused.
    #[must_use]
    pub const fn paused(&self) -> bool {
        self.paused
    }

    /// The file.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The file's name alone.
    #[must_use]
    pub fn name(&self) -> String {
        self.path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
    }

    fn write(&mut self, text: &str) -> std::io::Result<()> {
        self.file.write_all(text.as_bytes())?;
        self.bytes = self.bytes.saturating_add(u64::try_from(text.len()).unwrap_or(u64::MAX));
        Ok(())
    }

    fn since(&self, now_ms: u64) -> u64 {
        now_ms.saturating_sub(self.began_ms)
    }

    /// Writes one entry, unless the recording is paused. Returns whether
    /// the recording goes on: false once the file has reached
    /// [`MAX_RECORDING_BYTES`], which is marked in it.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn entry(&mut self, now_ms: u64, direction: Direction, kind: &str, payload: Payload<'_>) -> std::io::Result<bool> {
        if self.paused {
            return Ok(true);
        }
        let arrow = match direction {
            Direction::Received => '<',
            Direction::Sent => '>',
        };
        let mut line = format!("{} {arrow} {kind} ", self.since(now_ms));
        match payload {
            Payload::Bytes(bytes) => {
                line.reserve(bytes.len().saturating_mul(2));
                for byte in bytes {
                    // Writing to a String cannot fail.
                    let _ = write!(line, "{byte:02x}");
                }
            }
            Payload::Text(text) => line.push_str(&one_line(text)),
        }
        line.push('\n');
        self.write(&line)?;
        if self.bytes >= MAX_RECORDING_BYTES {
            self.mark(now_ms, "stopped: the recording reached its largest size")?;
            return Ok(false);
        }
        Ok(true)
    }

    /// Writes a mark: a `#` line with the time and `text`.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn mark(&mut self, now_ms: u64, text: &str) -> std::io::Result<()> {
        let line = format!("# {} {}\n", self.since(now_ms), one_line(text));
        self.write(&line)
    }

    /// Pauses or resumes, marking it in the file.
    ///
    /// # Errors
    /// When the file cannot be written.
    pub fn set_paused(&mut self, paused: bool, now_ms: u64) -> std::io::Result<()> {
        if paused == self.paused {
            return Ok(());
        }
        self.paused = paused;
        self.mark(now_ms, if paused { "paused" } else { "resumed" })
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::indexing_slicing,
        clippy::arithmetic_side_effects,
        reason = "a test reports a failure by panicking, and indexes and counts what it made"
    )]

    use super::*;

    fn folder(name: &str) -> PathBuf {
        let folder = std::env::temp_dir().join(format!("neoteletext-recording-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        std::fs::create_dir_all(&folder).unwrap();
        folder
    }

    /// 16 November 2026, 02:10:11 UTC, in seconds since 1970.
    const A_TIME: i64 = 1_794_795_011;

    #[test]
    fn a_time_is_stamped_in_utc_with_nothing_a_file_name_cannot_hold() {
        assert_eq!(stamp(0), "1970-01-01T00-00-00Z");
        assert_eq!(stamp(A_TIME), "2026-11-16T02-10-11Z");
        assert_eq!(file_host("app.neottx.com"), "app.neottx.com");
        assert_eq!(file_host("../../etc/passwd"), "etcpasswd", "no path survives");
        assert_eq!(file_host("[::1]:8093"), "18093");
        assert_eq!(file_host("///"), "tab");
    }

    /// REQ-6406 items 3 to 5: the header, then each entry on a line with
    /// its time, its direction, its kind and its bytes or text.
    #[test]
    fn a_recording_has_its_header_and_an_entry_a_line_in_order() {
        let data = folder("entries");
        let mut recording = Recording::start(&data, 7, "app.neottx.com", "https://app.neottx.com/regis/", "0.1.0", 1_000, A_TIME).unwrap();
        assert!(recording.entry(1_005, Direction::Sent, "send", Payload::Bytes(&[0x82, 0x09, 0xff])).unwrap());
        assert!(recording.entry(1_012, Direction::Received, "pushed-packets", Payload::Bytes(&[0x00, 0x2a])).unwrap());
        assert!(recording.entry(1_020, Direction::Received, "report", Payload::Text("push:\nconnected")).unwrap());
        let path = recording.path().to_owned();
        assert_eq!(recording.name(), "2026-11-16T02-10-11Z-app.neottx.com.ntrec");
        assert_eq!(path.parent().unwrap(), data.join(FOLDER));
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "# NeoTeletext recording 1\n# browser 0.1.0\n# address https://app.neottx.com/regis/\n# started 2026-11-16T02-10-11Z\n5 > send 8209ff\n12 < pushed-packets 002a\n20 < report push: connected\n"
        );
        assert_eq!(recording.bytes(), u64::try_from(text.len()).unwrap());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600, "the user's alone");
            assert_eq!(std::fs::metadata(data.join(FOLDER)).unwrap().permissions().mode() & 0o777, 0o700);
        }
    }

    /// REQ-6406 item 2: paused, nothing is written but the marks.
    #[test]
    fn a_paused_recording_writes_only_its_marks() {
        let data = folder("paused");
        let mut recording = Recording::start(&data, 7, "h", "a", "v", 0, A_TIME).unwrap();
        recording.set_paused(true, 10).unwrap();
        assert!(recording.paused());
        assert!(recording.entry(20, Direction::Sent, "send", Payload::Bytes(&[1])).unwrap());
        recording.set_paused(true, 25).unwrap();
        recording.set_paused(false, 30).unwrap();
        assert!(recording.entry(40, Direction::Sent, "send", Payload::Bytes(&[2])).unwrap());
        let text = std::fs::read_to_string(recording.path()).unwrap();
        let after_header: Vec<&str> = text.lines().skip(4).collect();
        assert_eq!(after_header, ["# 10 paused", "# 30 resumed", "40 > send 02"]);
    }

    /// REQ-6406 item 6: at its largest size a recording stops, marked.
    #[test]
    fn a_recording_stops_at_its_largest_size() {
        let data = folder("bound");
        let mut recording = Recording::start(&data, 7, "h", "a", "v", 0, A_TIME).unwrap();
        // A megabyte an entry, two characters a byte: the bound is reached in a few dozen.
        let megabyte = vec![0xab_u8; 1024 * 1024];
        let mut entries = 0;
        while recording.entry(1, Direction::Received, "fetched", Payload::Bytes(&megabyte)).unwrap() {
            entries += 1;
            assert!(entries < 64, "the bound stops it");
        }
        assert!(recording.bytes() >= MAX_RECORDING_BYTES);
        let text = std::fs::read_to_string(recording.path()).unwrap();
        assert!(text.ends_with("# 1 stopped: the recording reached its largest size\n"));
    }

    /// REQ-6406 item 6: the twenty-first recording deletes the oldest, and
    /// nothing else in the folder.
    #[test]
    fn starting_a_recording_deletes_the_oldest_beyond_those_kept() {
        let data = folder("kept");
        let recordings = data.join(FOLDER);
        std::fs::create_dir_all(&recordings).unwrap();
        std::fs::write(recordings.join("notes.txt"), "the user's own").unwrap();
        let name = |index: usize| format!("{}-h.{EXTENSION}", stamp(A_TIME + i64::try_from(index).unwrap()));
        for index in 0..KEPT_RECORDINGS {
            std::fs::write(recordings.join(name(index)), "old").unwrap();
        }
        let recording = Recording::start(&data, 7, "h", "a", "v", 0, A_TIME + 1_000).unwrap();
        let kept: Vec<String> = {
            let mut names: Vec<String> =
                std::fs::read_dir(&recordings).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            names
        };
        assert_eq!(kept.iter().filter(|kept| kept.ends_with(EXTENSION)).count(), KEPT_RECORDINGS, "twenty with the new one");
        assert!(!kept.contains(&name(0)), "the oldest is gone");
        assert!(kept.contains(&name(1)) && kept.contains(&recording.name()));
        assert!(kept.contains(&String::from("notes.txt")), "nothing else is touched");
    }
}
