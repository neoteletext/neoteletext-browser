//! Page images' tests: every form reads back exactly, and a broken image is
//! refused.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]

use super::*;

const PALETTE: [[u8; 3]; 3] = [[0, 0, 0], [255, 0, 0], [17, 51, 255]];

fn page() -> Vec<u8> {
    (0..PAGE_DOTS).map(|dot| u8::from(dot / 1000 % 3 == 1)).collect()
}

/// Requirement 9.3: each form gives back the exact palette, screen
/// colour and dots.
#[test]
fn every_form_reads_back_exactly() {
    let first = page();
    let runs = encode(&PALETTE, 2, &first, None).unwrap();
    assert_eq!(runs[1 + 3 * 3 + 1], FORM_RUNS);
    assert_eq!(decode(&runs, None).unwrap(), Image { palette: PALETTE.to_vec(), screen: 2, indices: first.clone() });

    let mut flashed = first.clone();
    flashed[1234..1300].fill(2);
    flashed[PAGE_DOTS - 1] = 2;
    let against = encode(&PALETTE, 2, &flashed, Some((7, &first))).unwrap();
    assert_eq!(base(&against), Some(7));
    assert!(against.len() < 200, "a few changed dots are a few bytes: {}", against.len());
    assert_eq!(decode(&against, Some((7, &first))).unwrap().indices, flashed);
    assert_eq!(decode(&against, Some((6, &first))), Err(FrameError::Base), "against another frame");
    assert_eq!(decode(&against, None), Err(FrameError::Base), "against no frame");

    // Alternate dots make the longest runs: raw is chosen, and the
    // largest image is within its bound.
    let noise: Vec<u8> = (0..PAGE_DOTS).map(|dot| u8::try_from(dot % 2).unwrap()).collect();
    let raw = encode(&PALETTE, 0, &noise, Some((1, &first))).unwrap();
    assert_eq!(raw[1 + 3 * 3 + 1], FORM_RAW);
    assert!(raw.len() <= MAX_IMAGE);
    assert_eq!(decode(&raw, None).unwrap().indices, noise);
    assert_eq!(base(&raw), None);
}

/// Requirement 9.3: anything else is refused.
#[test]
fn a_broken_image_is_refused() {
    let first = page();
    assert_eq!(encode(&PALETTE, 3, &first, None), None, "screen outside the palette");
    assert_eq!(encode(&[], 0, &first, None), None);
    assert_eq!(encode(&PALETTE, 0, &first[1..], None), None);
    let good = encode(&PALETTE, 1, &first, None).unwrap();
    let broken = |at: usize, value: u8| {
        let mut bytes = good.clone();
        bytes[at] = value;
        decode(&bytes, None)
    };
    assert_eq!(broken(0, 0), Err(FrameError::Palette));
    assert_eq!(broken(0, 33), Err(FrameError::Palette));
    assert_eq!(broken(10, 3), Err(FrameError::Index), "screen");
    assert_eq!(broken(11, 9), Err(FrameError::Form));
    assert_eq!(broken(12, 3), Err(FrameError::Index), "a dot");
    assert_eq!(broken(13, 0), Err(FrameError::Empty));
    assert_eq!(decode(&good[..good.len() - 2], None), Err(FrameError::Shape), "short");
    let mut long = good.clone();
    long.extend_from_slice(&[0, 1]);
    assert_eq!(decode(&long, None), Err(FrameError::Shape), "too many dots");
    // A changes form whose groups run past the page.
    let mut flashed = first.clone();
    flashed[0] = 2;
    let mut against = encode(&PALETTE, 0, &flashed, Some((1, &first))).unwrap();
    let groups = 1 + 9 + 2 + VERSION_BYTES;
    against[groups..groups + 2].copy_from_slice(&u16::MAX.to_be_bytes());
    assert_eq!(decode(&against, Some((1, &first))), Err(FrameError::Shape));
    // Every dot unchanged, but the palette shrank below the held dots'
    // index: each dot is checked against the new palette.
    let held: Vec<u8> = vec![2; PAGE_DOTS];
    let mut unchanged = header(&PALETTE[..2], 0, FORM_CHANGES, 0);
    unchanged.extend_from_slice(&5_u32.to_be_bytes());
    let mut left = PAGE_DOTS;
    while left > 0 {
        let same = left.min(MAX_GROUP);
        unchanged.extend_from_slice(&u16::try_from(same).unwrap().to_be_bytes());
        unchanged.extend_from_slice(&0_u16.to_be_bytes());
        left -= same;
    }
    assert_eq!(decode(&unchanged, Some((5, &held))), Err(FrameError::Index));
}
