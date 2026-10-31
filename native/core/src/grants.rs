//! The files the broker may read for the trusted core, kept between runs
//! (DEC-1551 item 1, REQ-1698), and the file that keeps them, `grants.txt`.
//! It lives in a folder of the broker's own, beside the settings and never
//! in them: the settings folder is the one place the confined trusted core
//! writes, so a grant kept there would be its own to write. A compromised
//! core must not be able to give itself a file.
//!
//! The file is the user's own, and is still read as strictly as any kept
//! file: its lines are bounded, each must be whole, and one that breaks a
//! rule is dropped. A path may hold any character, so it is written in
//! hexadecimal, and no line is ever split by what a path holds.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use confine::FileIdentity;

use crate::private_file::write_whole;

/// The most grants kept: a viewer's files over years. The oldest goes first.
pub const MAX_GRANTS: usize = 4096;
/// The longest path a grant names: `confine::broker::MAX_TEXT`, what the
/// broker's own wire carries.
const MAX_PATH: usize = confine::broker::MAX_TEXT;
/// The longest file: every grant at its longest, and the folder last chosen.
const MAX_FILE: u64 = (MAX_GRANTS as u64 + 2) * (2 * MAX_PATH as u64 + 64);
const HEADER: &str = "neoteletext-grants 1";
const FILE_NAME: &str = "grants.txt";
const TEMPORARY_NAME: &str = "grants.txt.new";
const LAST_FOLDER: &str = "folder";
const GRANT: &str = "grant";
/// The folder the broker keeps its files in, beside the settings folder: a
/// sibling, so that the core's one writable folder does not hold it.
const FOLDER_SUFFIX: &str = "-broker";

/// A file the user chose, or that was read once for them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grant {
    /// Its canonical path.
    pub path: PathBuf,
    /// What it was when granted: a file put at the path since is not it.
    pub identity: FileIdentity,
}

/// The grants and the folder the chooser last chose.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Grants {
    /// The oldest first.
    grants: Vec<Grant>,
    /// Where the chooser starts (DEC-1549 item 1).
    pub last_folder: Option<PathBuf>,
}

/// The folder the broker keeps its files in, for the settings folder
/// `settings`.
#[must_use]
pub fn folder(settings: &Path) -> Option<PathBuf> {
    let name = settings.file_name()?;
    let mut name = name.to_os_string();
    name.push(FOLDER_SUFFIX);
    settings.parent().map(|parent| parent.join(name))
}

fn hexadecimal(path: &Path) -> Option<String> {
    let text = path.to_str()?;
    (text.len() <= MAX_PATH).then(|| {
        let mut digits = String::with_capacity(text.len().saturating_mul(2));
        for byte in text.bytes() {
            let _ = write!(digits, "{byte:02x}");
        }
        digits
    })
}

fn path_from(digits: &str) -> Option<PathBuf> {
    if digits.len() > MAX_PATH.saturating_mul(2) || !digits.len().is_multiple_of(2) {
        return None;
    }
    let bytes: Option<Vec<u8>> = digits.as_bytes().chunks(2).map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()).collect();
    let path = PathBuf::from(String::from_utf8(bytes?).ok()?);
    path.is_absolute().then_some(path)
}

impl Grants {
    /// The grant of `path`, its canonical path.
    #[must_use]
    pub fn find(&self, path: &Path) -> Option<&Grant> {
        self.grants.iter().find(|grant| grant.path == path)
    }

    /// Grants `path` afresh, as the newest, in place of any grant it had.
    pub fn record(&mut self, grant: Grant) {
        self.grants.retain(|kept| kept.path != grant.path);
        self.grants.push(grant);
        let excess = self.grants.len().saturating_sub(MAX_GRANTS);
        self.grants.drain(..excess);
    }

    /// How many there are.
    #[must_use]
    pub fn len(&self) -> usize {
        self.grants.len()
    }

    /// Whether there are none.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.grants.is_empty()
    }

    /// The file's text. A path that is not text, or is longer than a path
    /// may be, cannot be named on the broker's wire and is left out.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut text = format!("{HEADER}\n");
        if let Some(folder) = self.last_folder.as_deref().and_then(hexadecimal) {
            let _ = writeln!(text, "{LAST_FOLDER} {folder}");
        }
        for grant in &self.grants {
            if let Some(path) = hexadecimal(&grant.path) {
                let _ = writeln!(text, "{GRANT} {:x} {:x} {path}", grant.identity.volume, grant.identity.file);
            }
        }
        text
    }

    /// Reads a file's text, strictly: anything not begun by its header is
    /// no grants at all, and a line that breaks a rule is dropped and
    /// counted.
    #[must_use]
    pub fn parse(text: &str) -> (Self, usize) {
        let mut lines = text.lines();
        if lines.next() != Some(HEADER) {
            return (Self::default(), usize::from(!text.is_empty()));
        }
        let mut grants = Self::default();
        let mut dropped = 0_usize;
        for line in lines {
            let fields: Vec<&str> = line.split(' ').collect();
            match fields.as_slice() {
                [LAST_FOLDER, folder] if path_from(folder).is_some() => grants.last_folder = path_from(folder),
                [GRANT, volume, file, path] => match (u64::from_str_radix(volume, 16), u64::from_str_radix(file, 16), path_from(path)) {
                    (Ok(volume), Ok(file), Some(path)) => grants.record(Grant { path, identity: FileIdentity { volume, file } }),
                    _ => dropped = dropped.saturating_add(1),
                },
                _ => dropped = dropped.saturating_add(1),
            }
        }
        (grants, dropped)
    }

    /// The grants kept in `folder`, and how many lines were dropped. A file
    /// that is missing, unreadable or too long is no grants.
    #[must_use]
    pub fn load(folder: &Path) -> (Self, usize) {
        use std::io::Read as _;
        let Ok(file) = std::fs::File::open(folder.join(FILE_NAME)) else { return (Self::default(), 0) };
        let mut text = String::new();
        if file.take(MAX_FILE.saturating_add(1)).read_to_string(&mut text).is_err() || text.len() as u64 > MAX_FILE {
            return (Self::default(), 1);
        }
        Self::parse(&text)
    }

    /// Keeps the grants in `folder`, whole.
    ///
    /// # Errors
    /// When it cannot be written.
    pub fn save(&self, folder: &Path) -> std::io::Result<()> {
        write_whole(folder, FILE_NAME, TEMPORARY_NAME, self.to_text())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    fn grant(path: &str, file: u64) -> Grant {
        Grant { path: PathBuf::from(path), identity: FileIdentity { volume: 7, file } }
    }

    #[test]
    fn grants_round_trip_whatever_a_path_holds() {
        let mut grants = Grants { last_folder: Some(PathBuf::from("/home/a b")), ..Grants::default() };
        grants.record(grant("/home/news\nof the day é.t42", 1));
        grants.record(grant("/home/other.tti", 2));
        let (read, dropped) = Grants::parse(&grants.to_text());
        assert_eq!((read, dropped), (grants, 0));
    }

    #[test]
    fn a_line_that_breaks_a_rule_is_dropped() {
        let good = format!("{HEADER}\ngrant 7 1 2f61\n");
        assert_eq!(Grants::parse(&good).0.find(Path::new("/a")), Some(&grant("/a", 1)));
        for bad in ["grant 7 1", "grant 7 1 61", "grant 7 1 zz", "grant 7 1 612f", "grant x 1 2f61", "grant 7 1 2f61 extra", "folder 61", "another 1"] {
            let (read, dropped) = Grants::parse(&format!("{good}{bad}\n"));
            assert_eq!((read.len(), dropped), (1, 1), "{bad}");
        }
        assert_eq!(Grants::parse("grant 7 1 2f61\n"), (Grants::default(), 1), "no header");
        assert_eq!(Grants::parse(""), (Grants::default(), 0));
    }

    #[test]
    fn the_oldest_goes_first_and_a_new_grant_replaces_its_own() {
        let mut grants = Grants::default();
        for index in 0..=MAX_GRANTS {
            grants.record(grant(&format!("/{index}"), 1));
        }
        assert_eq!(grants.len(), MAX_GRANTS);
        assert!(grants.find(Path::new("/0")).is_none());
        grants.record(grant("/1", 9));
        assert_eq!(grants.len(), MAX_GRANTS);
        assert_eq!(grants.find(Path::new("/1")).map(|kept| kept.identity.file), Some(9));
    }

    /// REQ-1698: the broker's folder is beside the settings folder, the one
    /// place the confined core writes, and not in it.
    #[test]
    fn the_grants_folder_is_not_in_the_settings_folder() {
        let settings = Path::new("/home/a/.config/neoteletext");
        let broker = folder(settings).unwrap();
        assert_eq!(broker, Path::new("/home/a/.config/neoteletext-broker"));
        assert!(!broker.starts_with(settings));
    }
}
