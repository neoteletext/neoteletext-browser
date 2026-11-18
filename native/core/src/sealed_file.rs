//! A file sealed under a key the broker holds in the platform's secret
//! store (neoteletext-common's `docs/viewer-state.md` VS-108): its first
//! line names what it is and how it is sealed and is authenticated with it,
//! then a nonce drawn afresh for every write, then the text enciphered and
//! authenticated with ChaCha20-Poly1305 (RFC 8439). A copy of the file
//! alone is useless; a byte changed anywhere makes it unreadable.

use crate::private_file::write_whole;

/// A ChaCha20-Poly1305 key (RFC 8439 section 2.8): the broker's
/// sealing key, of its length.
pub const KEY_BYTES: usize = 32;
const _: () = assert!(KEY_BYTES == confine::broker::SEALING_KEY_BYTES, "the broker's sealing key");
const NONCE_BYTES: usize = ring::aead::NONCE_LEN;

fn cipher(key: &[u8; KEY_BYTES]) -> Option<ring::aead::LessSafeKey> {
    ring::aead::UnboundKey::new(&ring::aead::CHACHA20_POLY1305, key).ok().map(ring::aead::LessSafeKey::new)
}

/// The largest sealed file whose text is at most `max_text`: its line,
/// nonce and tag besides.
#[must_use]
pub const fn max_sealed(magic: &[u8], max_text: usize) -> usize {
    magic.len().saturating_add(NONCE_BYTES).saturating_add(max_text).saturating_add(ring::aead::MAX_TAG_LEN)
}

/// Seals `text` under `key`, its first line `magic`. A random 96-bit
/// nonce is safe for the writes one key sees: at most one a second.
#[must_use]
pub fn seal(key: &[u8; KEY_BYTES], magic: &[u8], text: &[u8]) -> Option<Vec<u8>> {
    use ring::rand::SecureRandom as _;
    let mut nonce = [0_u8; NONCE_BYTES];
    ring::rand::SystemRandom::new().fill(&mut nonce).ok()?;
    let mut sealed = text.to_vec();
    cipher(key)?.seal_in_place_append_tag(ring::aead::Nonce::assume_unique_for_key(nonce), ring::aead::Aad::from(magic), &mut sealed).ok()?;
    let mut out = magic.to_vec();
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&sealed);
    Some(out)
}

/// The text of a sealed file; none when it is not one of `magic`, was
/// sealed under another key, or was changed.
#[must_use]
pub fn open(key: &[u8; KEY_BYTES], magic: &[u8], sealed: &[u8]) -> Option<Vec<u8>> {
    let rest = sealed.strip_prefix(magic)?;
    let (nonce, enciphered) = rest.split_at_checked(NONCE_BYTES)?;
    let nonce = ring::aead::Nonce::try_assume_unique_for_key(nonce).ok()?;
    let mut buffer = enciphered.to_vec();
    let text = cipher(key)?.open_in_place(nonce, ring::aead::Aad::from(magic), &mut buffer).ok()?;
    Some(text.to_vec())
}

/// Writes `sealed` whole as `name` in `folder`, readable by the user alone,
/// through `temporary`.
///
/// # Errors
/// When it cannot be written.
pub fn save(folder: &std::path::Path, name: &str, temporary: &str, sealed: &[u8]) -> std::io::Result<()> {
    write_whole(folder, name, temporary, sealed)
}

/// Reads `name` in `folder`, sealed as it is; none when there is no file.
///
/// # Errors
/// When the file exists but cannot be read, or is larger than `max`.
pub fn load(folder: &std::path::Path, name: &str, max: usize) -> std::io::Result<Option<Vec<u8>>> {
    let path = folder.join(name);
    match std::fs::metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
        Ok(metadata) if metadata.len() > u64::try_from(max).unwrap_or(u64::MAX) => Err(std::io::Error::other(format!("{name} is larger than {max} bytes"))),
        Ok(_) => std::fs::read(&path).map(Some),
    }
}

/// Removes `name` in `folder`, if it is there.
///
/// # Errors
/// When it exists and cannot be removed.
pub fn remove(folder: &std::path::Path, name: &str) -> std::io::Result<()> {
    match std::fs::remove_file(folder.join(name)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        other => other,
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::*;

    const MAGIC: &[u8] = b"neoteletext-test-sealed 1 chacha20-poly1305\n";

    /// VS-108: the file opens only under its key, and a byte changed
    /// anywhere, the line included, makes it unreadable.
    #[test]
    fn a_sealed_file_opens_only_whole_and_under_its_key() {
        let key = [7; KEY_BYTES];
        let sealed = seal(&key, MAGIC, b"secret text").unwrap();
        assert_eq!(open(&key, MAGIC, &sealed).unwrap(), b"secret text");
        assert!(!sealed.windows(6).any(|window| window == b"secret"), "the text is enciphered");
        assert!(open(&[8; KEY_BYTES], MAGIC, &sealed).is_none(), "another key");
        for place in 0..sealed.len() {
            let mut changed = sealed.clone();
            changed[place] ^= 1;
            assert!(open(&key, MAGIC, &changed).is_none(), "byte {place} changed");
        }
        assert_ne!(seal(&key, MAGIC, b"secret text").unwrap(), sealed, "a fresh nonce each time");
    }
}
