//! The cookie file (`cookies.md` sections 6 and 7, requirement 21.4):
//! the persistent cookies' text sealed under the key the broker holds,
//! written whole and read back.

use crate::private_file::write_whole;

use super::{MAX_FILE, Store};

/// The cookie file's name in the settings folder (`cookies.md` section 6).
pub const FILE_NAME: &str = "cookies.bin";
const TEMPORARY_NAME: &str = "cookies.bin.new";
/// The sealed file's first line: what it is and how it is sealed. It is
/// also the cipher's associated data, so a file cannot pass for another.
const SEALED_MAGIC: &[u8] = b"neoteletext-cookies-sealed 1 chacha20-poly1305\n";
/// A ChaCha20-Poly1305 key (RFC 8439 section 2.8).
pub const KEY_BYTES: usize = 32;
const NONCE_BYTES: usize = ring::aead::NONCE_LEN;
/// The largest sealed file: the largest text, its line, nonce and tag.
const MAX_SEALED: usize = SEALED_MAGIC.len() + NONCE_BYTES + MAX_FILE + ring::aead::MAX_TAG_LEN;

fn cipher(key: &[u8; KEY_BYTES]) -> Option<ring::aead::LessSafeKey> {
    ring::aead::UnboundKey::new(&ring::aead::CHACHA20_POLY1305, key).ok().map(ring::aead::LessSafeKey::new)
}

/// Seals the file's text (`cookies.md` section 7): the line, a nonce drawn
/// afresh from the system's random source for every write, then the text
/// enciphered and authenticated. A random 96-bit nonce is safe for the
/// writes one key sees: at most one a second.
#[must_use]
pub fn seal(key: &[u8; KEY_BYTES], text: &[u8]) -> Option<Vec<u8>> {
    use ring::rand::SecureRandom as _;
    let mut nonce = [0_u8; NONCE_BYTES];
    ring::rand::SystemRandom::new().fill(&mut nonce).ok()?;
    let mut sealed = text.to_vec();
    cipher(key)?.seal_in_place_append_tag(ring::aead::Nonce::assume_unique_for_key(nonce), ring::aead::Aad::from(SEALED_MAGIC), &mut sealed).ok()?;
    let mut out = SEALED_MAGIC.to_vec();
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    Some(out)
}

/// The text of a sealed file; none when it is not one, was sealed under
/// another key, or was changed.
#[must_use]
pub fn open(key: &[u8; KEY_BYTES], sealed: &[u8]) -> Option<Vec<u8>> {
    let rest = sealed.strip_prefix(SEALED_MAGIC)?;
    let (nonce, enciphered) = rest.split_at_checked(NONCE_BYTES)?;
    let nonce = ring::aead::Nonce::try_assume_unique_for_key(nonce).ok()?;
    let mut buffer = enciphered.to_vec();
    let text = cipher(key)?.open_in_place(nonce, ring::aead::Aad::from(SEALED_MAGIC), &mut buffer).ok()?;
    Some(text.to_vec())
}

/// Writes the persistent cookies, sealed, to `cookies.bin` in `folder`,
/// whole and readable by the user alone (requirement 21.4).
///
/// # Errors
/// When the file cannot be sealed or written.
pub fn save(folder: &std::path::Path, key: &[u8; KEY_BYTES], store: &Store, now: u64) -> std::io::Result<()> {
    let sealed = seal(key, &store.export(now)).ok_or_else(|| std::io::Error::other("the cookies could not be sealed"))?;
    write_whole(folder, FILE_NAME, TEMPORARY_NAME, sealed)
}

/// Reads `cookies.bin` in `folder`, sealed as it is; none when there is no
/// file.
///
/// # Errors
/// When the file exists but cannot be read, or is larger than any the
/// browser writes.
pub fn load(folder: &std::path::Path) -> std::io::Result<Option<Vec<u8>>> {
    let path = folder.join(FILE_NAME);
    match std::fs::metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
        Ok(metadata) if metadata.len() > u64::try_from(MAX_SEALED).unwrap_or(u64::MAX) => {
            Err(std::io::Error::other(format!("the cookie file is larger than {MAX_SEALED} bytes")))
        }
        Ok(_) => std::fs::read(&path).map(Some),
    }
}

/// Removes `cookies.bin`, when the settings keep no cookie.
///
/// # Errors
/// When it exists and cannot be removed.
pub fn remove(folder: &std::path::Path) -> std::io::Result<()> {
    match std::fs::remove_file(folder.join(FILE_NAME)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}
