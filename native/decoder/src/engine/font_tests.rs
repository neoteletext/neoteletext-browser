//! The engine's compiled font drawn through the decoder (the engine's
//! DEC-1761): a page's text is Noto Teletext's own glyph, line for line in a
//! normal cell and enlarged as clause 12.2's double height and double size
//! enlarge any character, and a page in one of the engine's own character
//! sets is drawn in it. Each page goes through the engine's reader, decoder
//! and renderer as bytes of a TTI file. Glyph bitmaps are compared, not
//! pictures.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]

use super::*;
use interface_core::canvas::{CELL_HEIGHT, CELL_WIDTH};
use neoteletext_core::charset::{CharRef, CharSet, G0, G0Set, JoinForm, joined_forms, to_unicode};
use neoteletext_core::glyphs::GlyphSet;
use neoteletext_core::types::{PageAddress, SubCode};
use neoteletext_formats::author::{DisplayPage, neutral_fields};

/// The cell's lines, as the engine's glyphs give them: 12 pixels, bit 11
/// the leftmost.
type Lines = [u16; CELL_HEIGHT as usize];

/// White, the text's colour on every test page.
const WHITE: [u8; 3] = [255, 255, 255];
/// Spacing attributes (EN 300 706 Table 26): double height, double size.
const DOUBLE_HEIGHT: u8 = 0x0D;
const DOUBLE_SIZE: u8 = 0x0F;
/// The grave accent's diacritical mark (Table 29, column mode 10001, the
/// mark of G2 position 4/1).
const GRAVE: u8 = 1;
/// The half of a 12-pixel line each half of a double-width character
/// enlarges (Table 26 note: double width shows each half of the character
/// across a cell).
const HALF_DOTS: u32 = 6;
const HALF_MASK: u16 = 0b11_1111;

/// The designations of the engine's own sets (its `EXTENDED_TABLE_32`, the
/// values Table 32 reserves), a group of four bits and an option of three:
/// groups 1001 and 1100, with options 000, 010 and 100.
const PERSIAN: u8 = 0b100_1000;
const THAI: u8 = 0b110_0010;
const LAO: u8 = 0b110_0100;

/// Noto Teletext's glyph of `character`, as the engine compiles it.
fn noto_glyph(character: char) -> Lines {
    *GlyphSet::noto_teletext().lookup(character).unwrap()
}

/// The code of `character` in a G0 set.
fn code_of(set: G0Set, character: char) -> u8 {
    let set = CharSet::G0(G0 { set, national: None });
    (0x20..=0x7F).find(|&code| to_unicode(CharRef::new(set, code)).base == character).unwrap()
}

/// The code point the engine draws for a joined form of an Arabic letter.
fn joined(letter: char, form: JoinForm) -> char {
    joined_forms().iter().find(|&&(each, shape, _)| each == letter && shape == form).unwrap().2
}

/// A page whose X/28/0 Format 1 designates `value` (Table 32): how a page
/// chooses a set of its own, whatever the receiver's region.
fn page_in(value: u8) -> DisplayPage {
    let mut page = page();
    page.page_data(neutral_fields(value));
    page
}

/// The pixels of the cell at `row` and `column` drawn in white.
fn cell(rendered: &Rendered, row: usize, column: usize) -> Lines {
    let (width, cell_width, cell_height) = (usize::from(PAGE_WIDTH), usize::from(CELL_WIDTH), usize::from(CELL_HEIGHT));
    let mut lines = [0u16; CELL_HEIGHT as usize];
    for (line, out) in lines.iter_mut().enumerate() {
        for dot in 0..cell_width {
            let index = rendered.indices[(row * cell_height + line) * width + column * cell_width + dot];
            if rendered.palette[usize::from(index)] == WHITE {
                *out |= 1 << (cell_width - 1 - dot);
            }
        }
    }
    lines
}

/// Each of a half's six pixels drawn twice.
fn widen(six: u16) -> u16 {
    (0..HALF_DOTS).fold(0, |wide, dot| if (six >> dot) & 1 == 1 { wide | (0b11 << (dot * 2)) } else { wide })
}

fn engine_with_page(page: &DisplayPage) -> Engine {
    let mut bytes = Vec::new();
    neoteletext_formats::write::write_tti(&[&page.build().unwrap()], &mut bytes).unwrap();
    let mut engine = Engine::new();
    engine.feed(&bytes);
    engine.finish();
    engine
}

fn page() -> DisplayPage {
    DisplayPage::new(PageAddress::from_mpp(0x100).unwrap(), SubCode::new(0))
}

/// DEC-1761: "A" in a normal cell, a double-height cell and a double-size
/// cell is Noto Teletext's glyph, line for line and enlarged.
#[test]
fn a_page_is_drawn_in_noto_teletext_at_every_size() {
    let mut page = page();
    page.text(3, 4, b"A").unwrap();
    page.text(6, 4, &[DOUBLE_HEIGHT, b'A']).unwrap();
    page.text(10, 4, &[DOUBLE_SIZE, b'A']).unwrap();
    let glyph = noto_glyph('A');
    let drawn = engine_with_page(&page).render(0x100).unwrap();
    assert_eq!(cell(&drawn, 3, 4), glyph, "normal size: line for line");
    let half = |lines: &[u16]| -> Lines { core::array::from_fn(|line| lines[line / 2]) };
    assert_eq!(cell(&drawn, 6, 5), half(&glyph[..10]), "double height, upper row: the glyph's upper half, each line twice");
    assert_eq!(cell(&drawn, 7, 5), half(&glyph[10..]), "double height, lower row: its lower half");
    let upper = half(&glyph[..10]);
    let lower = half(&glyph[10..]);
    assert_eq!(cell(&drawn, 10, 5), upper.map(|line| widen(line >> HALF_DOTS)), "double size, upper left");
    assert_eq!(cell(&drawn, 10, 6), upper.map(|line| widen(line & HALF_MASK)), "double size, upper right");
    assert_eq!(cell(&drawn, 11, 5), lower.map(|line| widen(line >> HALF_DOTS)), "double size, lower left");
    assert_eq!(cell(&drawn, 11, 6), lower.map(|line| widen(line & HALF_MASK)), "double size, lower right");
}

/// DEC-1761: "À", which Latin G0 has only as A with the grave mark of a
/// Level 2.5 enhancement (Table 29), is the font's own precomposed letter.
#[test]
fn a_letter_and_a_mark_are_the_fonts_composed_letter() {
    let mut page = page();
    page.text(3, 4, b"AA").unwrap();
    page.local().position(3, 0).unwrap().diacritic(5, b'A', GRAVE);
    let drawn = engine_with_page(&page).render(0x100).unwrap();
    assert_eq!(cell(&drawn, 3, 4), noto_glyph('A'), "the plain A");
    assert_eq!(cell(&drawn, 3, 5), noto_glyph('À'), "the À");
}

/// TODO-1585: an Arabic-script page. Persian, the engine's own set 1001 000,
/// is sent as plain letters in visual order, and the receiver joins them:
/// "بسم" reads right to left, so its cells hold م, س and ب.
#[test]
fn a_persian_page_is_joined_and_drawn_through_the_decoder() {
    let codes = ['م', 'س', 'ب'].map(|letter| code_of(G0Set::Persian, letter));
    let mut page = page_in(PERSIAN);
    page.text(3, 4, &codes).unwrap();
    let drawn = engine_with_page(&page).render(0x100).unwrap();
    assert_eq!(cell(&drawn, 3, 4), noto_glyph(joined('م', JoinForm::Final)), "the last letter, on the left");
    assert_eq!(cell(&drawn, 3, 5), noto_glyph(joined('س', JoinForm::Medial)), "the middle letter");
    assert_eq!(cell(&drawn, 3, 6), noto_glyph(joined('ب', JoinForm::Initial)), "the first letter, on the right");
}

/// TODO-1585: a Thai page, set 1100 010: a consonant and a side vowel, and in
/// the row above the consonant's upper mark, which the engine's decision
/// E.80 draws at the bottom of its own cell.
#[test]
fn a_thai_page_is_drawn_through_the_decoder() {
    let mut page = page_in(THAI);
    page.text(2, 4, &[code_of(G0Set::Thai, '\u{0E48}')]).unwrap();
    page.text(3, 4, &[code_of(G0Set::Thai, 'ก'), code_of(G0Set::Thai, 'า')]).unwrap();
    let drawn = engine_with_page(&page).render(0x100).unwrap();
    assert_eq!(cell(&drawn, 2, 4), noto_glyph('\u{0E48}'), "the tone mark");
    assert_eq!(cell(&drawn, 3, 4), noto_glyph('ก'));
    assert_eq!(cell(&drawn, 3, 5), noto_glyph('า'));
}

/// TODO-1585: a Lao page, set 1100 100: Lao is the engine's Indic-derived
/// script beside Thai, as its languages design leaves the scripts of India
/// out of scope.
#[test]
fn a_lao_page_is_drawn_through_the_decoder() {
    let mut page = page_in(LAO);
    page.text(3, 4, &[code_of(G0Set::Lao, 'ກ'), code_of(G0Set::Lao, 'າ')]).unwrap();
    let drawn = engine_with_page(&page).render(0x100).unwrap();
    assert_eq!(cell(&drawn, 3, 4), noto_glyph('ກ'));
    assert_eq!(cell(&drawn, 3, 5), noto_glyph('າ'));
}
