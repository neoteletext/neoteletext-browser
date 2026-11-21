//! The glyph store's tests (TODO-7197): its bounds hold however the faces
//! are used.
#![allow(clippy::arithmetic_side_effects, clippy::unwrap_used, reason = "a test's own small counts and known fonts")]

use std::sync::Arc;

use parley::FontData;
use parley::fontique::Blob;

use super::{GLYPH_CACHE_LIMIT, Glyphs};
use crate::ROBOTO;

fn roboto() -> FontData {
    FontData::new(Blob::new(Arc::new(ROBOTO)), 0)
}

/// Draws glyphs `ids` in `font` at `size`, as one run.
fn run(glyphs: &mut Glyphs, font: &FontData, size: f32, ids: impl Iterator<Item = u32>) {
    let mut face = glyphs.face(font, size, &[]).unwrap();
    for id in ids {
        face.glyph(id);
    }
}

#[test]
fn one_face_used_for_ever_new_glyphs_holds_no_more_than_the_limit() {
    // As a channel's titles of ever-new ideographs in one fallback font at
    // one size would: one face, held throughout, asked for new glyphs run
    // after run. Glyph numbers past the font's own are kept as empty
    // outlines and counted alike.
    let font = roboto();
    let mut glyphs = Glyphs::default();
    let run_length = 60;
    let runs = 2 * GLYPH_CACHE_LIMIT / run_length;
    let mut most = 0;
    for number in 0..runs {
        let first = u32::try_from(number * run_length).unwrap();
        run(&mut glyphs, &font, 14.0, first..first + u32::try_from(run_length).unwrap());
        let (held, faces) = glyphs.held();
        assert_eq!(faces, 1);
        assert_eq!(held, glyphs.kept, "the count kept is the glyphs held");
        most = most.max(held);
    }
    assert!(most <= GLYPH_CACHE_LIMIT, "held {most} glyphs in one face, past GLYPH_CACHE_LIMIT ({GLYPH_CACHE_LIMIT})");
    assert!(most > GLYPH_CACHE_LIMIT - run_length, "the store fills to its limit before it is emptied: held at most {most}");
}
