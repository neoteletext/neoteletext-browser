//! Subresource Integrity (the W3C standard the Fetch standard's main
//! fetch applies): whether a body matches a request's integrity metadata,
//! by SHA-256, SHA-384 or SHA-512 from ring.

use ring::digest::{Algorithm, SHA256, SHA384, SHA512, digest};

/// The base64 alphabet (RFC 4648 section 4).
const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
/// The bytes one group of base64 encodes, and the characters it takes.
const GROUP_BYTES: usize = 3;
/// The bits of one base64 character.
const CHARACTER_BITS: u32 = 6;
/// A base64 character's value: its six bits.
const CHARACTER_MASK: u32 = 0x3F;

/// `bytes` in base64, padded.
fn base64(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len().div_ceil(GROUP_BYTES).saturating_mul(4));
    for group in bytes.chunks(GROUP_BYTES) {
        let mut bits = 0_u32;
        for index in 0..GROUP_BYTES {
            bits = (bits << 8) | u32::from(group.get(index).copied().unwrap_or(0));
        }
        // Four characters a group, the most significant first; a short
        // group's last characters are padding.
        for (position, shift) in [3 * CHARACTER_BITS, 2 * CHARACTER_BITS, CHARACTER_BITS, 0].into_iter().enumerate() {
            let character = if position <= group.len() { ALPHABET.get(((bits >> shift) & CHARACTER_MASK) as usize).copied().unwrap_or(b'=') } else { b'=' };
            text.push(char::from(character));
        }
    }
    text
}

/// A hash the standard knows, strongest last.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Hash {
    Sha256,
    Sha384,
    Sha512,
}

impl Hash {
    fn named(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "sha256" => Some(Self::Sha256),
            "sha384" => Some(Self::Sha384),
            "sha512" => Some(Self::Sha512),
            _ => None,
        }
    }

    fn algorithm(self) -> &'static Algorithm {
        match self {
            Self::Sha256 => &SHA256,
            Self::Sha384 => &SHA384,
            Self::Sha512 => &SHA512,
        }
    }
}

/// The hashes `metadata` names with a known algorithm, each with its
/// base64 value (the standard's "parse metadata"): tokens apart by
/// whitespace, `algorithm-value`, options after `?` ignored.
fn parse(metadata: &str) -> Vec<(Hash, &str)> {
    metadata
        .split_ascii_whitespace()
        .filter_map(|token| {
            let (algorithm, rest) = token.split_once('-')?;
            let value = rest.split_once('?').map_or(rest, |(value, _)| value);
            Some((Hash::named(algorithm)?, value))
        })
        .collect()
}

/// Whether a request carries integrity metadata that names a hash.
pub(crate) fn asks(metadata: &str) -> bool {
    !parse(metadata).is_empty()
}

/// Whether `body` matches `metadata` (the standard's "do bytes match
/// metadataList"): with no known hash named it matches; otherwise it must
/// match one of the hashes of the strongest algorithm named. A value in
/// the URL-safe alphabet is read as the standard one.
pub(crate) fn matches(metadata: &str, body: &[u8]) -> bool {
    let named = parse(metadata);
    let Some(strongest) = named.iter().map(|&(hash, _)| hash).max() else { return true };
    let actual = base64(digest(strongest.algorithm(), body).as_ref());
    named.iter().filter(|&&(hash, _)| hash == strongest).any(|&(_, expected)| expected.replace('-', "+").replace('_', "/") == actual)
}

/// The SHA-256 integrity metadata of `body`, for a test or a fuzz target
/// to ask with.
pub(crate) fn metadata_of(body: &[u8]) -> String {
    format!("sha256-{}", base64(digest(&SHA256, body).as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4648 section 10's test vectors.
    #[test]
    fn base64_is_the_rfcs() {
        for (bytes, text) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(bytes.as_bytes()), text);
        }
        assert_eq!(base64(&[0xFB, 0xFF]), "+/8=");
    }

    /// The hashes of the empty body, as `openssl dgst -binary | base64`
    /// gives them.
    const EMPTY_256: &str = "sha256-47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU=";
    const EMPTY_384: &str = "sha384-OLBgp1GsljhM2TJ+sbHjaiH9txEUvgdDTAzHv2P24donTt6/529l+9Ua0vFImLlb";
    const EMPTY_512: &str = "sha512-z4PhNX7vuL3xVChQ1m2AB9Yg5AULVxXcg/SpIdNs6c5H0NE8XYXysP+DGNKHfuwvY7kxvUdBeoGlODJ6+SfaPg==";

    #[test]
    fn a_body_matches_its_hash_and_no_other() {
        for metadata in [EMPTY_256, EMPTY_384, EMPTY_512] {
            assert!(asks(metadata));
            assert!(matches(metadata, b""), "{metadata}");
            assert!(!matches(metadata, b"x"), "{metadata}");
        }
        assert!(matches("sha256-47DEQpj8HBSa-_TImW-5JCeuQeRkm5NMpJWZG3hSuFU=", b""), "the URL-safe alphabet");
        assert!(matches(&format!("{EMPTY_256}?opt=1"), b""), "options are ignored");
        assert!(matches(&format!("sha256-wrong {EMPTY_256}"), b""), "any hash of the strongest algorithm");
        assert!(!matches(&format!("{EMPTY_256} sha512-wrong"), b""), "only the strongest algorithm counts");
        assert!(matches(&format!("sha256-wrong {EMPTY_512}"), b""));
        assert!(!asks("") && matches("", b"x"), "no metadata");
        assert!(!asks("md5-abc junk") && matches("md5-abc junk", b"x"), "no known hash named");
    }
}
