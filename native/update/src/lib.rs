//! What the launcher, the updater, the installer and the release tool
//! share (`installation-and-updates.md`, decision 54):
//!
//! - [`version`]: a release's version, which only ever increases.
//! - [`platform`]: the desktops and the files a version has on each, the
//!   only paths a release may name (section 6).
//! - [`manifest`]: a release's list of its files, their lengths and hashes,
//!   a target of The Update Framework and, on Linux, signed by the release
//!   key (section 5).
//! - [`state`]: the launcher's state file, the pointer whose atomic rename
//!   is the switch (requirement 13.4).
//! - [`gzip`]: the release files' compression, read with a bound.
//! - [`signature`]: the Linux release key's detached Ed25519 signatures.
//! - [`mark`]: the version a program carries inside its signed bytes.
//! - [`wire`]: the trusted core's messages to and from the updater.
//! - [`layout`]: where each desktop keeps an installed browser.
//! - [`files`]: writes that survive a power cut, and the folder exchange.
//! - [`accept`]: the launcher's acceptance of a staged version.
//! - [`profile`]: a settings format's own file, copied forward, never
//!   rewritten (section 3.3).
//! - [`fault`]: the test builds' fault points (section 13).
//!
//! Every reader here reads input from outside its process (a downloaded
//! file, a file another process wrote), so each is bounded and fuzzed.

#![deny(
    clippy::indexing_slicing,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

pub mod accept;
pub mod fault;
pub mod files;
pub mod gzip;
pub mod layout;
pub mod manifest;
pub mod mark;
pub mod platform;
pub mod profile;
pub mod signature;
pub mod state;
pub mod version;
pub mod wire;

/// The bytes of SHA-256, which names every release file (section 5).
pub const SHA256_BYTES: usize = 32;

/// SHA-256 of `bytes`.
#[must_use]
pub fn sha256(bytes: &[u8]) -> [u8; SHA256_BYTES] {
    let digest = ring::digest::digest(&ring::digest::SHA256, bytes);
    let mut out = [0_u8; SHA256_BYTES];
    out.copy_from_slice(digest.as_ref());
    out
}

/// Lower-case hexadecimal, as the manifest writes a hash.
#[must_use]
pub fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut text = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        let _ = write!(text, "{byte:02x}");
    }
    text
}

/// Bytes from lower-case hexadecimal of exactly `N` bytes; anything else,
/// upper case included, is refused, so each hash has one spelling.
#[must_use]
pub fn from_hex<const N: usize>(text: &str) -> Option<[u8; N]> {
    let digits = text.as_bytes();
    if digits.len() != N.checked_mul(2)? {
        return None;
    }
    let mut out = [0_u8; N];
    let value = |digit: u8| match digit {
        b'0'..=b'9' => Some(digit.wrapping_sub(b'0')),
        b'a'..=b'f' => Some(digit.wrapping_sub(b'a').wrapping_add(10)),
        _ => None,
    };
    for (byte, &[high, low]) in out.iter_mut().zip(digits.as_chunks::<2>().0) {
        *byte = value(high)?.checked_mul(16)?.checked_add(value(low)?)?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hexadecimal_has_one_spelling() {
        assert_eq!(hex(&[0x00, 0xab, 0xff]), "00abff");
        assert_eq!(from_hex::<3>("00abff"), Some([0x00, 0xab, 0xff]));
        assert_eq!(from_hex::<3>("00ABFF"), None, "upper case");
        assert_eq!(from_hex::<3>("00abf"), None, "short");
        assert_eq!(from_hex::<3>("00abfff0"), None, "long");
        assert_eq!(from_hex::<1>("g0"), None);
    }

    #[test]
    fn sha256_of_nothing_is_the_standard_value() {
        // FIPS 180-4's SHA-256 of the empty message.
        assert_eq!(hex(&sha256(b"")), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }
}
