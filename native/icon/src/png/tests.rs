#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, clippy::cast_possible_truncation, reason = "a test")]

use super::*;
use crate::source;

/// Annex D's CRC is gzip's: the check value of "123456789" (as every
/// CRC-32 catalogue gives it) and the release files' own function agree.
#[test]
fn the_crc_is_the_specifications() {
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    for bytes in [&b""[..], b"IEND", crate::SOURCE] {
        assert_eq!(crc32(bytes), update::gzip::crc32(bytes));
    }
}

/// Section 9: `filter` applied to `line` given the row above, as a writer
/// would.
fn filtered(filter: u8, line: &[u8], previous: &[u8]) -> Vec<u8> {
    let mut out = vec![filter];
    for at in 0..line.len() {
        let left = if at >= BYTES_PER_PIXEL { line[at - BYTES_PER_PIXEL] } else { 0 };
        let up = previous[at];
        let up_left = if at >= BYTES_PER_PIXEL { previous[at - BYTES_PER_PIXEL] } else { 0 };
        let predictor = match filter {
            FILTER_SUB => left,
            FILTER_UP => up,
            FILTER_AVERAGE => u16::midpoint(u16::from(left), u16::from(up)) as u8,
            FILTER_PAETH => paeth(left, up, up_left),
            _ => 0,
        };
        out.push(line[at].wrapping_sub(predictor));
    }
    out
}

/// A PNG of `image` with each row given the filter its number chooses,
/// the image data split over `parts` chunks, and `extra` chunks after the
/// header.
fn with_filters(image: &Image, parts: usize, extra: &[([u8; 4], &[u8])]) -> Vec<u8> {
    let row = image.width() as usize * BYTES_PER_PIXEL;
    let mut raw = Vec::new();
    let mut previous = vec![0; row];
    for (number, line) in image.rgba().chunks_exact(row).enumerate() {
        raw.extend(filtered((number % 5) as u8, line, &previous));
        previous = line.to_vec();
    }
    let compressed = compress_to_vec_zlib(&raw, LEVEL_STRONGEST);
    let mut out = SIGNATURE.to_vec();
    push_chunk(&mut out, IHDR, &header_data(image)).unwrap();
    for (kind, data) in extra {
        push_chunk(&mut out, *kind, data).unwrap();
    }
    for part in compressed.chunks(compressed.len().div_ceil(parts)) {
        push_chunk(&mut out, IDAT, part).unwrap();
    }
    push_chunk(&mut out, IEND, &[]).unwrap();
    out
}

/// A picture of every byte value, so each filter meets every case.
fn noise(side: u32) -> Image {
    let rgba = (0..side * side * 4).map(|n| (n.wrapping_mul(2_654_435_761) >> 13) as u8).collect();
    Image::new(side, side, rgba).unwrap()
}

/// Section 9.2: rows with each of the five filters, the image data split
/// over chunks, and ancillary chunks and a truecolour image's suggested
/// palette passed over, read back as written.
#[test]
fn every_filter_is_undone() {
    let image = noise(37);
    let png = with_filters(&image, 3, &[(*b"tEXt", b"Comment\0x"), (PLTE, &[0, 0, 0])]);
    assert_eq!(decode(&png).unwrap(), image);
    // The art's own file uses Sub, Up and Paeth (measured).
    assert_eq!(decode(crate::SOURCE).unwrap().width(), crate::SOURCE_SIDE);
}

/// What [`encode`] writes, [`decode`] reads as it was, at every size.
#[test]
fn encoded_images_read_back() {
    let art = source().unwrap();
    for side in [16, 24, 32, 48, 64, 128, 256, 1024] {
        let image = art.scaled(side).unwrap();
        let png = encode(&image).unwrap();
        assert_eq!(decode(&png).unwrap(), image, "{side}");
        assert_eq!(&png[12..16], b"IHDR");
        assert_eq!(&png[37..41], b"sRGB", "the art's colour space is said");
    }
    assert_eq!(decode(&encode(&noise(5)).unwrap()).unwrap(), noise(5));
}

/// Every departure from an 8-bit RGBA PNG, and every fault in one, is
/// refused.
#[test]
fn malformed_files_are_refused() {
    let image = noise(4);
    let good = encode(&image).unwrap();
    assert!(decode(&good).is_ok());
    let header = |change: &dyn Fn(&mut Vec<u8>)| {
        let mut data = header_data(&image);
        change(&mut data);
        let mut out = SIGNATURE.to_vec();
        push_chunk(&mut out, IHDR, &data).unwrap();
        out.extend_from_slice(&good[8 + 25..]);
        out
    };
    let cases: Vec<(&str, Vec<u8>)> = vec![
        ("no signature", good[1..].to_vec()),
        ("a wrong CRC", {
            let mut bad = good.clone();
            bad[20] ^= 1;
            bad
        }),
        ("cut short", good[..good.len() - 3].to_vec()),
        ("bytes after the end", [good.clone(), vec![0]].concat()),
        ("16-bit", header(&|data| data[8] = 16)),
        ("truecolour without alpha", header(&|data| data[9] = 2)),
        ("interlaced", header(&|data| data[12] = 1)),
        ("another filter method", header(&|data| data[11] = 1)),
        ("no width", header(&|data| data[0..4].copy_from_slice(&0_u32.to_be_bytes()))),
        ("too wide", header(&|data| data[0..4].copy_from_slice(&(MAX_SIDE + 1).to_be_bytes()))),
        ("a short header", header(&|data| data.truncate(12))),
        ("an unknown critical chunk", with_filters(&image, 1, &[(*b"ZZZZ", b"")])),
        ("a second header", with_filters(&image, 1, &[(IHDR, &header_data(&image))])),
        ("a bad filter type", {
            let mut raw = vec![5];
            raw.extend_from_slice(&image.rgba()[..16]);
            for row in image.rgba().as_chunks::<16>().0.iter().skip(1) {
                raw.push(0);
                raw.extend_from_slice(row);
            }
            let mut out = SIGNATURE.to_vec();
            push_chunk(&mut out, IHDR, &header_data(&image)).unwrap();
            push_chunk(&mut out, IDAT, &compress_to_vec_zlib(&raw, 1)).unwrap();
            push_chunk(&mut out, IEND, &[]).unwrap();
            out
        }),
        ("too much image data", {
            let mut out = SIGNATURE.to_vec();
            push_chunk(&mut out, IHDR, &header_data(&image)).unwrap();
            push_chunk(&mut out, IDAT, &compress_to_vec_zlib(&[0; 100], 1)).unwrap();
            push_chunk(&mut out, IEND, &[]).unwrap();
            out
        }),
        ("too little image data", {
            let mut out = SIGNATURE.to_vec();
            push_chunk(&mut out, IHDR, &header_data(&image)).unwrap();
            push_chunk(&mut out, IDAT, &compress_to_vec_zlib(&[0; 10], 1)).unwrap();
            push_chunk(&mut out, IEND, &[]).unwrap();
            out
        }),
        ("the header not first", {
            let mut out = SIGNATURE.to_vec();
            push_chunk(&mut out, SRGB, &[0]).unwrap();
            out.extend_from_slice(&good[8..]);
            out
        }),
    ];
    for (what, bytes) in cases {
        assert!(decode(&bytes).is_err(), "{what} was read");
    }
}
