//! The WAVE files an episode is read from: RIFF chunks, a `fmt ` chunk
//! of 16-bit PCM, mono or stereo, at a rate the audio profile carries.

use std::path::Path;

use neoteletext_audio::rate_code;

use super::Episode;

/// The largest file read: an hour of 48 kHz stereo is about 700 MB.
const MAX_FILE_BYTES: u64 = 1 << 30;
// The WAVE format: a RIFF header of 12 bytes, chunks of an 8-byte header,
// and a `fmt ` chunk of at least 16 bytes whose tag 1 is PCM.
const RIFF_HEADER: usize = 12;
const CHUNK_HEADER: usize = 8;
const FMT_BYTES: usize = 16;
const FMT_CHANNELS: usize = 2;
const FMT_RATE: usize = 4;
const FMT_BITS: usize = 14;
pub(super) const WAVE_PCM: u16 = 1;
pub(super) const BITS_16: u16 = 16;
const BYTES_PER_SAMPLE: usize = 2;

fn u16_at(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

/// Reads a WAVE file of 16-bit PCM at one of the profile's rates; stereo
/// is mixed to mono.
///
/// # Errors
/// A file that cannot be read, is too large, or is not such a file.
pub fn read(path: &Path) -> Result<Episode, String> {
    let bad = |why: &str| format!("{}: {why}", path.display());
    let size = std::fs::metadata(path).map_err(|error| bad(&error.to_string()))?.len();
    if size > MAX_FILE_BYTES {
        return Err(bad("larger than a gigabyte"));
    }
    let bytes = std::fs::read(path).map_err(|error| bad(&error.to_string()))?;
    if bytes.get(..4) != Some(b"RIFF") || bytes.get(8..RIFF_HEADER) != Some(b"WAVE") {
        return Err(bad("not a WAVE file"));
    }
    let (mut at, mut format, mut data) = (RIFF_HEADER, None, None);
    while let (Some(id), Some(length)) = (bytes.get(at..at.saturating_add(4)), u32_at(&bytes, at.saturating_add(4))) {
        let body = at.saturating_add(CHUNK_HEADER);
        let length = usize::try_from(length).unwrap_or(usize::MAX);
        let end = body.saturating_add(length).min(bytes.len());
        match id {
            b"fmt " if length >= FMT_BYTES => {
                let field = |offset: usize| u16_at(&bytes, body.saturating_add(offset));
                format = Some((field(0), field(FMT_CHANNELS), u32_at(&bytes, body.saturating_add(FMT_RATE)), field(FMT_BITS)));
            }
            b"data" => data = bytes.get(body..end),
            _ => {}
        }
        // Chunks are padded to an even length.
        at = end.saturating_add(length % 2);
    }
    let Some((Some(WAVE_PCM), Some(channels @ (1 | 2)), Some(rate), Some(BITS_16))) = format else {
        return Err(bad("not 16-bit PCM, mono or stereo"));
    };
    rate_code(rate).ok_or_else(|| bad("a rate the audio profile does not carry"))?;
    let data = data.ok_or_else(|| bad("no data"))?;
    let samples = data
        .chunks_exact(usize::from(channels) * BYTES_PER_SAMPLE)
        .map(|frame| {
            let sum: i32 = frame.as_chunks::<BYTES_PER_SAMPLE>().0.iter().map(|pair| i32::from(i16::from_le_bytes(*pair))).sum();
            i16::try_from(sum / i32::from(channels)).unwrap_or(0)
        })
        .collect();
    let title = path.file_stem().map_or_else(|| "Episode".to_owned(), |stem| stem.to_string_lossy().into_owned());
    Ok(Episode { title, rate, samples })
}
