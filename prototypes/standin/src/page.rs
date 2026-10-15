//! Synthetic teletext pages at the native raster, standing in for
//! `neoteletext-core`'s renderer. A page is a deterministic function of its
//! channel and page number, so tests can predict every frame.

/// Character cells per row and rows per page (EN 300 706 clause 2).
pub const COLUMNS: usize = 40;
pub const ROWS: usize = 25;
/// Dots per character cell, as `neoteletext-core`'s `CELL_WIDTH` and
/// `CELL_HEIGHT`.
pub const CELL_WIDTH: usize = 12;
pub const CELL_HEIGHT: usize = 20;
pub const COLUMNS_DOTS: usize = COLUMNS * CELL_WIDTH;
pub const ROWS_DOTS: usize = ROWS * CELL_HEIGHT;
/// The Colour Map of Presentation Level 2.5: four tables of eight.
pub const MAX_COLOURS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rgb {
    pub red: u8,
    pub green: u8,
    pub blue: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Frame {
    pub channel: u32,
    pub page: u16,
    pub flash_on: bool,
    pub width: usize,
    pub height: usize,
    pub palette: Vec<Rgb>,
    pub indices: Vec<u8>,
}

const FULL: u8 = 0xFF;
const HALF: u8 = 0x77;

// Level 1 colour numbering: bit 0 red, bit 1 green, bit 2 blue
// (EN 300 706 table 26), at full intensity, then the same at half.
fn palette() -> Vec<Rgb> {
    [FULL, HALF]
        .iter()
        .flat_map(|&level| {
            (0..8_u8).map(move |colour| Rgb {
                red: if colour & 1 != 0 { level } else { 0 },
                green: if colour & 2 != 0 { level } else { 0 },
                blue: if colour & 4 != 0 { level } else { 0 },
            })
        })
        .collect()
}

const BLACK: u8 = 0;
const RED: u8 = 1;
const GREEN: u8 = 2;
const YELLOW: u8 = 3;
const BLUE: u8 = 4;
const CYAN: u8 = 6;
const WHITE: u8 = 7;
const DIM: u8 = 8;

// Mosaic cells are 2 by 3 blocks (EN 300 706 clause 15.7).
const MOSAIC_COLUMNS: usize = 2;
const MOSAIC_ROWS: usize = 3;

// A 3 by 5 pattern per decimal or hexadecimal digit, drawn for this
// project; one bit per block, top row first, most significant bit left.
const DIGITS: [[u8; 5]; 16] = [
    [0b111, 0b101, 0b101, 0b101, 0b111],
    [0b010, 0b110, 0b010, 0b010, 0b111],
    [0b111, 0b001, 0b111, 0b100, 0b111],
    [0b111, 0b001, 0b111, 0b001, 0b111],
    [0b101, 0b101, 0b111, 0b001, 0b001],
    [0b111, 0b100, 0b111, 0b001, 0b111],
    [0b111, 0b100, 0b111, 0b101, 0b111],
    [0b111, 0b001, 0b010, 0b010, 0b010],
    [0b111, 0b101, 0b111, 0b101, 0b111],
    [0b111, 0b101, 0b111, 0b001, 0b111],
    [0b111, 0b101, 0b111, 0b101, 0b101],
    [0b110, 0b101, 0b110, 0b101, 0b110],
    [0b111, 0b100, 0b100, 0b100, 0b111],
    [0b110, 0b101, 0b101, 0b101, 0b110],
    [0b111, 0b100, 0b110, 0b100, 0b111],
    [0b111, 0b100, 0b110, 0b100, 0b100],
];
const DIGIT_WIDTH: usize = 3;
const DIGIT_HEIGHT: usize = 5;

const HEADER_ROW: usize = 0;
const BODY_FIRST_ROW: usize = 2;
const BODY_LAST_ROW: usize = 18;
const FLASH_ROW: usize = 20;
const FASTEXT_ROW: usize = 24;
// The large page number: each digit block is this many cells square.
const BIG_BLOCK_CELLS: usize = 2;
const BIG_FIRST_ROW: usize = 6;
const BIG_FIRST_COLUMN: usize = 9;
// One in this many body cells carries a mosaic.
const MOSAIC_DENSITY: u32 = 3;

struct Canvas {
    indices: Vec<u8>,
}

impl Canvas {
    fn fill_cell_part(&mut self, column: usize, row: usize, part: (usize, usize, usize, usize), colour: u8) {
        let (left, top, width, height) = part;
        let x0 = column * CELL_WIDTH + left;
        let y0 = row * CELL_HEIGHT + top;
        for y in y0..y0 + height {
            let start = y * COLUMNS_DOTS + x0;
            self.indices[start..start + width].fill(colour);
        }
    }

    fn fill_cell(&mut self, column: usize, row: usize, colour: u8) {
        self.fill_cell_part(column, row, (0, 0, CELL_WIDTH, CELL_HEIGHT), colour);
    }

    // A 2 by 3 mosaic: each of the six bits lights one block.
    fn mosaic(&mut self, column: usize, row: usize, bits: u8, colour: u8) {
        let block_width = CELL_WIDTH / MOSAIC_COLUMNS;
        let block_height = CELL_HEIGHT / MOSAIC_ROWS;
        for block in 0..MOSAIC_COLUMNS * MOSAIC_ROWS {
            if bits & (1 << block) != 0 {
                let (x, y) = (block % MOSAIC_COLUMNS, block / MOSAIC_COLUMNS);
                self.fill_cell_part(column, row, (x * block_width, y * block_height, block_width, block_height), colour);
            }
        }
    }

    // A digit drawn in small blocks within single cells (one cell per digit
    // column of the pattern, using the cell's upper part).
    fn small_digit(&mut self, column: usize, row: usize, digit: usize, colour: u8) {
        let block_width = CELL_WIDTH / DIGIT_WIDTH;
        let block_height = CELL_HEIGHT / DIGIT_HEIGHT;
        for (y, pattern) in DIGITS[digit].iter().enumerate() {
            for x in 0..DIGIT_WIDTH {
                if pattern & (1 << (DIGIT_WIDTH - 1 - x)) != 0 {
                    self.fill_cell_part(column, row, (x * block_width, y * block_height, block_width, block_height), colour);
                }
            }
        }
    }

    fn big_digit(&mut self, column: usize, row: usize, digit: usize, colour: u8) {
        for (y, pattern) in DIGITS[digit].iter().enumerate() {
            for x in 0..DIGIT_WIDTH {
                if pattern & (1 << (DIGIT_WIDTH - 1 - x)) != 0 {
                    for dy in 0..BIG_BLOCK_CELLS {
                        for dx in 0..BIG_BLOCK_CELLS {
                            self.fill_cell(column + x * BIG_BLOCK_CELLS + dx, row + y * BIG_BLOCK_CELLS + dy, colour);
                        }
                    }
                }
            }
        }
    }
}

// A small fixed hash (FNV-1a over the inputs) so pages are deterministic.
fn hash(values: &[u32]) -> u32 {
    const SEED: u32 = 2_166_136_261;
    const PRIME: u32 = 16_777_619;
    values.iter().flat_map(|value| value.to_le_bytes()).fold(SEED, |state, byte| (state ^ u32::from(byte)).wrapping_mul(PRIME))
}

const HEX_DIGITS: usize = 3;
const NIBBLE_BITS: u32 = 4;
const NIBBLE_MASK: u16 = 0xF;

fn page_digits(page: u16) -> [usize; HEX_DIGITS] {
    let digit = |shift: u32| usize::from((page >> shift) & NIBBLE_MASK);
    [digit(2 * NIBBLE_BITS), digit(NIBBLE_BITS), digit(0)]
}

/// Renders one synthetic page.
#[must_use]
pub fn render(channel: u32, page: u16, flash_on: bool) -> Frame {
    let mut canvas = Canvas { indices: vec![BLACK; COLUMNS_DOTS * ROWS_DOTS] };
    let digits = page_digits(page);
    let accent = 1 + u8::try_from(hash(&[channel]) % 6).unwrap_or(0);

    // Header: the page number, then the channel number in its accent colour.
    for (offset, &digit) in digits.iter().enumerate() {
        canvas.small_digit(1 + offset, HEADER_ROW, digit, WHITE);
    }
    let channel_digits = channel % 1000;
    for (offset, divisor) in [100, 10, 1].iter().enumerate() {
        let digit = usize::try_from(channel_digits / divisor % 10).unwrap_or(0);
        canvas.small_digit(8 + offset, HEADER_ROW, digit, accent);
    }

    // Body: mosaics in a pattern fixed by channel and page.
    for row in BODY_FIRST_ROW..=BODY_LAST_ROW {
        for column in 0..COLUMNS {
            let value = hash(&[channel, u32::from(page), u32::try_from(row * COLUMNS + column).unwrap_or(0)]);
            if value.is_multiple_of(MOSAIC_DENSITY) {
                let colour = DIM + u8::try_from(value >> 8 & 7).unwrap_or(0);
                canvas.mosaic(column, row, u8::try_from(value >> 16 & 0x3F).unwrap_or(0), colour);
            }
        }
    }

    // The page number, large, over the body.
    for (offset, &digit) in digits.iter().enumerate() {
        let column = BIG_FIRST_COLUMN + offset * (DIGIT_WIDTH + 1) * BIG_BLOCK_CELLS;
        canvas.big_digit(column, BIG_FIRST_ROW, digit, if offset == 0 { accent } else { WHITE });
    }

    // A flashing row, shown only in the flash-on phase.
    if flash_on {
        for column in (2..COLUMNS - 2).step_by(2) {
            canvas.mosaic(column, FLASH_ROW, 0x3F, YELLOW);
        }
    }

    // Fastext row: four coloured keys.
    let key_width = COLUMNS / 4;
    for (key, colour) in [RED, GREEN, YELLOW, CYAN].into_iter().enumerate() {
        for column in key * key_width + 1..(key + 1) * key_width - 1 {
            canvas.fill_cell(column, FASTEXT_ROW, colour);
        }
    }
    canvas.fill_cell(0, FASTEXT_ROW, BLUE);

    Frame { channel, page, flash_on, width: COLUMNS_DOTS, height: ROWS_DOTS, palette: palette(), indices: canvas.indices }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raster_is_the_native_teletext_size() {
        let frame = render(1, 0x100, false);
        assert_eq!((frame.width, frame.height), (480, 500));
        assert_eq!(frame.indices.len(), 480 * 500);
        assert!(frame.palette.len() <= MAX_COLOURS);
        assert!(frame.indices.iter().all(|&index| usize::from(index) < frame.palette.len()));
    }

    #[test]
    fn pages_are_deterministic_and_distinct() {
        assert_eq!(render(3, 0x123, false), render(3, 0x123, false));
        assert_ne!(render(3, 0x123, false).indices, render(3, 0x124, false).indices);
        assert_ne!(render(3, 0x123, false).indices, render(4, 0x123, false).indices);
    }

    #[test]
    fn flash_changes_only_the_flash_row() {
        let off = render(5, 0x200, false);
        let on = render(5, 0x200, true);
        let changed: Vec<usize> = off.indices.iter().zip(&on.indices).enumerate().filter(|(_, (a, b))| a != b).map(|(i, _)| i / COLUMNS_DOTS / CELL_HEIGHT).collect();
        assert!(!changed.is_empty());
        assert!(changed.iter().all(|&row| row == FLASH_ROW));
    }

    #[test]
    fn page_digits_are_hexadecimal() {
        assert_eq!(page_digits(0x1FE), [1, 15, 14]);
    }
}
