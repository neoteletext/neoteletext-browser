//! The broker's store of one secret, the viewer key file's sealing key
//! (neoteletext-common's `docs/viewer-state.md` VS-108), in the platform's
//! own secret store (DEC-7168):
//!
//! - macOS: a generic password in the user's default keychain, through
//!   Security.framework's `SecItemCopyMatching`, `SecItemAdd`,
//!   `SecItemUpdate` and `SecItemDelete` (Apple's "Keychain Services");
//! - Windows: a generic credential in Credential Manager, through
//!   `CredReadW`, `CredWriteW`, `CredDeleteW` and `CredFree` (Microsoft's
//!   `wincred.h`);
//! - Linux and the other Unix desktops: an item of the default collection
//!   of the freedesktop Secret Service (the Secret Service API, version
//!   0.2), over the browser's own D-Bus client.
//!
//! An entry is named by a service and an account, and each platform keeps
//! it where `keyring` 4.2.0 kept it, which the broker used before, so a key
//! stored then is read now: the keychain's service and account; the
//! credential `<account>.<service>`; the Secret Service attributes
//! `service` and `username`.
//!
//! The secret is never written into an error or a log, and every buffer of
//! this crate's that held it is set to zero before it is freed ([`Secret`]).
//! Only the broker, the one process not confined, links this crate; the
//! trusted core reaches the key only through the broker (VS-108).

// No panicking constructs: on Linux this crate reads what the session bus
// sends. The same list is in each such crate; scripts/check.sh checks that
// they agree.
#![deny(
    clippy::indexing_slicing,
    clippy::string_slice,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::arithmetic_side_effects,
    clippy::unreachable,
    clippy::todo,
    clippy::unimplemented
)]

#[cfg(target_os = "macos")]
mod macos;
#[cfg(all(unix, not(target_os = "macos")))]
mod secret_service;
#[cfg(windows)]
mod windows;

#[cfg(target_os = "macos")]
use macos as platform;
#[cfg(all(unix, not(target_os = "macos")))]
use secret_service as platform;
#[cfg(windows)]
use windows as platform;

/// The longest secret read: eight times the broker's 32-byte key. A longer
/// one is not the broker's, and is refused before it is copied.
pub const MAX_SECRET_BYTES: usize = 256;

/// A secret's bytes, set to zero when dropped.
pub struct Secret(Vec<u8>);

impl Secret {
    /// A secret of `bytes`, which it takes.
    #[must_use]
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The secret's bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// A secret copied from `bytes`, or [`Error::TooLong`] beyond
    /// [`MAX_SECRET_BYTES`].
    fn copied(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > MAX_SECRET_BYTES {
            return Err(Error::TooLong);
        }
        Ok(Self(bytes.to_vec()))
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        wipe(&mut self.0);
    }
}

/// Never the secret's bytes.
impl std::fmt::Debug for Secret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Secret({} bytes)", self.0.len())
    }
}

/// Sets `bytes` to zero in a way the optimiser keeps: `black_box` makes the
/// zeroed buffer observed, so the stores are not removed as dead before the
/// buffer is freed.
pub(crate) fn wipe(bytes: &mut [u8]) {
    bytes.fill(0);
    std::hint::black_box(&*bytes);
}

/// Why the store did not answer. None of these holds a secret's bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// There is no store to ask: no session bus or no Secret Service on
    /// Linux, no keychain on macOS.
    NoStore(String),
    /// The store refused, or the person dismissed its prompt.
    Refused(String),
    /// More than one entry has the service and account.
    Ambiguous,
    /// A secret longer than [`MAX_SECRET_BYTES`].
    TooLong,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoStore(why) => write!(f, "no secret store: {why}"),
            Self::Refused(why) => write!(f, "the secret store refused: {why}"),
            Self::Ambiguous => write!(f, "the secret store holds more than one entry of the name"),
            Self::TooLong => write!(f, "the secret store's entry is longer than {MAX_SECRET_BYTES} bytes"),
        }
    }
}

impl std::error::Error for Error {}

/// The secret of `service` and `account`, or none when there is no entry.
///
/// # Errors
///
/// When the store cannot be reached or refuses.
pub fn read(service: &str, account: &str) -> Result<Option<Secret>, Error> {
    platform::read(service, account)
}

/// Keeps `secret` as the secret of `service` and `account`, replacing one
/// there is.
///
/// # Errors
///
/// When the store cannot be reached or refuses, or `secret` is longer than
/// [`MAX_SECRET_BYTES`].
pub fn write(service: &str, account: &str, secret: &[u8]) -> Result<(), Error> {
    if secret.len() > MAX_SECRET_BYTES {
        return Err(Error::TooLong);
    }
    platform::write(service, account, secret)
}

/// Deletes the entry of `service` and `account`; none there is no error.
///
/// # Errors
///
/// When the store cannot be reached or refuses.
pub fn delete(service: &str, account: &str) -> Result<(), Error> {
    platform::delete(service, account)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;

    #[test]
    fn a_secret_is_never_shown_and_is_bounded() {
        assert_eq!(format!("{:?}", Secret::new(vec![0x41; 32])), "Secret(32 bytes)");
        assert_eq!(Secret::copied(&[7; MAX_SECRET_BYTES]).unwrap().as_bytes(), [7; MAX_SECRET_BYTES]);
        assert_eq!(Secret::copied(&[7; MAX_SECRET_BYTES + 1]).err(), Some(Error::TooLong));
        assert_eq!(write("service", "account", &[0; MAX_SECRET_BYTES + 1]), Err(Error::TooLong));
    }

    #[test]
    fn wiping_sets_every_byte_to_zero() {
        let mut bytes = vec![9_u8; 32];
        wipe(&mut bytes);
        assert_eq!(bytes, [0; 32]);
    }
}
