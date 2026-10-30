//! The fonts a page's text is drawn in: Bedstead, the engine's compiled-in
//! font (its REQ-FNT-001), or "Noto Teletext", the engine's Noto-derived
//! 12 by 20 font (its REQ-FNT-012, decision E.77), chosen by the user's
//! setting. The Noto file is the engine's own BDF file, bundled in this
//! program so that the decoder, which may open no file, needs no new
//! access; it is read once, the first time a page is drawn in it, by the
//! engine's bounded reader of loaded fonts (its REQ-FNT-002) and checked by
//! the engine's loaded-font rules (its REQ-FNT-008). It is the user's font
//! for every character set it conforms for, as the engine's decision E.75
//! item 6 has the user choose a font for each set; a character it lacks,
//! and every set it does not conform for, is drawn in Bedstead, and row 0,
//! mosaics and the G3 characters are the engine's own whichever font is
//! chosen (its REQ-FNT-005 and decision E.72 item 4).

use std::sync::LazyLock;

use neoteletext_core::consts::FONT_MAX_CHARACTERS;
use neoteletext_core::glyphs::conformance::{FONT_SET_COUNT, FONT_SETS, conformance};
use neoteletext_core::glyphs::{GlyphSet, LoadedFont};
use neoteletext_core::types::FontGlyph;
use neoteletext_formats::bdf::{self, FontError};
use protocol::settings::TeletextFont;

/// The engine's `third_party/noto/noto-teletext.bdf`, byte for byte, at the
/// commit the workspace pins (`fonts/noto/PROVENANCE.md`).
pub(crate) const NOTO_TELETEXT_BDF: &[u8] = include_bytes!("../fonts/noto/noto-teletext.bdf");

/// Bedstead alone.
static BEDSTEAD: GlyphSet<'static> = GlyphSet::bedstead();

/// Noto Teletext's glyphs, sorted by code point: none if the engine's
/// reader refused the file, and pages are then drawn in Bedstead.
static NOTO_GLYPHS: LazyLock<Vec<FontGlyph>> = LazyLock::new(|| {
    read(NOTO_TELETEXT_BDF).unwrap_or_else(|error| {
        eprintln!("neoteletext-decoder: the bundled Noto Teletext font was refused ({error:?}); pages are drawn in Bedstead");
        Vec::new()
    })
});

/// Bedstead with Noto Teletext as the user's font for every set it
/// conforms for (the engine's REQ-FNT-011): the font draws every character
/// of those sets it has, Bedstead the rest.
static NOTO_TELETEXT: LazyLock<GlyphSet<'static>> = LazyLock::new(|| match LoadedFont::new(&NOTO_GLYPHS) {
    Ok(font) => BEDSTEAD.with_fonts(None, by_set(font)),
    Err(error) => {
        eprintln!("neoteletext-decoder: the bundled Noto Teletext font is not a loaded font ({error:?}); pages are drawn in Bedstead");
        BEDSTEAD
    }
});

/// `font` in the place of every set of the engine's `FONT_SETS` it
/// conforms for, and no font in the others.
fn by_set(font: LoadedFont<'static>) -> [Option<LoadedFont<'static>>; FONT_SET_COUNT] {
    let mut fonts = [None; FONT_SET_COUNT];
    for (slot, (_, set)) in fonts.iter_mut().zip(FONT_SETS) {
        *slot = conformance(&font, set).conforms().then_some(font);
    }
    fonts
}

/// A BDF file's glyphs, as the engine's reader takes them: into a table of
/// the most characters a font may have, cut to those it has.
pub(crate) fn read(bytes: &[u8]) -> Result<Vec<FontGlyph>, FontError> {
    let mut table = vec![FontGlyph::EMPTY; FONT_MAX_CHARACTERS];
    let font = bdf::read(bytes, &mut table)?;
    table.truncate(font.characters);
    Ok(table)
}

/// The glyphs a page is drawn with in `font`.
pub(crate) fn glyphs(font: TeletextFont) -> &'static GlyphSet<'static> {
    match font {
        TeletextFont::Bedstead => &BEDSTEAD,
        TeletextFont::NotoTeletext => &NOTO_TELETEXT,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]

    use super::*;

    /// REQ-1584: Noto Teletext is the font of every one of the standard's
    /// 24 character sets (the first 24 of the engine's `FONT_SETS`), the
    /// sets the browser's decoder designates, as the engine's
    /// `third_party/noto/fonts.txt` says it conforms for each.
    #[test]
    fn noto_teletext_draws_every_standard_set() {
        /// The standard's sets, first in `FONT_SETS` (REQ-FNT-011).
        const STANDARD_SETS: usize = 24;
        let font = LoadedFont::new(&NOTO_GLYPHS).unwrap();
        let fonts = by_set(font);
        let missing: Vec<&str> = FONT_SETS[..STANDARD_SETS].iter().zip(&fonts).filter(|(_, slot)| slot.is_none()).map(|((name, _), _)| *name).collect();
        assert!(missing.is_empty(), "no Noto Teletext for {missing:?}");
    }
}
