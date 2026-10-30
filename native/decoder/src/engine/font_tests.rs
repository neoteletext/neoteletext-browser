//! The font setting drawn (REQ-1584): a page's text in Noto Teletext is
//! the font's own glyph, line for line in a normal cell and enlarged as
//! clause 12.2's double height and double size enlarge any character; in
//! Bedstead it is not; and a character Noto Teletext lacks is drawn in
//! Bedstead. Glyph bitmaps are compared, read from the bundled file by the
//! engine's reader, not pictures.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, reason = "a test")]

use super::*;
use crate::fonts::{NOTO_TELETEXT_BDF, read};
use interface_core::canvas::{CELL_HEIGHT, CELL_WIDTH};
use neoteletext_core::glyphs::GlyphSet;
use neoteletext_core::types::{PageAddress, SubCode};
use neoteletext_formats::author::DisplayPage;

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

/// Noto Teletext's glyph of `character`, as the bundled file holds it.
fn noto_glyph(character: char) -> Lines {
    let table = read(NOTO_TELETEXT_BDF).unwrap();
    let glyph = table.iter().find(|glyph| glyph.code_point == u32::from(character)).unwrap();
    assert!(glyph.tall, "Noto Teletext is a 12 by 20 font");
    glyph.rows
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

fn engine_with_page(page: &DisplayPage, font: TeletextFont) -> Engine {
    let mut bytes = Vec::new();
    neoteletext_formats::write::write_tti(&[&page.build().unwrap()], &mut bytes).unwrap();
    let mut engine = Engine::new();
    engine.set_font(font);
    engine.feed(&bytes);
    engine.finish();
    engine
}

fn page() -> DisplayPage {
    DisplayPage::new(PageAddress::from_mpp(0x100).unwrap(), SubCode::new(0))
}

/// REQ-1584: "A" in a normal cell, a double-height cell and a double-size
/// cell is Noto Teletext's glyph, line for line and enlarged; in Bedstead
/// it is another glyph.
#[test]
fn noto_teletext_draws_its_own_glyph_at_every_size_and_bedstead_does_not() {
    let mut page = page();
    page.text(3, 4, b"A").unwrap();
    page.text(6, 4, &[DOUBLE_HEIGHT, b'A']).unwrap();
    page.text(10, 4, &[DOUBLE_SIZE, b'A']).unwrap();
    let glyph = noto_glyph('A');
    let noto = engine_with_page(&page, TeletextFont::NotoTeletext).render(0x100).unwrap();
    assert_eq!(cell(&noto, 3, 4), glyph, "normal size: line for line");
    let half = |lines: &[u16]| -> Lines { core::array::from_fn(|line| lines[line / 2]) };
    assert_eq!(cell(&noto, 6, 5), half(&glyph[..10]), "double height, upper row: the glyph's upper half, each line twice");
    assert_eq!(cell(&noto, 7, 5), half(&glyph[10..]), "double height, lower row: its lower half");
    let upper = half(&glyph[..10]);
    let lower = half(&glyph[10..]);
    assert_eq!(cell(&noto, 10, 5), upper.map(|line| widen(line >> HALF_DOTS)), "double size, upper left");
    assert_eq!(cell(&noto, 10, 6), upper.map(|line| widen(line & HALF_MASK)), "double size, upper right");
    assert_eq!(cell(&noto, 11, 5), lower.map(|line| widen(line >> HALF_DOTS)), "double size, lower left");
    assert_eq!(cell(&noto, 11, 6), lower.map(|line| widen(line & HALF_MASK)), "double size, lower right");
    let bedstead = engine_with_page(&page, TeletextFont::Bedstead).render(0x100).unwrap();
    let drawn = cell(&bedstead, 3, 4);
    assert!(drawn.iter().any(|&line| line != 0), "Bedstead draws the A");
    assert_ne!(drawn, glyph, "Bedstead's A is not Noto Teletext's");
}

/// REQ-1584: a character Noto Teletext lacks is drawn in Bedstead, on a
/// page whose other text is Noto Teletext's: "À", which Latin G0 has only
/// as A with the grave mark of a Level 2.5 enhancement (Table 29).
#[test]
fn a_character_noto_teletext_lacks_is_drawn_in_bedstead() {
    assert!(read(NOTO_TELETEXT_BDF).unwrap().iter().all(|glyph| glyph.code_point != u32::from('À')), "Noto Teletext has no À");
    assert!(GlyphSet::bedstead().lookup('À').is_some(), "Bedstead has one");
    let mut page = page();
    page.text(3, 4, b"AA").unwrap();
    page.local().position(3, 0).unwrap().diacritic(5, b'A', GRAVE);
    let noto = engine_with_page(&page, TeletextFont::NotoTeletext).render(0x100).unwrap();
    let bedstead = engine_with_page(&page, TeletextFont::Bedstead).render(0x100).unwrap();
    assert_eq!(cell(&noto, 3, 4), noto_glyph('A'), "the plain A is Noto Teletext's");
    assert_eq!(cell(&noto, 3, 5), cell(&bedstead, 3, 5), "the À is Bedstead's");
    assert_ne!(cell(&noto, 3, 5), cell(&noto, 3, 4), "and is not the plain A");
}

/// REQ-1584: a page drawn in one font and then the other is drawn whole
/// in the second, though nothing on it changed, so a page on screen
/// changes font at once.
#[test]
fn a_page_drawn_again_after_the_font_changes_is_in_the_new_font() {
    let mut page = page();
    page.text(3, 4, b"A").unwrap();
    let mut engine = engine_with_page(&page, TeletextFont::Bedstead);
    let first = engine.render(0x100).unwrap();
    engine.set_font(TeletextFont::NotoTeletext);
    let second = engine.render_over(0x100, 0, Some(&first.indices), Vec::new()).unwrap();
    assert_eq!(cell(&second, 3, 4), noto_glyph('A'));
    engine.set_font(TeletextFont::Bedstead);
    let third = engine.render_over(0x100, 0, Some(&second.indices), Vec::new()).unwrap();
    assert_eq!(cell(&third, 3, 4), cell(&first, 3, 4), "and back");
}

/// The bundled file is the engine's, whose SHA-256 its
/// `third_party/noto/PROVENANCE.md` records at the pinned commit, and the
/// engine's reader takes all its 848 glyphs.
#[test]
fn the_bundled_font_is_the_engines_and_its_reader_takes_it() {
    /// `third_party/noto/PROVENANCE.md`, "Checksums", at the pinned commit.
    const ENGINE_SHA256: &[u8] = b"82fe9e2bb463bfc301ba4509c21542fecef7f5a8a27d0c87de0f5a529a7a4a4a";
    /// The same file's `CHARS` line.
    const CHARACTERS: usize = 848;
    let digest = neoteletext_web::sha256::digest(NOTO_TELETEXT_BDF);
    assert_eq!(neoteletext_web::sha256::to_hex(&digest).as_slice(), ENGINE_SHA256);
    assert_eq!(read(NOTO_TELETEXT_BDF).unwrap().len(), CHARACTERS);
}
