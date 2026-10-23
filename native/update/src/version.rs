//! A release's version: three numbers, as Cargo writes the workspace's,
//! compared in order. Versions only increase (requirement 13.8): the
//! launcher accepts only a version newer than the one it runs.

use std::fmt;

/// The most digits a part may have: 65,535 is the largest part.
const MAX_DIGITS: usize = 5;
/// The longest version as text: three parts and two points.
pub const MAX_TEXT: usize = 3 * MAX_DIGITS + 2;

/// `major.minor.patch`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version {
    /// The first number.
    pub major: u16,
    /// The second.
    pub minor: u16,
    /// The third.
    pub patch: u16,
}

impl Version {
    /// A version from its parts.
    #[must_use]
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self { major, minor, patch }
    }

    /// Reads `text` strictly: three decimal parts with no sign, no leading
    /// zero (but `0` itself) and nothing else, so each version has one
    /// spelling and a folder's name is its version.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        if text.len() > MAX_TEXT {
            return None;
        }
        let mut parts = text.split('.');
        let (Some(major), Some(minor), Some(patch), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
            return None;
        };
        Some(Self { major: part(major)?, minor: part(minor)?, patch: part(patch)? })
    }
}

/// One part: decimal digits, no leading zero.
fn part(text: &str) -> Option<u16> {
    if text.is_empty() || text.len() > MAX_DIGITS || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    if text.len() > 1 && text.starts_with('0') {
        return None;
    }
    text.parse().ok()
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_have_one_spelling_and_compare_in_order() {
        assert_eq!(Version::parse("0.1.0"), Some(Version::new(0, 1, 0)));
        assert_eq!(Version::parse("65535.0.10"), Some(Version::new(65535, 0, 10)));
        for bad in ["", "1", "1.2", "1.2.3.4", "01.2.3", "1.2.-3", "+1.2.3", "1.2.3 ", "1..3", "65536.0.0", "1.2.a", "１.2.3"] {
            assert_eq!(Version::parse(bad), None, "{bad:?}");
        }
        assert!(Version::new(0, 2, 0) > Version::new(0, 1, 9));
        assert!(Version::new(1, 0, 0) > Version::new(0, 65535, 65535));
        assert_eq!(Version::new(0, 10, 2).to_string(), "0.10.2");
        assert_eq!(Version::parse(&Version::new(65535, 65535, 65535).to_string()).map(|version| version.to_string().len()), Some(MAX_TEXT));
    }
}
