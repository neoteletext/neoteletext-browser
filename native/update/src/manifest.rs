//! A release's manifest: its version and platform, and each of its files
//! with its length and SHA-256 (`installation-and-updates.md` sections 5
//! and 6). It is a target of The Update Framework, so the updater stages
//! only what the targets key named; it is the launcher's list of what a
//! complete version is; and on Linux it is signed by the release key and
//! is where the launcher reads the version (section 5).
//!
//! ```text
//! neoteletext-release 1
//! version 0.2.0
//! platform linux-x86_64
//! file neoteletext 4718592 3a7bd3e2360a3d29eea436fcfb7e44c735d117c42d1c1835420b6b9942dd4f1b
//! ```
//!
//! Read strictly: the lines in this order, the files in byte order of
//! their paths and each once, every path one the platform's versions have
//! (`platform`), every file a version needs present, and nothing else.

use crate::platform::{MAX_FILES, Platform};
use crate::version::Version;
use crate::{SHA256_BYTES, from_hex, hex};

/// The first line: the format and its version.
const HEADER: &str = "neoteletext-release 1";
/// The largest file a release may have: the browser's programs are a few
/// megabytes each (`installation-and-updates.md` section 2: 11.3 MB for
/// all five), so 64 MiB is far beyond any and still bounds what is held.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// The longest path a release names: the longest of `platform`'s is under
/// 60 bytes.
const MAX_PATH: usize = 200;
/// The longest manifest: every file at the longest line, rounded up.
pub const MAX_MANIFEST: usize = 16 * 1024;
const _: () = assert!(MAX_FILES * (MAX_PATH + 100) + 100 <= MAX_MANIFEST, "the bound holds every file at its longest");

/// One file of the release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// Its path in the version's folder.
    pub path: String,
    /// Its length in bytes.
    pub length: u64,
    /// Its SHA-256.
    pub sha256: [u8; SHA256_BYTES],
}

/// A release's manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    /// The release's version.
    pub version: Version,
    /// The platform it is for.
    pub platform: Platform,
    /// Its files, in byte order of their paths.
    pub files: Vec<Entry>,
}

/// Why a manifest was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Refused(pub String);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn refuse<T>(why: impl Into<String>) -> Result<T, Refused> {
    Err(Refused(why.into()))
}

/// A length: decimal, no leading zero, at most [`MAX_FILE_BYTES`].
fn length(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) || (text.len() > 1 && text.starts_with('0')) {
        return None;
    }
    text.parse().ok().filter(|&length| length <= MAX_FILE_BYTES)
}

impl Manifest {
    /// Reads a manifest strictly.
    ///
    /// # Errors
    /// On anything the format does not allow.
    pub fn parse(bytes: &[u8]) -> Result<Self, Refused> {
        if bytes.len() > MAX_MANIFEST {
            return refuse("the manifest is too long");
        }
        let text = std::str::from_utf8(bytes).map_err(|_| Refused(String::from("the manifest is not text")))?;
        let Some(body) = text.strip_suffix('\n') else { return refuse("the manifest does not end its last line") };
        let mut lines = body.split('\n');
        if lines.next() != Some(HEADER) {
            return refuse("not a release manifest of this format");
        }
        let version =
            lines.next().and_then(|line| line.strip_prefix("version ")).and_then(Version::parse).ok_or_else(|| Refused(String::from("no version line")))?;
        let platform = lines
            .next()
            .and_then(|line| line.strip_prefix("platform "))
            .and_then(Platform::from_name)
            .ok_or_else(|| Refused(String::from("no platform line")))?;
        let allowed = platform.files();
        let mut files: Vec<Entry> = Vec::new();
        for line in lines {
            if files.len() >= MAX_FILES {
                return refuse("too many files");
            }
            let mut fields = line.split(' ');
            let (Some("file"), Some(path), Some(size), Some(hash), None) = (fields.next(), fields.next(), fields.next(), fields.next(), fields.next()) else {
                return refuse("a line that is not a file");
            };
            if path.len() > MAX_PATH || !allowed.iter().any(|file| file.path == path) {
                return refuse(format!("a file this platform's versions do not have: {}", path.escape_debug()));
            }
            if files.last().is_some_and(|last| last.path.as_str() >= path) {
                return refuse("the files are not each once in order");
            }
            let length = length(size).ok_or_else(|| Refused(String::from("a length that is not one")))?;
            let sha256 = from_hex::<SHA256_BYTES>(hash).ok_or_else(|| Refused(String::from("a hash that is not SHA-256 in lower-case hexadecimal")))?;
            files.push(Entry { path: path.to_owned(), length, sha256 });
        }
        if let Some(missing) = allowed.iter().find(|file| file.required && !files.iter().any(|entry| entry.path == file.path)) {
            return refuse(format!("the release lacks {}", missing.path));
        }
        Ok(Self { version, platform, files })
    }

    /// The manifest as its file holds it.
    #[must_use]
    pub fn to_text(&self) -> String {
        use std::fmt::Write as _;
        let mut text = format!("{HEADER}\nversion {}\nplatform {}\n", self.version, self.platform);
        for file in &self.files {
            let _ = writeln!(text, "file {} {} {}", file.path, file.length, hex(&file.sha256));
        }
        text
    }

    /// The entry for `path`.
    #[must_use]
    pub fn entry(&self, path: &str) -> Option<&Entry> {
        self.files.iter().find(|entry| entry.path == path)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::*;

    /// A complete manifest for `platform`.
    pub(crate) fn complete(platform: Platform) -> Manifest {
        let files =
            platform.files().into_iter().filter(|file| file.required).map(|file| Entry { path: file.path, length: 3, sha256: [7; SHA256_BYTES] }).collect();
        Manifest { version: Version::new(0, 2, 0), platform, files }
    }

    #[test]
    fn a_manifest_round_trips_on_every_platform() {
        for platform in Platform::ALL {
            let manifest = complete(platform);
            assert_eq!(Manifest::parse(manifest.to_text().as_bytes()), Ok(manifest));
        }
    }

    #[test]
    fn anything_the_format_does_not_allow_is_refused() {
        let good = complete(Platform::LinuxX86_64).to_text();
        let refused = |text: &str| Manifest::parse(text.as_bytes()).is_err();
        assert!(!refused(&good));
        assert!(refused(good.trim_end()), "no last line end");
        assert!(refused(&good.replace("neoteletext-release 1", "neoteletext-release 2")), "another format");
        assert!(refused(&good.replace("version 0.2.0", "version 0.2")), "a version that is not one");
        assert!(refused(&good.replace("platform linux-x86_64", "platform linux-riscv")), "an unknown platform");
        assert!(refused(&good.replace("file neoteletext ", "file ../neoteletext ")), "a path outside the version");
        assert!(refused(&good.replace("file neoteletext ", "file /etc/passwd ")), "an absolute path");
        assert!(refused(&good.replace("\nfile neoteletext.apparmor", "\nfile neoteletext.apparmor.missing")), "a file no version has");
        let lines: Vec<&str> = good.lines().collect();
        let without = |index: usize| lines.iter().enumerate().filter(|&(at, _)| at != index).map(|(_, line)| *line).collect::<Vec<_>>().join("\n") + "\n";
        assert!(refused(&without(3)), "a required file missing");
        let mut swapped = lines.clone();
        swapped.swap(3, 4);
        assert!(refused(&(swapped.join("\n") + "\n")), "out of order");
        let mut twice = lines.clone();
        twice.insert(4, lines[3]);
        assert!(refused(&(twice.join("\n") + "\n")), "a file twice");
        assert!(refused(&good.replacen(" 3 ", " 03 ", 1)), "a leading zero");
        assert!(refused(&good.replacen(" 3 ", &format!(" {} ", MAX_FILE_BYTES + 1), 1)), "too large");
        assert!(refused(&good.replacen("0707", "0G07", 1)), "not hexadecimal");
        assert!(refused(&good.replacen(&"07".repeat(32), &"07".repeat(31), 1)), "a short hash");
        assert!(refused(&good.replace('\n', "\r\n")), "carriage returns");
        assert!(refused(&format!("{good}file x 1 {}\n", "00".repeat(32))), "after the last");
        assert!(refused(&"x".repeat(MAX_MANIFEST + 1)), "too long");
    }
}
