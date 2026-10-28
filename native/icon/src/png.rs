//! The PNG files the icon is read from and written as (the PNG
//! specification, second edition, W3C Recommendation of 10 November 2003;
//! ISO/IEC 15948:2004). Only what the icon needs: 8-bit RGBA, not
//! interlaced. Reading is bounded and strict: at most [`MAX_SIDE`] square,
//! every chunk's CRC checked, the image data inflated to exactly its size
//! and no more, the five filters of section 9.2 undone, ancillary chunks
//! passed over and any other critical chunk refused. Writing gives each row
//! no filter, and the sRGB chunk the owner's art carries, so every copy is
//! shown in the art's own colours.

use miniz_oxide::deflate::compress_to_vec_zlib;
use miniz_oxide::inflate::decompress_to_vec_zlib_with_limit;

use crate::{BYTES_PER_PIXEL, Image, MAX_SIDE, Refused};

/// Section 5.2: every PNG's first eight bytes.
const SIGNATURE: [u8; 8] = [137, 80, 78, 71, 13, 10, 26, 10];
/// Section 5.3: a chunk's length is at most 2^31 - 1.
const MAX_CHUNK_LENGTH: u32 = 0x7fff_ffff;
/// Section 11.2.2: the image header, first.
const IHDR: [u8; 4] = *b"IHDR";
/// Section 11.2.3: a palette, which for a truecolour image only suggests
/// colours to a display with fewer, and is passed over here.
const PLTE: [u8; 4] = *b"PLTE";
/// Section 11.2.4: the image data, in one or more chunks one after another.
const IDAT: [u8; 4] = *b"IDAT";
/// Section 11.2.5: the end.
const IEND: [u8; 4] = *b"IEND";
/// Section 11.3.3.5: the image is in the sRGB colour space.
const SRGB: [u8; 4] = *b"sRGB";
/// Section 11.3.3.5: the perceptual rendering intent, the owner's art's.
const PERCEPTUAL: u8 = 0;
/// Section 5.4: bit 5 of a chunk type's first byte, set, makes the chunk
/// ancillary, which a reader that does not know it may pass over.
const ANCILLARY: u8 = 0x20;
/// Section 11.2.2: the image header's data: width, height, bit depth,
/// colour type, compression, filter and interlace methods.
const IHDR_BYTES: usize = 13;
/// Section 11.2.2: 8 bits a sample.
const BIT_DEPTH: u8 = 8;
/// Section 11.2.2: truecolour with alpha.
const COLOUR_RGBA: u8 = 6;
/// Section 10: compression method 0, zlib's DEFLATE.
const COMPRESSION_DEFLATE: u8 = 0;
/// Section 9.1: filter method 0, adaptive filtering with five types.
const FILTER_ADAPTIVE: u8 = 0;
/// Section 8.1: interlace method 0, none.
const INTERLACE_NONE: u8 = 0;
/// Section 9.2's filter types.
const FILTER_NONE: u8 = 0;
const FILTER_SUB: u8 = 1;
const FILTER_UP: u8 = 2;
const FILTER_AVERAGE: u8 = 3;
const FILTER_PAETH: u8 = 4;
/// The largest file read: a [`MAX_SIDE`] square stored with no compression
/// at all is 4.2 MB, which this bounds with room for zlib's stored blocks
/// and any ancillary chunks.
const MAX_FILE_BYTES: usize = 16 * 1024 * 1024;
/// miniz's strongest level, as the release files' gzip uses.
const LEVEL_STRONGEST: u8 = 9;

/// Annex D's CRC: the polynomial of ISO 3309 and ITU-T V.42, reflected,
/// the one gzip uses (`update::gzip`).
const CRC_POLYNOMIAL: u32 = 0xEDB8_8320;

/// Annex D's table of the CRC of every byte.
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

/// Annex D: the CRC of `parts`, one after another.
fn crc(parts: &[&[u8]]) -> u32 {
    let mut c = u32::MAX;
    for &byte in parts.iter().flat_map(|part| part.iter()) {
        let index = usize::from(u8::try_from((c ^ u32::from(byte)) & 0xff).unwrap_or(0));
        c = CRC_TABLE.get(index).copied().unwrap_or(0) ^ (c >> 8);
    }
    c ^ u32::MAX
}

/// Annex D's CRC of `bytes`.
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    crc(&[bytes])
}

/// The bytes of one row of the image data: its filter type, then its
/// pixels (section 7.2).
fn row_bytes(width: u32) -> Option<usize> {
    usize::try_from(width).ok()?.checked_mul(BYTES_PER_PIXEL)?.checked_add(1)
}

/// A chunk's type and data, and the bytes after it.
type Chunk<'a> = ([u8; 4], &'a [u8], &'a [u8]);

/// The next chunk of `bytes`: its type, its data and what follows it, its
/// CRC checked (section 5.3).
fn chunk(bytes: &[u8]) -> Result<Chunk<'_>, Refused> {
    let (length, rest) = bytes.split_first_chunk::<4>().ok_or(Refused("a chunk cut short"))?;
    let length = u32::from_be_bytes(*length);
    if length > MAX_CHUNK_LENGTH {
        return Err(Refused("a chunk longer than the specification allows"));
    }
    let (kind, rest) = rest.split_first_chunk::<4>().ok_or(Refused("a chunk cut short"))?;
    let (data, rest) = rest.split_at_checked(usize::try_from(length).map_err(|_| Refused("a chunk too long"))?).ok_or(Refused("a chunk cut short"))?;
    let (check, rest) = rest.split_first_chunk::<4>().ok_or(Refused("a chunk cut short"))?;
    if crc(&[kind, data]) != u32::from_be_bytes(*check) {
        return Err(Refused("a chunk whose CRC is wrong"));
    }
    Ok((*kind, data, rest))
}

/// Section 11.2.2: the width and height of an image header that is 8-bit
/// RGBA, compressed and filtered as the specification's methods 0, and not
/// interlaced.
fn header(data: &[u8]) -> Result<(u32, u32), Refused> {
    let fields: &[u8; IHDR_BYTES] = data.try_into().map_err(|_| Refused("an image header of the wrong length"))?;
    let [w0, w1, w2, w3, h0, h1, h2, h3, depth, colour, compression, filter, interlace] = *fields;
    let (width, height) = (u32::from_be_bytes([w0, w1, w2, w3]), u32::from_be_bytes([h0, h1, h2, h3]));
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(Refused("an image of no size, or larger than any icon"));
    }
    if depth != BIT_DEPTH || colour != COLOUR_RGBA {
        return Err(Refused("an image that is not 8-bit RGBA"));
    }
    if compression != COMPRESSION_DEFLATE || filter != FILTER_ADAPTIVE || interlace != INTERLACE_NONE {
        return Err(Refused("an image compressed, filtered or interlaced otherwise"));
    }
    Ok((width, height))
}

/// Section 9.3's average of the bytes to the left and above.
fn average(left: u8, up: u8) -> u8 {
    u8::try_from(u16::from(left).wrapping_add(u16::from(up)) >> 1).unwrap_or(u8::MAX)
}

/// Section 9.4: of the bytes to the left, above and above left, the one
/// nearest `left + up - up_left`, ties going left, then up.
fn paeth(left: u8, up: u8, up_left: u8) -> u8 {
    let (a, b, c) = (i16::from(left), i16::from(up), i16::from(up_left));
    let estimate = a.wrapping_add(b).wrapping_sub(c);
    let (to_a, to_b, to_c) = (estimate.wrapping_sub(a).wrapping_abs(), estimate.wrapping_sub(b).wrapping_abs(), estimate.wrapping_sub(c).wrapping_abs());
    if to_a <= to_b && to_a <= to_c {
        left
    } else if to_b <= to_c {
        up
    } else {
        up_left
    }
}

/// Undoes `filter` on `line` in place, `previous` being the row above as
/// already undone, or zeros for the first (section 9.2).
fn unfilter(filter: u8, line: &mut [u8], previous: &[u8]) -> Result<(), Refused> {
    for at in 0..line.len() {
        let before = at.checked_sub(BYTES_PER_PIXEL);
        let left = before.and_then(|index| line.get(index)).copied().unwrap_or(0);
        let up = previous.get(at).copied().unwrap_or(0);
        let up_left = before.and_then(|index| previous.get(index)).copied().unwrap_or(0);
        let predictor = match filter {
            FILTER_NONE => 0,
            FILTER_SUB => left,
            FILTER_UP => up,
            FILTER_AVERAGE => average(left, up),
            FILTER_PAETH => paeth(left, up, up_left),
            _ => return Err(Refused("a row with a filter type the specification does not have")),
        };
        if let Some(byte) = line.get_mut(at) {
            *byte = byte.wrapping_add(predictor);
        }
    }
    Ok(())
}

/// The picture in `bytes`, an 8-bit RGBA PNG, not interlaced.
///
/// # Errors
/// What in it is not such a PNG, or larger than [`MAX_SIDE`].
pub fn decode(bytes: &[u8]) -> Result<Image, Refused> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(Refused("a file larger than any icon"));
    }
    let mut rest = bytes.strip_prefix(&SIGNATURE).ok_or(Refused("not a PNG"))?;
    let mut size = None;
    let mut data = Vec::new();
    loop {
        let (kind, chunk_data, after) = chunk(rest)?;
        rest = after;
        let [first, ..] = kind;
        match (kind, size) {
            (IHDR, None) => size = Some(header(chunk_data)?),
            (_, None) => return Err(Refused("the image header is not the first chunk")),
            (IHDR, Some(_)) => return Err(Refused("a second image header")),
            (IDAT, Some(_)) => {
                if data.len().saturating_add(chunk_data.len()) > MAX_FILE_BYTES {
                    return Err(Refused("more image data than any icon has"));
                }
                data.extend_from_slice(chunk_data);
            }
            (IEND, Some(_)) => break,
            (PLTE, Some(_)) => {}
            (_, Some(_)) if first & ANCILLARY != 0 => {}
            (_, Some(_)) => return Err(Refused("a critical chunk this reader does not know")),
        }
    }
    if !rest.is_empty() {
        return Err(Refused("bytes after the end"));
    }
    let (width, height) = size.ok_or(Refused("no image header"))?;
    let row = row_bytes(width).ok_or(Refused("a row too long"))?;
    let expected = row.checked_mul(usize::try_from(height).map_err(|_| Refused("too many rows"))?).ok_or(Refused("too many rows"))?;
    let raw = decompress_to_vec_zlib_with_limit(&data, expected).map_err(|_| Refused("image data that does not inflate to the image's size"))?;
    if raw.len() != expected {
        return Err(Refused("image data shorter than the image"));
    }
    let pixels = row.saturating_sub(1);
    let mut rgba = Vec::with_capacity(expected);
    let mut previous = vec![0; pixels];
    for filtered in raw.chunks_exact(row) {
        let (&filter, line) = filtered.split_first().ok_or(Refused("an empty row"))?;
        let mut line = line.to_vec();
        unfilter(filter, &mut line, &previous)?;
        rgba.extend_from_slice(&line);
        previous = line;
    }
    Image::new(width, height, rgba)
}

/// Appends the chunk `kind` holding `data` to `out` (section 5.3).
fn push_chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) -> Result<(), Refused> {
    let length = u32::try_from(data.len()).ok().filter(|length| *length <= MAX_CHUNK_LENGTH).ok_or(Refused("a chunk too long"))?;
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    out.extend_from_slice(&crc(&[&kind, data]).to_be_bytes());
    Ok(())
}

/// The image header of `image`, as [`header`] reads it.
fn header_data(image: &Image) -> Vec<u8> {
    let mut data = Vec::with_capacity(IHDR_BYTES);
    data.extend_from_slice(&image.width().to_be_bytes());
    data.extend_from_slice(&image.height().to_be_bytes());
    data.extend_from_slice(&[BIT_DEPTH, COLOUR_RGBA, COMPRESSION_DEFLATE, FILTER_ADAPTIVE, INTERLACE_NONE]);
    data
}

/// `image` as a PNG: its header, sRGB, its rows unfiltered and compressed
/// as one image data chunk, and the end. The same image always makes the
/// same bytes.
///
/// # Errors
/// Never for an [`Image`], whose size is bounded; the lengths are checked
/// all the same.
pub fn encode(image: &Image) -> Result<Vec<u8>, Refused> {
    let row = row_bytes(image.width()).ok_or(Refused("a row too long"))?;
    let mut raw = Vec::with_capacity(row.saturating_mul(usize::try_from(image.height()).unwrap_or(0)));
    for pixels in image.rgba().chunks_exact(row.saturating_sub(1).max(1)) {
        raw.push(FILTER_NONE);
        raw.extend_from_slice(pixels);
    }
    let mut out = SIGNATURE.to_vec();
    push_chunk(&mut out, IHDR, &header_data(image))?;
    push_chunk(&mut out, SRGB, &[PERCEPTUAL])?;
    push_chunk(&mut out, IDAT, &compress_to_vec_zlib(&raw, LEVEL_STRONGEST))?;
    push_chunk(&mut out, IEND, &[])?;
    Ok(out)
}

#[cfg(test)]
mod tests;
