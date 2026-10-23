//! The release files' compression (`installation-and-updates.md` section
//! 6): each file is gzip (RFC 1952) of its own, with no archive around it.
//! The reader takes only what the release tool writes, a member with no
//! optional fields, and refuses anything else: output beyond the length
//! the manifest gives, a wrong CRC-32 or length, or bytes after the
//! member.

use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::{DecompressorOxide, decompress as inflate, inflate_flags};

/// RFC 1952 section 2.3.1: the member's first two bytes, then the method,
/// DEFLATE.
const ID1: u8 = 0x1f;
const ID2: u8 = 0x8b;
const CM_DEFLATE: u8 = 8;
/// No optional fields (FLG, section 2.3.1), as the release tool writes.
const FLG_NONE: u8 = 0;
/// XFL 2: the slowest, strongest compression (section 2.3.1).
const XFL_STRONGEST: u8 = 2;
/// OS 255: unknown, so the bytes are the same whoever made them.
const OS_UNKNOWN: u8 = 255;
/// The fixed header: ID1, ID2, CM, FLG, MTIME (4), XFL, OS.
const HEADER_BYTES: usize = 10;
/// The trailer: CRC32 and ISIZE, each four bytes, little-endian.
const TRAILER_BYTES: usize = 8;
/// miniz's strongest level, which `gzip -9` also is.
const LEVEL_STRONGEST: u8 = 9;

/// The CRC-32 of RFC 1952 section 8: polynomial 0xEDB88320, reflected.
const CRC_POLYNOMIAL: u32 = 0xEDB8_8320;

/// The byte-wise table of RFC 1952 section 8's sample code.
const CRC_TABLE: [u32; 256] = {
    let mut table = [0_u32; 256];
    let mut n = 0;
    while n < 256 {
        #[allow(clippy::cast_possible_truncation, reason = "n is below 256")]
        let mut c = n as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 == 1 { CRC_POLYNOMIAL ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        #[allow(clippy::indexing_slicing, reason = "n is below the table's 256 entries, at compile time")]
        {
            table[n] = c;
        }
        n += 1;
    }
    table
};

/// The CRC-32 of `bytes`.
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut c = u32::MAX;
    for &byte in bytes {
        let index = usize::from(u8::try_from((c ^ u32::from(byte)) & 0xff).unwrap_or(0));
        c = CRC_TABLE.get(index).copied().unwrap_or(0) ^ (c >> 8);
    }
    c ^ u32::MAX
}

/// ISIZE: the length modulo 2^32 (section 2.3.1).
fn isize_field(length: usize) -> u32 {
    let low = u64::try_from(length).unwrap_or(u64::MAX) & u64::from(u32::MAX);
    u32::try_from(low).unwrap_or(u32::MAX)
}

/// The most bytes [`compress`] makes of `length` bytes: DEFLATE's worst
/// case is stored blocks, each behind 5 bytes of header (RFC 1951 section
/// 3.2.4), with one more block to end, inside the member's header and
/// trailer (RFC 1952 section 2.3). `miniz_oxide` stores at most its 32 KiB
/// window in a block (measured: 300,000 bytes that do not compress made
/// 300,068); the bound counts a block for every 16 KiB, so it holds for
/// any encoder whose blocks are that long or longer. The updater bounds a
/// release file's download by it (finding F-81).
#[must_use]
pub fn max_compressed(length: usize) -> usize {
    /// Half `miniz_oxide`'s window: the shortest stored block counted.
    const STORED_BLOCK: usize = 16 * 1024;
    /// The block's header bits, padded to a byte, then LEN and NLEN.
    const STORED_HEADER: usize = 5;
    let blocks = (length / STORED_BLOCK).saturating_add(2);
    HEADER_BYTES.saturating_add(TRAILER_BYTES).saturating_add(length).saturating_add(blocks.saturating_mul(STORED_HEADER))
}

/// Why compressed bytes were refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Refused(pub &'static str);

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}

/// `bytes` compressed as one gzip member with no optional fields and no
/// time, so a file's compressed form is the same on every build.
#[must_use]
pub fn compress(bytes: &[u8]) -> Vec<u8> {
    let mut out = vec![ID1, ID2, CM_DEFLATE, FLG_NONE, 0, 0, 0, 0, XFL_STRONGEST, OS_UNKNOWN];
    out.extend(miniz_oxide::deflate::compress_to_vec(bytes, LEVEL_STRONGEST));
    out.extend_from_slice(&crc32(bytes).to_le_bytes());
    out.extend_from_slice(&isize_field(bytes.len()).to_le_bytes());
    out
}

/// The bytes of a gzip member, which must be exactly `length` long: the
/// output is never allowed beyond it.
///
/// # Errors
/// On a member the release tool would not write, a damaged stream, a
/// length other than `length`, or a CRC that does not match.
pub fn decompress(member: &[u8], length: usize) -> Result<Vec<u8>, Refused> {
    let out = decompress_at_most(member, length)?;
    if out.len() == length { Ok(out) } else { Err(Refused("not the length the release says")) }
}

/// The bytes of a gzip member of at most `max` bytes, for a file whose
/// own length nothing gives beforehand: the manifest, which gives the
/// others'.
///
/// # Errors
/// As [`decompress`], and on output beyond `max`.
pub fn decompress_at_most(member: &[u8], max: usize) -> Result<Vec<u8>, Refused> {
    let length = max;
    let Some((header, rest)) = member.split_first_chunk::<HEADER_BYTES>() else { return Err(Refused("shorter than a gzip header")) };
    let [id1, id2, method, flags, _, _, _, _, _, _] = *header;
    if (id1, id2, method) != (ID1, ID2, CM_DEFLATE) {
        return Err(Refused("not gzip's DEFLATE"));
    }
    if flags != FLG_NONE {
        return Err(Refused("gzip optional fields, which a release file never has"));
    }
    // One byte more than allowed, so a stream that would write past the
    // length is seen to, rather than stopping at the end of the buffer.
    let room = length.checked_add(1).ok_or(Refused("too long"))?;
    let mut out = vec![0_u8; room];
    let mut state = Box::<DecompressorOxide>::default();
    let (status, consumed, written) = inflate(&mut state, rest, &mut out, 0, inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF);
    match status {
        TINFLStatus::Done => {}
        TINFLStatus::HasMoreOutput => return Err(Refused("longer than the release says")),
        _ => return Err(Refused("a damaged or truncated stream")),
    }
    if written > length {
        return Err(Refused("longer than the release says"));
    }
    out.truncate(written);
    let trailer = rest.get(consumed..).ok_or(Refused("a damaged stream"))?;
    let (&[c0, c1, c2, c3, s0, s1, s2, s3], extra) = trailer.split_first_chunk::<TRAILER_BYTES>().ok_or(Refused("no gzip trailer"))?;
    if !extra.is_empty() {
        return Err(Refused("bytes after the gzip member"));
    }
    if u32::from_le_bytes([c0, c1, c2, c3]) != crc32(&out) {
        return Err(Refused("the CRC does not match"));
    }
    if u32::from_le_bytes([s0, s1, s2, s3]) != isize_field(written) {
        return Err(Refused("the length in the trailer does not match"));
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {

    /// Bytes that do not compress at all are within the bound.
    #[test]
    fn what_compress_makes_is_within_its_bound() {
        let mut state = 0x2545_f491_u32;
        let noise: Vec<u8> = (0..300_000)
            .map(|_| {
                // xorshift32 (Marsaglia, 2003): bytes no compressor shrinks.
                state ^= state << 13;
                state ^= state >> 17;
                state ^= state << 5;
                state.to_le_bytes()[0]
            })
            .collect();
        for length in [0, 1, 16_383, 32_768, 65_535, 65_536, 200_001, noise.len()] {
            assert!(compress(&noise[..length]).len() <= max_compressed(length), "{length}: {} > {}", compress(&noise[..length]).len(), max_compressed(length));
        }
    }

    use super::*;

    #[test]
    fn crc32_is_rfc_1952s() {
        // The CRC-32 check value of "123456789" (ITU-T V.42, the same CRC).
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn a_file_round_trips_and_nothing_else_is_taken() {
        let data: Vec<u8> = (0..100_000_u32).map(|n| u8::try_from(n % 251).unwrap()).collect();
        let member = compress(&data);
        assert!(member.len() < data.len() / 10, "compressed: {}", member.len());
        assert_eq!(decompress(&member, data.len()).unwrap(), data);
        assert_eq!(compress(&data), member, "the same bytes every time");
        assert_eq!(decompress(&member, data.len() - 2), Err(Refused("longer than the release says")));
        assert!(decompress(&member, data.len() - 1).is_err(), "one byte too long");
        assert_eq!(decompress_at_most(&member, data.len() * 2).unwrap(), data);
        assert!(decompress_at_most(&member, data.len() - 1).is_err());
        assert_eq!(decompress(&member, data.len() + 1), Err(Refused("not the length the release says")));
        let mut flipped = member.clone();
        let last = flipped.len() - 5;
        flipped[last - 4] ^= 1;
        assert!(decompress(&flipped, data.len()).is_err(), "the CRC");
        assert!(decompress(&member[..member.len() - 1], data.len()).is_err(), "truncated");
        assert!(decompress(&member[..member.len() / 2], data.len()).is_err(), "cut in half");
        let mut extra = member.clone();
        extra.push(0);
        assert_eq!(decompress(&extra, data.len()), Err(Refused("bytes after the gzip member")));
        let mut named = member.clone();
        named[3] = 8;
        assert!(decompress(&named, data.len()).is_err(), "FNAME");
        assert!(decompress(&[], 0).is_err());
        assert_eq!(decompress(&compress(b""), 0).unwrap(), Vec::<u8>::new());
    }
}
