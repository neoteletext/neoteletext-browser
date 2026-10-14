//! An indexed Portable Network Graphics writer, standing in for
//! `neoteletext-image`. It compresses for real, so that frame sizes measured
//! with the stand-in are representative: each row uses the "up" filter
//! (PNG specification section 9.2, filter type 2), and the image data is
//! deflated with the fixed Huffman codes of RFC 1951 section 3.2.6, using
//! only matches at distance 1, which turns the long runs of a filtered
//! teletext page into a few codes.

use crate::page::Frame;

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1A, b'\n'];
const BIT_DEPTH: u8 = 8;
const COLOUR_TYPE_INDEXED: u8 = 3;
const FILTER_NONE: u8 = 0;
const FILTER_UP: u8 = 2;
// zlib header: deflate with a 32 kilobyte window, no dictionary, fastest
// level; 0x7801 is divisible by 31 as RFC 1950 section 2.2 requires.
const ZLIB_HEADER: [u8; 2] = [0x78, 0x01];

/// Encodes a frame as an indexed image with a palette of exactly the
/// frame's colours.
#[must_use]
pub fn encode(frame: &Frame) -> Vec<u8> {
    let mut out = SIGNATURE.to_vec();

    let mut header = Vec::new();
    header.extend_from_slice(&u32::try_from(frame.width).unwrap_or(0).to_be_bytes());
    header.extend_from_slice(&u32::try_from(frame.height).unwrap_or(0).to_be_bytes());
    header.extend_from_slice(&[BIT_DEPTH, COLOUR_TYPE_INDEXED, 0, 0, 0]);
    chunk(&mut out, *b"IHDR", &header);

    let palette: Vec<u8> = frame.palette.iter().flat_map(|colour| [colour.red, colour.green, colour.blue]).collect();
    chunk(&mut out, *b"PLTE", &palette);

    let filtered = filter_up(&frame.indices, frame.width);
    let mut stream = ZLIB_HEADER.to_vec();
    stream.extend_from_slice(&deflate_fixed(&filtered));
    stream.extend_from_slice(&adler32(&filtered).to_be_bytes());
    chunk(&mut out, *b"IDAT", &stream);

    chunk(&mut out, *b"IEND", &[]);
    out
}

fn chunk(out: &mut Vec<u8>, kind: [u8; 4], data: &[u8]) {
    out.extend_from_slice(&u32::try_from(data.len()).unwrap_or(0).to_be_bytes());
    let start = out.len();
    out.extend_from_slice(&kind);
    out.extend_from_slice(data);
    let crc = crc32(&out[start..]);
    out.extend_from_slice(&crc.to_be_bytes());
}

fn filter_up(indices: &[u8], width: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(indices.len() + indices.len() / width.max(1));
    for (row, line) in indices.chunks(width).enumerate() {
        if row == 0 {
            out.push(FILTER_NONE);
            out.extend_from_slice(line);
        } else {
            out.push(FILTER_UP);
            let above = &indices[(row - 1) * width..row * width];
            out.extend(line.iter().zip(above).map(|(value, up)| value.wrapping_sub(*up)));
        }
    }
    out
}

/// CRC-32 as PNG uses it (PNG specification annex D).
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    const POLYNOMIAL: u32 = 0xEDB8_8320;
    let mut crc = 0xFFFF_FFFF_u32;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 { (crc >> 1) ^ POLYNOMIAL } else { crc >> 1 };
        }
    }
    !crc
}

/// Adler-32 (RFC 1950 section 8).
#[must_use]
pub fn adler32(bytes: &[u8]) -> u32 {
    const MODULUS: u32 = 65_521;
    let (mut a, mut b) = (1_u32, 0_u32);
    for &byte in bytes {
        a = (a + u32::from(byte)) % MODULUS;
        b = (b + a) % MODULUS;
    }
    (b << 16) | a
}

struct BitWriter {
    bytes: Vec<u8>,
    current: u32,
    filled: u32,
}

impl BitWriter {
    // Writes the low `count` bits of `value`, least significant first.
    fn bits(&mut self, value: u32, count: u32) {
        self.current |= value << self.filled;
        self.filled += count;
        while self.filled >= 8 {
            self.bytes.push((self.current & 0xFF) as u8);
            self.current >>= 8;
            self.filled -= 8;
        }
    }

    // Huffman codes are packed most significant bit first (RFC 1951
    // section 3.1.1), so the code is reversed before writing.
    fn code(&mut self, code: u32, length: u32) {
        let reversed = code.reverse_bits() >> (32 - length);
        self.bits(reversed, length);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.filled > 0 {
            self.bytes.push((self.current & 0xFF) as u8);
        }
        self.bytes
    }
}

// Fixed literal and length codes, RFC 1951 section 3.2.6.
fn write_literal_or_length(writer: &mut BitWriter, symbol: u32) {
    match symbol {
        0..=143 => writer.code(0x30 + symbol, 8),
        144..=255 => writer.code(0x190 + symbol - 144, 9),
        256..=279 => writer.code(symbol - 256, 7),
        _ => writer.code(0xC0 + symbol - 280, 8),
    }
}

// Length codes 257 to 285 with their base lengths and extra bits,
// RFC 1951 section 3.2.5.
const LENGTH_BASES: [(u32, u32); 29] = [
    (3, 0), (4, 0), (5, 0), (6, 0), (7, 0), (8, 0), (9, 0), (10, 0),
    (11, 1), (13, 1), (15, 1), (17, 1),
    (19, 2), (23, 2), (27, 2), (31, 2),
    (35, 3), (43, 3), (51, 3), (59, 3),
    (67, 4), (83, 4), (99, 4), (115, 4),
    (131, 5), (163, 5), (195, 5), (227, 5),
    (258, 0),
];
const FIRST_LENGTH_CODE: u32 = 257;
const END_OF_BLOCK: u32 = 256;
const MIN_MATCH: usize = 3;
const MAX_MATCH: usize = 258;
// Distance code 0 means distance 1, written with the fixed 5-bit code.
const DISTANCE_ONE_CODE: u32 = 0;
const DISTANCE_CODE_BITS: u32 = 5;
// Block header: BFINAL = 1, BTYPE = 01 (fixed Huffman codes).
const FINAL_FIXED_BLOCK: u32 = 0b011;
const BLOCK_HEADER_BITS: u32 = 3;

fn write_length(writer: &mut BitWriter, length: usize) {
    let length = u32::try_from(length).unwrap_or(0);
    let index = LENGTH_BASES.iter().rposition(|&(base, _)| base <= length).unwrap_or(0);
    let (base, extra) = LENGTH_BASES[index];
    write_literal_or_length(writer, FIRST_LENGTH_CODE + u32::try_from(index).unwrap_or(0));
    if extra > 0 {
        writer.bits(length - base, extra);
    }
}

/// Deflates with one fixed-code block and matches at distance 1 only.
#[must_use]
pub fn deflate_fixed(data: &[u8]) -> Vec<u8> {
    let mut writer = BitWriter { bytes: Vec::new(), current: 0, filled: 0 };
    writer.bits(FINAL_FIXED_BLOCK, BLOCK_HEADER_BITS);
    let mut position = 0;
    while position < data.len() {
        let run = if position == 0 {
            0
        } else {
            let previous = data[position - 1];
            data[position..].iter().take(MAX_MATCH).take_while(|&&byte| byte == previous).count()
        };
        if run >= MIN_MATCH {
            write_length(&mut writer, run);
            writer.code(DISTANCE_ONE_CODE, DISTANCE_CODE_BITS);
            position += run;
        } else {
            write_literal_or_length(&mut writer, u32::from(data[position]));
            position += 1;
        }
    }
    write_literal_or_length(&mut writer, END_OF_BLOCK);
    writer.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::page::render;

    // An independent decoder for fixed-code blocks with distance-1 matches,
    // written from RFC 1951's tables for testing only.
    fn inflate_fixed(data: &[u8]) -> Vec<u8> {
        let mut bit = 0_usize;
        let mut take = |count: usize| -> u32 {
            let mut value = 0;
            for offset in 0..count {
                let position = bit + offset;
                value |= u32::from((data[position / 8] >> (position % 8)) & 1) << offset;
            }
            bit += count;
            value
        };
        assert_eq!(take(3), FINAL_FIXED_BLOCK);
        let mut out: Vec<u8> = Vec::new();
        loop {
            // Read a code most significant bit first, growing from 7 bits.
            let mut code = 0;
            let mut length = 0;
            let symbol = loop {
                code = (code << 1) | take(1);
                length += 1;
                match (length, code) {
                    (7, 0..=0x17) => break 256 + code,
                    (8, 0x30..=0xBF) => break code - 0x30,
                    (8, 0xC0..=0xC7) => break 280 + code - 0xC0,
                    (9, 0x190..=0x1FF) => break 144 + code - 0x190,
                    (9, _) => panic!("invalid code"),
                    _ => {}
                }
            };
            match symbol {
                0..=255 => out.push(u8::try_from(symbol).unwrap()),
                256 => return out,
                _ => {
                    let (base, extra) = LENGTH_BASES[usize::try_from(symbol - FIRST_LENGTH_CODE).unwrap()];
                    let length = base + take(usize::try_from(extra).unwrap());
                    let mut distance_code = 0;
                    for _ in 0..DISTANCE_CODE_BITS {
                        distance_code = (distance_code << 1) | take(1);
                    }
                    assert_eq!(distance_code, DISTANCE_ONE_CODE);
                    let byte = *out.last().unwrap();
                    out.extend(std::iter::repeat_n(byte, usize::try_from(length).unwrap()));
                }
            }
        }
    }

    #[test]
    fn checksums_match_published_values() {
        // PNG's IEND chunk CRC, and the Adler-32 example from its definition.
        assert_eq!(crc32(b"IEND"), 0xAE42_6082);
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }

    #[test]
    fn deflate_round_trips_through_an_independent_decoder() {
        for data in [
            Vec::new(),
            b"abc".to_vec(),
            vec![7; 1000],
            (0..=255).cycle().take(3000).collect::<Vec<u8>>(),
            filter_up(&render(9, 0x1AB, true).indices, 480),
        ] {
            assert_eq!(inflate_fixed(&deflate_fixed(&data)), data);
        }
    }

    #[test]
    fn up_filter_reverses() {
        let frame = render(2, 0x300, false);
        let filtered = filter_up(&frame.indices, frame.width);
        let mut rebuilt = Vec::new();
        for (row, line) in filtered.chunks(frame.width + 1).enumerate() {
            let values = &line[1..];
            if row == 0 {
                rebuilt.extend_from_slice(values);
            } else {
                let above = rebuilt[(row - 1) * frame.width..row * frame.width].to_vec();
                rebuilt.extend(values.iter().zip(above).map(|(value, up)| value.wrapping_add(up)));
            }
        }
        assert_eq!(rebuilt, frame.indices);
    }

    #[test]
    fn a_page_compresses_to_a_few_kilobytes() {
        let size = encode(&render(1, 0x100, false)).len();
        // An unfiltered, uncompressed page is 240,000 bytes of indices.
        assert!(size < 40_000, "page image is {size} bytes");
    }
}
