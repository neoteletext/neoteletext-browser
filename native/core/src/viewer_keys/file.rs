//! `viewer-keys.bin` (neoteletext-common's `docs/viewer-state.md` VS-108,
//! VS-118): the remembered keys, and nothing else, sealed under the key the
//! broker holds in the platform's secret store. Each line holds a channel
//! identity, the 32-byte seed, and the times the key was made, remembered
//! and last used, and no other data. Its reader is bounded and strict: the
//! file is outside input, if only from a copied or altered profile.

use super::{Key, MOST_KEYS_PER_JAR, SEED_BYTES, Store};
use crate::dropped::Dropped;
use crate::sealed_file;
use neoteletext_web::viewer_state::{ChannelIdentity, MAX_CHANNEL_IDENTITY_BYTES};

/// The file's name in the settings folder.
pub const FILE_NAME: &str = "viewer-keys.bin";
const TEMPORARY_NAME: &str = "viewer-keys.bin.new";
/// The sealed file's first line: what it is and how it is sealed, which
/// the cipher authenticates with the text.
const SEALED_MAGIC: &[u8] = b"neoteletext-viewer-keys-sealed 1 chacha20-poly1305\n";
/// The text's first line: its format.
const TEXT_VERSION: &str = "neoteletext-viewer-keys 1";
const FIELD_SEPARATOR: char = '\t';
const FIELDS: usize = 5;
/// A seed as text: two hexadecimal digits a byte.
const SEED_TEXT: usize = 2 * SEED_BYTES;
/// The longest time as text: `u64::MAX`'s 20 digits.
const MOST_TIME_DIGITS: usize = 20;
/// The longest line: an identity, a seed, three times, four separators
/// and the line's end.
const MOST_LINE: usize = MAX_CHANNEL_IDENTITY_BYTES + SEED_TEXT + 3 * MOST_TIME_DIGITS + FIELDS;
/// Section 10.3 row 14: the largest text, every key at the longest line
/// with the version's line, within the 4 MiB the document names.
pub const MAX_TEXT: usize = TEXT_VERSION.len() + 1 + MOST_KEYS_PER_JAR * MOST_LINE;
/// The largest sealed file (`MAX_KEY_FILE_BYTES`, 4 MiB).
pub const MAX_KEY_FILE_BYTES: usize = 4 * 1024 * 1024;
const _: () = assert!(sealed_file::max_sealed(SEALED_MAGIC, MAX_TEXT) <= MAX_KEY_FILE_BYTES, "every jar fits the file's bound");

/// A seed as text.
fn hex(seed: &[u8; SEED_BYTES]) -> String {
    use std::fmt::Write as _;
    seed.iter().fold(String::with_capacity(SEED_TEXT), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

/// A seed from its text: exactly 64 lower-case hexadecimal digits.
fn seed_of(text: &str) -> Option<[u8; SEED_BYTES]> {
    if text.len() != SEED_TEXT || !text.bytes().all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)) {
        return None;
    }
    let mut seed = [0_u8; SEED_BYTES];
    for (byte, pair) in seed.iter_mut().zip(text.as_bytes().as_chunks::<2>().0) {
        *byte = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(seed)
}

/// A time in seconds since 1970, written by this file's writer: digits
/// only, at most twenty.
fn time_of(text: &str) -> Option<u64> {
    if text.is_empty() || text.len() > MOST_TIME_DIGITS || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

impl Store {
    /// The remembered keys as the file's text; visit keys and the private
    /// jar are never written (VS-105, VS-110).
    #[must_use]
    pub fn export(&self) -> Vec<u8> {
        use std::fmt::Write as _;
        let mut text = format!("{TEXT_VERSION}\n");
        for (identity, key) in &self.ordinary {
            let Some(remembered) = key.remembered else { continue };
            let _ = writeln!(text, "{identity}\t{}\t{}\t{remembered}\t{}", hex(&key.seed), key.created, key.used);
        }
        text.into_bytes()
    }

    /// The remembered keys of the file's `text`, added to the ordinary jar;
    /// returns why each line not taken was not, without its content. A
    /// file of another format is taken as having none.
    pub fn import(&mut self, text: &[u8], now: u64) -> Dropped {
        let mut dropped = Dropped::default();
        let Ok(text) = std::str::from_utf8(text) else {
            dropped.push(|| String::from("the viewer key file is not text; no remembered key was read"));
            return dropped;
        };
        let mut lines = text.lines();
        if lines.next() != Some(TEXT_VERSION) {
            dropped.push(|| String::from("the viewer key file is of another format; no remembered key was read"));
            return dropped;
        }
        for (number, line) in lines.enumerate() {
            if self.ordinary.len() >= MOST_KEYS_PER_JAR {
                dropped.push(|| format!("the viewer key file holds more than {MOST_KEYS_PER_JAR} keys; the rest were not read"));
                break;
            }
            let fields: Vec<&str> = line.split(FIELD_SEPARATOR).collect();
            let key = match fields.as_slice() {
                &[identity, seed, created, remembered, used] if line.len() <= MOST_LINE => {
                    ChannelIdentity::read(identity).ok().zip(seed_of(seed)).and_then(|(identity, seed)| {
                        let key = Key::of(seed, time_of(created)?, Some(time_of(remembered)?), time_of(used)?)?;
                        Some((identity.as_str().to_owned(), key))
                    })
                }
                _ => None,
            };
            match key {
                Some((identity, key)) if !key.expired(now) && !self.ordinary.contains_key(&identity) => {
                    self.ordinary.insert(identity, key);
                }
                Some(_) => {}
                None => dropped.push(|| format!("line {} of the viewer key file was not read", number.saturating_add(2))),
            }
        }
        dropped
    }
}

/// Writes the remembered keys, sealed, to `viewer-keys.bin` in `folder`,
/// whole and readable by the user alone (VS-108).
///
/// # Errors
/// When the file cannot be sealed or written.
pub fn save(folder: &std::path::Path, key: &[u8; sealed_file::KEY_BYTES], store: &Store) -> std::io::Result<()> {
    let sealed = sealed_file::seal(key, SEALED_MAGIC, &store.export()).ok_or_else(|| std::io::Error::other("the viewer keys could not be sealed"))?;
    sealed_file::save(folder, FILE_NAME, TEMPORARY_NAME, &sealed)
}

/// What reading `viewer-keys.bin` in `folder` came to.
pub enum Loaded {
    /// There is no file.
    None,
    /// The file's text.
    Text(Vec<u8>),
    /// The file does not open under the key the secret store gives: a
    /// copied or restored profile, or a changed file (VS-118).
    NotOpened,
}

/// Reads and opens `viewer-keys.bin` in `folder` under `key`.
///
/// # Errors
/// When the file exists but cannot be read, or is larger than any the
/// browser writes.
pub fn load(folder: &std::path::Path, key: &[u8; sealed_file::KEY_BYTES]) -> std::io::Result<Loaded> {
    Ok(match sealed_file::load(folder, FILE_NAME, MAX_KEY_FILE_BYTES)? {
        None => Loaded::None,
        Some(sealed) => sealed_file::open(key, SEALED_MAGIC, &sealed).map_or(Loaded::NotOpened, Loaded::Text),
    })
}

/// Removes `viewer-keys.bin`, when it holds nothing worth keeping.
///
/// # Errors
/// When it exists and cannot be removed.
pub fn remove(folder: &std::path::Path) -> std::io::Result<()> {
    sealed_file::remove(folder, FILE_NAME)
}

/// The reader of the file's sealed bytes, for fuzzing (VS-108): it opens
/// them under `key` and reads what opens, bounded as the browser reads.
#[must_use]
pub fn read_for_fuzzing(key: &[u8; sealed_file::KEY_BYTES], sealed: &[u8], now: u64) -> usize {
    let mut store = Store::default();
    if let Some(text) = sealed_file::open(key, SEALED_MAGIC, sealed) {
        store.import(&text, now);
    }
    store.import(sealed, now);
    store.ordinary.len()
}

/// Seals `text` as the file does, for fuzzing's seeds.
#[must_use]
pub fn seal_for_fuzzing(key: &[u8; sealed_file::KEY_BYTES], text: &[u8]) -> Option<Vec<u8>> {
    sealed_file::seal(key, SEALED_MAGIC, text)
}
