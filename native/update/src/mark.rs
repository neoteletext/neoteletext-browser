//! The version a program carries in its own bytes
//! (`installation-and-updates.md` section 5): on macOS and Windows the
//! launcher reads a staged version's number from inside its signed files,
//! so the number it compares is covered by the platform's signature. The
//! broker (`platform::MARKED_PROGRAM`) holds one mark,
//! `\x01neoteletext-version\x01<version>\x01`, which the launcher finds by
//! a bounded search.
//!
//! The reader's own copy of the mark's start is kept reversed, so a
//! program that both carries a mark and reads one has the mark's start
//! once, in its mark.

use crate::version::{MAX_TEXT, Version};

/// The mark around a version, built by [`version_mark`] in the marked
/// program.
#[macro_export]
macro_rules! version_mark {
    ($version:expr) => {
        concat!("\u{1}neoteletext-version\u{1}", $version, "\u{1}")
    };
}

/// The identity mark around the name of the release identity a program
/// was built with (`release_identity::IDENTITY_MARK`, finding F-24):
/// [`PRODUCTION`] or [`TEST`].
#[macro_export]
macro_rules! identity_mark {
    ($identity:expr) => {
        concat!("\u{1}neoteletext-identity\u{1}", $identity, "\u{1}")
    };
}

/// The production identity's name in an identity mark.
pub const PRODUCTION: &str = "production";
/// A test identity's name in an identity mark.
pub const TEST: &str = "test";

/// The mark's start, reversed (see the module's head).
const START_REVERSED: &[u8] = b"\x01noisrev-txeteletoen\x01";
/// The identity mark's start, reversed likewise.
const IDENTITY_START_REVERSED: &[u8] = b"\x01ytitnedi-txeteletoen\x01";
/// The byte that ends the version.
const END: u8 = 0x01;

/// Why no version was found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused(pub &'static str);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

fn start() -> Vec<u8> {
    START_REVERSED.iter().rev().copied().collect()
}

/// The identity marked in `program`'s bytes: exactly one identity mark,
/// naming [`PRODUCTION`] or [`TEST`].
///
/// # Errors
/// When there is no identity mark, more than one, or one naming neither.
pub fn find_identity(program: &[u8]) -> Result<&'static str, Refused> {
    let start: Vec<u8> = IDENTITY_START_REVERSED.iter().rev().copied().collect();
    let mut found = None;
    let mut from = 0_usize;
    while let Some(at) = program.get(from..).and_then(|rest| rest.windows(start.len()).position(|window| window == start.as_slice())) {
        let begin = from.saturating_add(at).saturating_add(start.len());
        let tail = program.get(begin..).unwrap_or_default();
        let identity = [PRODUCTION, TEST]
            .into_iter()
            .find(|name| tail.get(..name.len()) == Some(name.as_bytes()) && tail.get(name.len()) == Some(&END))
            .ok_or(Refused("an identity mark naming no identity"))?;
        if found.replace(identity).is_some() {
            return Err(Refused("more than one identity mark"));
        }
        from = begin;
    }
    found.ok_or(Refused("no identity mark"))
}

/// The version marked in `program`'s bytes: exactly one mark, with a
/// version in it.
///
/// # Errors
/// When there is no mark, more than one, or a mark without a version.
pub fn find(program: &[u8]) -> Result<Version, Refused> {
    let start = start();
    let mut found = None;
    let mut from = 0_usize;
    while let Some(at) = program.get(from..).and_then(|rest| rest.windows(start.len()).position(|window| window == start.as_slice())) {
        let begin = from.saturating_add(at).saturating_add(start.len());
        let tail = program.get(begin..).unwrap_or_default();
        let text = tail
            .iter()
            .take(MAX_TEXT.saturating_add(1))
            .position(|&byte| byte == END)
            .and_then(|end| tail.get(..end))
            .ok_or(Refused("a mark without its end"))?;
        let version = std::str::from_utf8(text).ok().and_then(Version::parse).ok_or(Refused("a mark without a version"))?;
        if found.replace(version).is_some() {
            return Err(Refused("more than one version mark"));
        }
        from = begin;
    }
    found.ok_or(Refused("no version mark"))
}

/// The version in `mark`, the marked program's own (the mark is read,
/// so the linker keeps it).
///
/// # Errors
/// When the mark is not one.
pub fn own(mark: &str) -> Result<Version, Refused> {
    find(std::hint::black_box(mark).as_bytes())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    #[test]
    fn exactly_one_mark_names_the_version() {
        let mark = version_mark!("0.3.1");
        assert_eq!(own(mark), Ok(Version::new(0, 3, 1)));
        let program = [b"\x7fELF...".as_slice(), mark.as_bytes(), b"...rest"].concat();
        assert_eq!(find(&program), Ok(Version::new(0, 3, 1)));
        assert_eq!(find(b"\x7fELF no mark"), Err(Refused("no version mark")));
        let twice = [mark.as_bytes(), version_mark!("0.3.2").as_bytes()].concat();
        assert_eq!(find(&twice), Err(Refused("more than one version mark")));
        assert_eq!(find(version_mark!("0.3").as_bytes()), Err(Refused("a mark without a version")));
        let unended = &mark.as_bytes()[..mark.len() - 1];
        assert_eq!(find(unended), Err(Refused("a mark without its end")));
        assert!(mark.as_bytes().starts_with(&start()), "the reader looks for what the macro writes");
        // The reader's own bytes hold no mark: its copy is reversed.
        assert!(!START_REVERSED.windows(start().len()).any(|window| window == start().as_slice()));
    }

    #[test]
    fn exactly_one_identity_mark_names_the_identity() {
        assert_eq!(find_identity(identity_mark!("production").as_bytes()), Ok(PRODUCTION));
        assert_eq!(find_identity([b"...".as_slice(), identity_mark!("test").as_bytes(), b"..."].concat().as_slice()), Ok(TEST));
        assert_eq!(find_identity(b"no mark"), Err(Refused("no identity mark")));
        assert_eq!(find_identity(identity_mark!("productio").as_bytes()), Err(Refused("an identity mark naming no identity")));
        let twice = [identity_mark!("test"), identity_mark!("test")].concat();
        assert_eq!(find_identity(twice.as_bytes()), Err(Refused("more than one identity mark")));
        assert_eq!(find(identity_mark!("test").as_bytes()), Err(Refused("no version mark")), "the two marks are apart");
    }
}
