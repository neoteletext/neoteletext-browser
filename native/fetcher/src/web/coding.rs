//! An answer's content coding (RFC 9110 section 8.4.1): gzip, deflate and
//! brotli bodies decoded in Rust, a bounded piece at a time. Each call
//! writes into room the caller gives, so what a small coded body expands
//! to is never held: the caller sends each piece on before asking for the
//! next, and counts what it has sent against its own bound.

use brotli_decompressor::{BrotliDecompressStream, BrotliResult, BrotliState, StandardAlloc};
use flate2::{Decompress, FlushDecompress, Status};

use super::limits::MAX_GZIP_HEADER;

/// The codings the web client asks for, as `Accept-Encoding` says them.
pub(crate) const ACCEPT_ENCODING: &str = "gzip, deflate, br";

/// A coded body that is not what its coding says.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Malformed;

/// What one step did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Step {
    /// The bytes of the input taken.
    pub(crate) taken: usize,
    /// The bytes written to the room given.
    pub(crate) written: usize,
}

/// A gzip member's fixed header: two magic bytes, the method, the flags,
/// four of time, the extra flags and the system (RFC 1952 section 2.3).
const GZIP_FIXED: usize = 10;
/// A gzip member's magic bytes and its one method, deflate.
const GZIP_START: [u8; 3] = [0x1F, 0x8B, 8];
/// The flags: a header checksum, an extra field, a name, a comment.
const FLAG_HEADER_CRC: u8 = 0x02;
const FLAG_EXTRA: u8 = 0x04;
const FLAG_NAME: u8 = 0x08;
const FLAG_COMMENT: u8 = 0x10;
/// The flag bits that must be zero.
const FLAG_RESERVED: u8 = 0xE0;
/// A gzip member's trailer: the checksum and the length, four bytes each.
const GZIP_TRAILER: usize = 8;

/// The length of the gzip member header `bytes` starts with, `None` when
/// more bytes are needed.
///
/// # Errors
/// [`Malformed`] when the bytes are no gzip header, or one longer than
/// [`MAX_GZIP_HEADER`].
pub(crate) fn gzip_header_length(bytes: &[u8]) -> Result<Option<usize>, Malformed> {
    let more = if bytes.len() > MAX_GZIP_HEADER { Err(Malformed) } else { Ok(None) };
    for (index, expected) in GZIP_START.iter().enumerate() {
        match bytes.get(index) {
            Some(byte) if byte != expected => return Err(Malformed),
            _ => {}
        }
    }
    let Some(&flags) = bytes.get(3) else { return more };
    if flags & FLAG_RESERVED != 0 {
        return Err(Malformed);
    }
    let mut length = GZIP_FIXED;
    if flags & FLAG_EXTRA != 0 {
        let (Some(&low), Some(&high)) = (bytes.get(length), bytes.get(length.saturating_add(1))) else { return more };
        length = length.saturating_add(2).saturating_add(usize::from(u16::from_le_bytes([low, high])));
    }
    for flag in [FLAG_NAME, FLAG_COMMENT] {
        if flags & flag != 0 {
            let Some(end) = bytes.get(length..).and_then(|rest| rest.iter().position(|&byte| byte == 0)) else { return more };
            length = length.saturating_add(end).saturating_add(1);
        }
    }
    if flags & FLAG_HEADER_CRC != 0 {
        length = length.saturating_add(2);
    }
    if length > MAX_GZIP_HEADER {
        return Err(Malformed);
    }
    if bytes.len() < length { more } else { Ok(Some(length)) }
}

/// Where a gzip body's reading is.
pub(crate) enum Gzip {
    /// In the member's header, kept until it is whole.
    Header(Vec<u8>),
    /// In the deflate stream.
    Body { inflate: Box<Decompress>, checksum: flate2::Crc },
    /// In the trailer, kept until it is whole.
    Trailer { kept: Vec<u8>, checksum: u32, length: u32 },
    /// Past the member; what follows is ignored, as common clients do.
    Done,
}

/// Where a deflate body's reading is. The coding is the zlib format (RFC
/// 9110 section 8.4.1.2), which some servers send without its wrapper, so
/// the first two bytes say which.
pub(crate) enum Deflate {
    /// The first bytes, until two are there.
    Start(Vec<u8>),
    /// In the stream.
    Body(Box<Decompress>),
    /// Past its end.
    Done,
}

/// A coded body being decoded.
pub(crate) enum Decoder {
    /// `gzip` or `x-gzip`.
    Gzip(Gzip),
    /// `deflate`.
    Deflate(Deflate),
    /// `br`.
    Brotli { state: Box<BrotliState<StandardAlloc, StandardAlloc, StandardAlloc>>, done: bool },
}

/// One call of inflate: the bytes it took and wrote, and whether the
/// stream ended.
fn inflate(inflate: &mut Decompress, input: &[u8], room: &mut [u8]) -> Result<(Step, bool), Malformed> {
    let (before_in, before_out) = (inflate.total_in(), inflate.total_out());
    let status = inflate.decompress(input, room, FlushDecompress::None).map_err(|_| Malformed)?;
    let taken = usize::try_from(inflate.total_in().saturating_sub(before_in)).map_err(|_| Malformed)?;
    let written = usize::try_from(inflate.total_out().saturating_sub(before_out)).map_err(|_| Malformed)?;
    Ok((Step { taken, written }, status == Status::StreamEnd))
}

/// Whether two bytes start a zlib stream (RFC 1950 section 2.2): the
/// method deflate, a window of at most 32 KiB, and a check that divides
/// by 31.
fn is_zlib(first: u8, second: u8) -> bool {
    first & 0x0F == 8 && first >> 4 <= 7 && u16::from_be_bytes([first, second]).is_multiple_of(31)
}

impl Decoder {
    /// The decoder of the one coding `coding` names, in any case; `None`
    /// for a coding the web client did not ask for, whose body is passed
    /// on as it came (the Fetch standard's "handle content codings").
    pub(crate) fn of(coding: &[u8]) -> Option<Self> {
        match coding.trim_ascii().to_ascii_lowercase().as_slice() {
            b"gzip" | b"x-gzip" => Some(Self::Gzip(Gzip::Header(Vec::new()))),
            b"deflate" => Some(Self::Deflate(Deflate::Start(Vec::new()))),
            b"br" => Some(Self::Brotli {
                state: Box::new(BrotliState::new(StandardAlloc::default(), StandardAlloc::default(), StandardAlloc::default())),
                done: false,
            }),
            _ => None,
        }
    }

    /// Decodes some of `input` into `room`. Call again with the input not
    /// taken while a step takes or writes anything; a step that does
    /// neither needs more input.
    ///
    /// # Errors
    /// [`Malformed`] when the bytes are not their coding's.
    pub(crate) fn step(&mut self, input: &[u8], room: &mut [u8]) -> Result<Step, Malformed> {
        match self {
            Self::Gzip(gzip) => gzip_step(gzip, input, room),
            Self::Deflate(deflate) => deflate_step(deflate, input, room),
            Self::Brotli { state, done } => {
                if *done {
                    return Ok(Step { taken: input.len(), written: 0 });
                }
                let (mut available_in, mut input_offset, mut available_out, mut output_offset, mut total) = (input.len(), 0, room.len(), 0, 0);
                let result =
                    BrotliDecompressStream(&mut available_in, &mut input_offset, input, &mut available_out, &mut output_offset, room, &mut total, state);
                match result {
                    BrotliResult::ResultFailure => return Err(Malformed),
                    BrotliResult::ResultSuccess => *done = true,
                    BrotliResult::NeedsMoreInput | BrotliResult::NeedsMoreOutput => {}
                }
                Ok(Step { taken: input_offset, written: output_offset })
            }
        }
    }

    /// Whether the coded body reached its end: a body that stops short of
    /// it was cut.
    pub(crate) const fn is_complete(&self) -> bool {
        matches!(self, Self::Gzip(Gzip::Done) | Self::Deflate(Deflate::Done) | Self::Brotli { done: true, .. })
    }
}

fn gzip_step(gzip: &mut Gzip, input: &[u8], room: &mut [u8]) -> Result<Step, Malformed> {
    match gzip {
        Gzip::Header(kept) => {
            // The header is found in what was kept and the new bytes
            // together; only its own bytes are taken.
            let before = kept.len();
            kept.extend_from_slice(input.get(..input.len().min(MAX_GZIP_HEADER.saturating_add(1))).unwrap_or_default());
            match gzip_header_length(kept)? {
                Some(length) => {
                    let taken = length.saturating_sub(before);
                    *gzip = Gzip::Body { inflate: Box::new(Decompress::new(false)), checksum: flate2::Crc::new() };
                    Ok(Step { taken, written: 0 })
                }
                None => Ok(Step { taken: kept.len().saturating_sub(before), written: 0 }),
            }
        }
        Gzip::Body { inflate: stream, checksum } => {
            let (step, ended) = inflate(stream, input, room)?;
            checksum.update(room.get(..step.written).unwrap_or_default());
            if ended {
                *gzip = Gzip::Trailer { kept: Vec::with_capacity(GZIP_TRAILER), checksum: checksum.sum(), length: checksum.amount() };
            }
            Ok(step)
        }
        Gzip::Trailer { kept, checksum, length } => {
            let taken = input.len().min(GZIP_TRAILER.saturating_sub(kept.len()));
            kept.extend_from_slice(input.get(..taken).unwrap_or_default());
            if let Ok(trailer) = <[u8; GZIP_TRAILER]>::try_from(kept.as_slice()) {
                let [c0, c1, c2, c3, l0, l1, l2, l3] = trailer;
                // RFC 1952 section 2.3.1: the CRC-32 and the length modulo
                // 2^32 of what the member decodes to, least significant
                // byte first.
                if u32::from_le_bytes([c0, c1, c2, c3]) != *checksum || u32::from_le_bytes([l0, l1, l2, l3]) != *length {
                    return Err(Malformed);
                }
                *gzip = Gzip::Done;
            }
            Ok(Step { taken, written: 0 })
        }
        Gzip::Done => Ok(Step { taken: input.len(), written: 0 }),
    }
}

fn deflate_step(deflate: &mut Deflate, input: &[u8], room: &mut [u8]) -> Result<Step, Malformed> {
    match deflate {
        Deflate::Start(kept) => {
            let taken = input.len().min(2_usize.saturating_sub(kept.len()));
            kept.extend_from_slice(input.get(..taken).unwrap_or_default());
            if let &[first, second] = kept.as_slice() {
                let mut stream = Box::new(Decompress::new(is_zlib(first, second)));
                // The two bytes kept are the stream's first: nothing can
                // be written from them alone.
                let (step, ended) = inflate(&mut stream, &[first, second], &mut [])?;
                if step.taken != 2 && !ended {
                    return Err(Malformed);
                }
                *deflate = if ended { Deflate::Done } else { Deflate::Body(stream) };
            }
            Ok(Step { taken, written: 0 })
        }
        Deflate::Body(stream) => {
            let (step, ended) = inflate(stream, input, room)?;
            if ended {
                *deflate = Deflate::Done;
            }
            Ok(step)
        }
        Deflate::Done => Ok(Step { taken: input.len(), written: 0 }),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// `body` as a gzip member, a zlib stream and a bare deflate stream.
    pub(crate) fn gzip(body: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(body).unwrap();
        encoder.finish().unwrap()
    }

    fn zlib(body: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(body).unwrap();
        encoder.finish().unwrap()
    }

    fn bare(body: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(body).unwrap();
        encoder.finish().unwrap()
    }

    /// "Hello, brotli!\n" three times, as Google's brotli encoder 1.2
    /// writes it at quality 11 (made on 22 November 2026 with its Python
    /// binding; the browser has no brotli encoder).
    pub(crate) const BROTLI: [u8; 35] = [
        0x1B, 0x2C, 0x00, 0xF8, 0x9D, 0x09, 0x76, 0xAC, 0x15, 0x2B, 0x5D, 0xF2, 0xC2, 0x5A, 0x01, 0x04, 0x49, 0x4C, 0x95, 0xC9, 0x25, 0x5B, 0xDA, 0x5B, 0x86,
        0xB0, 0xD0, 0x45, 0x53, 0x21, 0x40, 0x08, 0xE7, 0x5F, 0x00,
    ];
    pub(crate) const BROTLI_TEXT: &[u8] = b"Hello, brotli!\nHello, brotli!\nHello, brotli!\n";

    /// Decodes `coded` given `at_a_time` bytes a call into `room` bytes of
    /// room.
    pub(crate) fn decode(coding: &str, coded: &[u8], at_a_time: usize, room: usize) -> Result<(Vec<u8>, bool), Malformed> {
        let mut decoder = Decoder::of(coding.as_bytes()).unwrap();
        let mut out = Vec::new();
        let mut buffer = vec![0_u8; room];
        for piece in coded.chunks(at_a_time) {
            let mut rest = piece;
            loop {
                let step = decoder.step(rest, &mut buffer)?;
                out.extend_from_slice(&buffer[..step.written]);
                rest = &rest[step.taken..];
                if step.taken == 0 && step.written == 0 {
                    break;
                }
            }
            assert!(rest.is_empty() || decoder.is_complete(), "input left with nothing done");
        }
        // What a last step can still write without input.
        loop {
            let step = decoder.step(&[], &mut buffer)?;
            out.extend_from_slice(&buffer[..step.written]);
            if step.written == 0 {
                break;
            }
        }
        Ok((out, decoder.is_complete()))
    }

    #[test]
    fn each_coding_decodes_however_its_bytes_arrive() {
        let body: Vec<u8> = (0..20_000_u32).flat_map(|n| format!("line {n} of the page\n").into_bytes()).collect();
        for (coding, coded) in [("gzip", gzip(&body)), ("X-GZIP", gzip(&body)), ("deflate", zlib(&body)), ("deflate", bare(&body))] {
            for (at_a_time, room) in [(1, 7), (3, 1), (4096, 65_536), (coded.len(), 100)] {
                assert_eq!(decode(coding, &coded, at_a_time, room), Ok((body.clone(), true)), "{coding} {at_a_time} {room}");
            }
        }
        for (at_a_time, room) in [(1, 1), (5, 4096), (BROTLI.len(), 3)] {
            assert_eq!(decode("br", &BROTLI, at_a_time, room), Ok((BROTLI_TEXT.to_vec(), true)), "br {at_a_time} {room}");
        }
        assert!(Decoder::of(b"zstd").is_none() && Decoder::of(b"identity").is_none() && Decoder::of(b"compress").is_none());
    }

    #[test]
    fn a_gzip_header_with_every_field_is_read() {
        let plain = gzip(b"the body");
        // Flags for an extra field, a name, a comment and a header
        // checksum, then each; the deflate stream and trailer follow.
        let mut member = vec![0x1F, 0x8B, 8, FLAG_EXTRA | FLAG_NAME | FLAG_COMMENT | FLAG_HEADER_CRC, 0, 0, 0, 0, 0, 255];
        member.extend_from_slice(&[3, 0, b'x', b'y', b'z']);
        member.extend_from_slice(b"name\0comment\0");
        member.extend_from_slice(&[0xAB, 0xCD]);
        assert_eq!(gzip_header_length(&member), Ok(Some(member.len())));
        for cut in 0..member.len() {
            assert_eq!(gzip_header_length(&member[..cut]), Ok(None), "cut at {cut}");
        }
        member.extend_from_slice(&plain[GZIP_FIXED..]);
        assert_eq!(decode("gzip", &member, 1, 16), Ok((b"the body".to_vec(), true)));
    }

    #[test]
    fn malformed_and_cut_bodies_are_told() {
        let coded = gzip(b"the body of the page, the body of the page");
        let mut wrong_magic = coded.clone();
        wrong_magic[1] = 0x8C;
        assert_eq!(decode("gzip", &wrong_magic, 4096, 4096), Err(Malformed));
        let mut reserved = coded.clone();
        reserved[3] = 0x80;
        assert_eq!(decode("gzip", &reserved, 4096, 4096), Err(Malformed));
        let mut wrong_checksum = coded.clone();
        let at = coded.len() - GZIP_TRAILER;
        wrong_checksum[at] ^= 1;
        assert_eq!(decode("gzip", &wrong_checksum, 4096, 4096), Err(Malformed));
        let mut wrong_length = coded.clone();
        let last = coded.len() - 1;
        wrong_length[last] ^= 1;
        assert_eq!(decode("gzip", &wrong_length, 4096, 4096), Err(Malformed));
        let mut wrong_stream = coded.clone();
        wrong_stream[GZIP_FIXED] = 0xFF;
        assert_eq!(decode("gzip", &wrong_stream, 4096, 4096), Err(Malformed));
        // Cut short: not malformed so far, and not complete.
        assert!(matches!(decode("gzip", &coded[..coded.len() - 3], 4096, 4096), Ok((_, false))));
        assert!(matches!(decode("deflate", &zlib(b"the body")[..5], 4096, 4096), Ok((_, false))));
        assert!(matches!(decode("br", &BROTLI[..10], 4096, 4096), Ok((_, false))));
        assert_eq!(decode("br", &[0xFF; 32], 4096, 4096), Err(Malformed));
        // A name that never ends is refused once past the bound.
        let mut endless = vec![0x1F, 0x8B, 8, FLAG_NAME, 0, 0, 0, 0, 0, 255];
        endless.extend(std::iter::repeat_n(b'a', MAX_GZIP_HEADER));
        assert_eq!(gzip_header_length(&endless), Err(Malformed));
        assert_eq!(decode("gzip", &endless, 100, 100), Err(Malformed));
        // What follows a member is ignored.
        let mut followed = coded;
        followed.extend_from_slice(b"trailing bytes");
        assert_eq!(decode("gzip", &followed, 4096, 4096).map(|(_, complete)| complete), Ok(true));
    }

    /// A small coded body that decodes to much is written only as far as
    /// the room given: a megabyte of zeros comes out a kilobyte a step.
    #[test]
    fn a_step_writes_no_more_than_its_room() {
        let coded = gzip(&vec![0_u8; 1 << 20]);
        assert!(coded.len() < 2048, "a megabyte of zeros is about a kilobyte coded");
        let mut decoder = Decoder::of(b"gzip").unwrap();
        let mut room = [0_u8; 1024];
        let mut rest = coded.as_slice();
        let mut total = 0_usize;
        loop {
            let step = decoder.step(rest, &mut room).unwrap();
            assert!(step.written <= room.len());
            total += step.written;
            rest = &rest[step.taken..];
            if step.taken == 0 && step.written == 0 {
                break;
            }
        }
        assert_eq!(total, 1 << 20);
        assert!(decoder.is_complete());
    }
}
