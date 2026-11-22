//! A context's two random sources, both from the 32-byte seed the trusted
//! core draws for it from the operating system (script-api.md section 13
//! item 7; the security review's findings S-13, TODO-7279, and S-18 item
//! 2, TODO-7285).
//!
//! `Math.random` is QuickJS's own small generator, whose state a script
//! can recover from a few outputs; [`math_seed`] is the state it starts
//! from. `crypto.getRandomValues` and `crypto.randomUUID` read
//! [`Generator`], a cryptographic generator. The two are derived from the
//! seed with HKDF-SHA-256 (RFC 5869) under distinct labels, so knowing
//! everything about one says nothing of the other, and no two contexts
//! share a seed.

use ring::aead::{Aad, CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey};
use ring::hkdf::{HKDF_SHA256, KeyType, Salt};

/// A context's seed, in bytes.
pub const SEED_BYTES: usize = 32;
/// The salt of the extraction: the purpose of every key derived here.
const SALT: &[u8] = b"neoteletext script random v1";
/// The label `Math.random`'s starting state is derived under.
const MATH_LABEL: &[u8] = b"Math.random state";
/// The label the cryptographic generator's key is derived under.
const CRYPTO_LABEL: &[u8] = b"crypto.getRandomValues key";
/// `ChaCha20`'s key, in bytes (RFC 8439 section 2.3).
const KEY_BYTES: usize = 32;
/// `ChaCha20-Poly1305`'s nonce, in bytes (RFC 8439 section 2.8).
const NONCE_BYTES: usize = 12;
/// `Math.random`'s state is given to QuickJS as a time in microseconds, a
/// positive signed 64-bit number; 62 bits of the derived value are used.
const MATH_SEED_BITS: u32 = 62;

/// A length HKDF is asked to fill.
struct Length(usize);

impl KeyType for Length {
    fn len(&self) -> usize {
        self.0
    }
}

/// Fills `out` with the bytes HKDF-SHA-256 derives from `seed` under `label`.
fn derive(seed: &[u8; SEED_BYTES], label: &[u8], out: &mut [u8]) -> bool {
    let labels = [label];
    let extracted = Salt::new(HKDF_SHA256, SALT).extract(seed);
    let Ok(expanded) = extracted.expand(&labels, Length(out.len())) else { return false };
    expanded.fill(out).is_ok()
}

/// The state `Math.random` starts from: a number from 1 to 2 to the 62,
/// never 0, which QuickJS's generator cannot leave.
#[must_use]
pub fn math_seed(seed: &[u8; SEED_BYTES]) -> i64 {
    let mut bytes = [0_u8; 8];
    if !derive(seed, MATH_LABEL, &mut bytes) {
        return 1;
    }
    let value = u64::from_le_bytes(bytes) >> (u64::BITS.saturating_sub(MATH_SEED_BITS));
    i64::try_from(value).unwrap_or(1).max(1)
}

/// The cryptographic generator: `ChaCha20`'s key stream under a key derived
/// from the seed, a fresh nonce for every request. The key stream is taken
/// as `ring`'s `ChaCha20-Poly1305` encryption of zero bytes, which is that
/// stream from block 1 (RFC 8439 section 2.8), so no cipher is written
/// here.
pub struct Generator {
    key: Option<LessSafeKey>,
    /// The number of requests answered: the next nonce.
    requests: u64,
}

impl Generator {
    /// The generator of the context whose seed is `seed`.
    #[must_use]
    pub fn new(seed: &[u8; SEED_BYTES]) -> Self {
        let mut key = [0_u8; KEY_BYTES];
        let made = derive(seed, CRYPTO_LABEL, &mut key).then(|| UnboundKey::new(&CHACHA20_POLY1305, &key).ok().map(LessSafeKey::new)).flatten();
        Self { key: made, requests: 0 }
    }

    /// Fills `out` with random bytes; false when the generator cannot
    /// (its key could not be made, or it has answered 2 to the 64
    /// requests), in which case `out` is left as zeros and the caller
    /// must not give it to a script.
    pub fn fill(&mut self, out: &mut [u8]) -> bool {
        out.fill(0);
        let Some(key) = &self.key else { return false };
        let mut nonce = [0_u8; NONCE_BYTES];
        for (slot, byte) in nonce.iter_mut().zip(self.requests.to_le_bytes()) {
            *slot = byte;
        }
        let Some(next) = self.requests.checked_add(1) else { return false };
        self.requests = next;
        key.seal_in_place_separate_tag(Nonce::assume_unique_for_key(nonce), Aad::empty(), out).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// REQ (script-api.md section 13 item 7; TODO-7285 item 2): the two
    /// sources of one seed differ, and two seeds give different sources.
    #[test]
    fn the_two_sources_are_independent_and_follow_the_seed() {
        let one = [7_u8; SEED_BYTES];
        let mut other = one;
        other[31] = 8;
        assert_ne!(math_seed(&one), math_seed(&other));
        assert!(math_seed(&one) >= 1);
        let mut first = [0_u8; 64];
        let mut second = [0_u8; 64];
        assert!(Generator::new(&one).fill(&mut first));
        assert!(Generator::new(&other).fill(&mut second));
        assert_ne!(first, second);
        // The generator's first bytes are not Math.random's state.
        assert_ne!(first[..8], math_seed(&one).to_le_bytes());
    }

    /// The same seed gives the same bytes (the runner's repeatable runs),
    /// and each request gives new ones.
    #[test]
    fn a_seed_repeats_and_requests_differ() {
        let seed = [1_u8; SEED_BYTES];
        let mut generator = Generator::new(&seed);
        let (mut a, mut b, mut again) = ([0_u8; 32], [0_u8; 32], [0_u8; 32]);
        assert!(generator.fill(&mut a));
        assert!(generator.fill(&mut b));
        assert_ne!(a, b);
        assert_ne!(a, [0_u8; 32]);
        assert!(Generator::new(&seed).fill(&mut again));
        assert_eq!(a, again);
    }

    /// The key stream is `ChaCha20`'s: RFC 8439 section 2.8.2's key and
    /// nonce give its cipher text of that section's plain text, so the
    /// stream of zeros is the cipher text exclusive-or the plain text.
    #[test]
    fn the_stream_is_chacha20_from_block_one() {
        // RFC 8439 section 2.4.2: key 00..1f, nonce 00 00 00 00 00 00 00 4a 00 00 00 00,
        // counter 1; the first key stream block's first bytes.
        let key: [u8; 32] = core::array::from_fn(|index| u8::try_from(index).unwrap());
        let nonce = [0, 0, 0, 0, 0, 0, 0, 0x4a, 0, 0, 0, 0];
        let sealing = LessSafeKey::new(UnboundKey::new(&CHACHA20_POLY1305, &key).unwrap());
        let mut stream = [0_u8; 16];
        let _tag = sealing.seal_in_place_separate_tag(Nonce::assume_unique_for_key(nonce), Aad::empty(), &mut stream).unwrap();
        assert_eq!(stream, [0x22, 0x4f, 0x51, 0xf3, 0x40, 0x1b, 0xd9, 0xe1, 0x2f, 0xde, 0x27, 0x6f, 0xb8, 0x63, 0x1d, 0xed]);
    }
}
