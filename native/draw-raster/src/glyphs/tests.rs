//! The glyph store's tests (TODO-7197): its bounds hold however the faces
//! are used.
#![allow(clippy::arithmetic_side_effects, clippy::unwrap_used, reason = "a test's own small counts and known fonts")]

use std::sync::Arc;

use parley::FontData;
use parley::fontique::Blob;

use super::{FACE_CACHE_LIMIT, GLYPH_CACHE_LIMIT, Glyphs};
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

#[test]
fn text_in_more_faces_than_the_limit_makes_again_only_the_faces_past_it() {
    // A title of one character from each of 68 scripts is 68 faces (each
    // script's fallback font at the title's size), drawn on every frame;
    // one font at 68 sizes is 68 faces in the same way.
    let font = roboto();
    let faces = FACE_CACHE_LIMIT + 4;
    let overflow = faces - FACE_CACHE_LIMIT;
    let mut glyphs = Glyphs::default();
    let draw = |glyphs: &mut Glyphs| {
        glyphs.next_frame();
        for face in 0..faces {
            run(glyphs, &font, 10.0 + f32::from(u8::try_from(face).unwrap()), 36..40);
        }
    };
    draw(&mut glyphs);
    assert_eq!(glyphs.made, faces);
    for frame in 1..6 {
        let made = glyphs.made;
        draw(&mut glyphs);
        let again = glyphs.made - made;
        assert!(again <= overflow, "frame {frame} made {again} of {faces} faces again, more than the {overflow} past the limit");
        let (held, kept) = glyphs.held();
        assert!(kept <= FACE_CACHE_LIMIT && held <= GLYPH_CACHE_LIMIT);
    }
}

#[test]
fn faces_not_drawn_lately_make_room_before_those_on_screen() {
    // A page in other faces, then text in the faces of the page before:
    // a face not used for two frames is dropped first.
    let font = roboto();
    let mut glyphs = Glyphs::default();
    glyphs.next_frame();
    for face in 0..FACE_CACHE_LIMIT {
        run(&mut glyphs, &font, 10.0 + f32::from(u8::try_from(face).unwrap()), 36..37);
    }
    glyphs.next_frame();
    glyphs.next_frame();
    // The first face is drawn again; a new one then drops the second,
    // the least recently used of those not drawn lately.
    run(&mut glyphs, &font, 10.0, 36..37);
    let made = glyphs.made;
    run(&mut glyphs, &font, 200.0, 36..37);
    run(&mut glyphs, &font, 10.0, 36..37);
    run(&mut glyphs, &font, 12.0, 36..37);
    assert_eq!(glyphs.made, made + 1, "the face drawn this frame and the stale faces after the oldest were kept");
    run(&mut glyphs, &font, 11.0, 36..37);
    assert_eq!(glyphs.made, made + 2, "the least recently used face was the one dropped");
}
