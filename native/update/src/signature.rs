//! The Linux release key's signatures (`installation-and-updates.md`
//! sections 4.3 and 5): Linux has no platform signature, so each release
//! file, the manifest among them, carries a detached Ed25519 signature
//! (RFC 8032) beside it, 64 raw bytes, which the launcher checks against
//! the release public key built into it. The same bytes verify with
//! OpenSSL (`openssl pkeyutl -verify -rawin`), for the first download.

use ring::signature::{ED25519, UnparsedPublicKey};

/// An Ed25519 public key's bytes (RFC 8032 section 5.1.5).
pub const PUBLIC_KEY_BYTES: usize = 32;
/// An Ed25519 signature's bytes (RFC 8032 section 5.1.6).
pub const SIGNATURE_BYTES: usize = 64;

/// Whether `signature` is the release key `public_key`'s over `message`.
#[must_use]
pub fn verify(public_key: &[u8; PUBLIC_KEY_BYTES], message: &[u8], signature: &[u8]) -> bool {
    signature.len() == SIGNATURE_BYTES && UnparsedPublicKey::new(&ED25519, public_key).verify(message, signature).is_ok()
}

/// RFC 8410 section 4: the DER prefix of an Ed25519 `SubjectPublicKeyInfo`
/// (the algorithm 1.3.101.112 and a 33-byte bit string), before the key's
/// 32 bytes.
const SPKI_PREFIX: [u8; 12] = [0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00];

/// The public key as PEM, as the download page publishes it for OpenSSL.
#[must_use]
pub fn public_key_pem(public_key: &[u8; PUBLIC_KEY_BYTES]) -> String {
    let mut der = SPKI_PREFIX.to_vec();
    der.extend_from_slice(public_key);
    format!("-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n", base64(&der))
}

/// Base64 (RFC 4648 section 4) with padding, for PEM.
#[must_use]
pub fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    const SIX_BITS: u32 = 0x3f;
    let symbol = |value: u32| char::from(ALPHABET.get(usize::try_from(value & SIX_BITS).unwrap_or(0)).copied().unwrap_or(b'A'));
    let mut text = String::new();
    for chunk in bytes.chunks(3) {
        let [a, b, c] = [0, 1, 2].map(|at| u32::from(chunk.get(at).copied().unwrap_or(0)));
        let triple = (a << 16) | (b << 8) | c;
        text.push(symbol(triple >> 18));
        text.push(symbol(triple >> 12));
        text.push(if chunk.len() > 1 { symbol(triple >> 6) } else { '=' });
        text.push(if chunk.len() > 2 { symbol(triple) } else { '=' });
    }
    text
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair as _};

    #[test]
    fn a_signature_verifies_only_for_its_key_and_message() {
        let random = ring::rand::SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&random).unwrap();
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap();
        let public: [u8; PUBLIC_KEY_BYTES] = pair.public_key().as_ref().try_into().unwrap();
        let signature = pair.sign(b"neoteletext");
        assert!(verify(&public, b"neoteletext", signature.as_ref()));
        assert!(!verify(&public, b"neoteletexT", signature.as_ref()), "another message");
        let other = Ed25519KeyPair::from_pkcs8(Ed25519KeyPair::generate_pkcs8(&random).unwrap().as_ref()).unwrap();
        let other_public: [u8; PUBLIC_KEY_BYTES] = other.public_key().as_ref().try_into().unwrap();
        assert!(!verify(&other_public, b"neoteletext", signature.as_ref()), "another key");
        assert!(!verify(&public, b"neoteletext", &signature.as_ref()[..SIGNATURE_BYTES - 1]), "short");
    }

    #[test]
    fn base64_is_rfc_4648s() {
        // RFC 4648 section 10's test vectors.
        for (plain, encoded) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foob", "Zm9vYg=="), ("fooba", "Zm9vYmE="), ("foobar", "Zm9vYmFy")]
        {
            assert_eq!(base64(plain.as_bytes()), encoded);
        }
        assert!(public_key_pem(&[0; PUBLIC_KEY_BYTES]).starts_with("-----BEGIN PUBLIC KEY-----\nMCowBQYDK2VwAyEA"));
    }
}
