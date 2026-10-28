#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, clippy::cast_possible_truncation, reason = "a test")]

use std::collections::BTreeSet;

use super::*;

const BLACK: [u8; 4] = [0, 0, 0, 0xff];
const MAGENTA: [u8; 4] = [0xff, 0, 0xff, 0xff];
const BLUE: [u8; 4] = [0, 0, 0xff, 0xff];

fn colours(image: &Image) -> BTreeSet<[u8; 4]> {
    image.rgba().as_chunks::<BYTES_PER_PIXEL>().0.iter().copied().collect()
}

/// Every side any platform's icon is made at.
fn every_side() -> Vec<u32> {
    let mut sides: Vec<u32> = MACOS_ICONSET.iter().map(|(_, side)| *side).chain(windows::SIZES).chain(update::platform::LINUX_ICON_SIZES).collect();
    sides.extend([SOURCE_SIDE, DOCK_SIDE]);
    sides.sort_unstable();
    sides.dedup();
    sides
}

/// The art as the owner gave it: 128 pixels square, opaque, in black,
/// magenta and blue alone.
#[test]
fn the_art_is_read_as_given() {
    let art = source().unwrap();
    assert_eq!((art.width(), art.height()), (SOURCE_SIDE, SOURCE_SIDE));
    assert_eq!(colours(&art), BTreeSet::from([BLACK, MAGENTA, BLUE]));
    assert_eq!(art.pixel(0, 0), Some(BLACK));
    assert_eq!(art.pixel(SOURCE_SIDE, 0), None);
}

/// The art is drawn in blocks of 2 by 2 pixels, and its letters on a grid
/// of 4 by 4 whose columns start 2 pixels in: so 64 is exact, and each
/// pixel of a 32 takes one cell of that grid whole.
#[test]
fn the_art_is_a_grid_of_blocks() {
    let art = source().unwrap();
    for y in 0..SOURCE_SIDE {
        for x in 0..SOURCE_SIDE {
            assert_eq!(art.pixel(x, y), art.pixel(x & !1, y & !1), "{x}, {y}: a block of 2 by 2");
        }
    }
    let half = art.scaled(SOURCE_SIDE / 2).unwrap();
    assert_eq!(half.scaled(SOURCE_SIDE).unwrap(), art, "64 holds every block, and doubles back to the art");
    let quarter = art.scaled(SOURCE_SIDE / 4).unwrap();
    for cell_y in 0..SOURCE_SIDE / 4 {
        for cell_x in 0..SOURCE_SIDE / 4 - 1 {
            let colour = art.pixel(4 * cell_x + 2, 4 * cell_y).unwrap();
            for y in 4 * cell_y..4 * cell_y + 4 {
                for x in 4 * cell_x + 2..4 * cell_x + 6 {
                    assert_eq!(art.pixel(x, y), Some(colour), "cell {cell_x}, {cell_y} is one colour");
                }
            }
            assert_eq!(quarter.pixel(cell_x, cell_y), Some(colour), "32 takes cell {cell_x}, {cell_y}");
        }
    }
}

/// Nearest neighbour: the art at its own side is itself; a multiple of it
/// makes each pixel a block of one colour; no size makes a colour the art
/// does not have.
#[test]
fn every_size_is_nearest_neighbour() {
    let art = source().unwrap();
    assert_eq!(art.scaled(SOURCE_SIDE).unwrap(), art);
    for factor in [2, 4, 8] {
        let large = art.scaled(SOURCE_SIDE * factor).unwrap();
        for y in 0..SOURCE_SIDE * factor {
            for x in 0..SOURCE_SIDE * factor {
                assert_eq!(large.pixel(x, y), art.pixel(x / factor, y / factor), "{factor}x at {x}, {y}");
            }
        }
    }
    for side in every_side() {
        let image = art.scaled(side).unwrap();
        assert_eq!((image.width(), image.height()), (side, side));
        assert!(colours(&image).is_subset(&colours(&art)), "{side}: no colour the art lacks");
    }
    // Each pixel of a 16 takes the art's pixel under its centre.
    let small = art.scaled(16).unwrap();
    for y in 0..16 {
        for x in 0..16 {
            assert_eq!(small.pixel(x, y), art.pixel(8 * x + 4, 8 * y + 4));
        }
    }
}

#[test]
fn sizes_out_of_bounds_are_refused() {
    let art = source().unwrap();
    assert!(art.scaled(0).is_err());
    assert!(art.scaled(MAX_SIDE + 1).is_err());
    assert!(every_side().iter().all(|side| *side <= MAX_SIDE));
    assert!(Image::new(2, 2, vec![0; 15]).is_err());
    assert!(Image::new(0, 0, Vec::new()).is_err());
}
