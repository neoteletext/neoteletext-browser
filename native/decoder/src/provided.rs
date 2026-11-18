//! The engine's 55 provided character sets (the owner's DEC-6669: a
//! picture's text is drawn only in them) as each picture renderer takes
//! them, with the joining data of the plain Arabic-script sets, 44 to 51
//! (neoteletext-common's `docs/renderer-parity.md` PAR-3, PAR-13, PAR-14
//! and PAR-36 item 4).
//!
//! The glyphs are the Noto Teletext glyphs the decoder has compiled in,
//! the teletext renderer's own, so that a code of a set draws the same in
//! all three renderers. Each list is made once and copied into each
//! terminal.

use std::sync::OnceLock;

use naplps::{BitmapSet, JoinForm};
use neoteletext_core::charset::CharRef;
use neoteletext_core::consts::{CELL_HEIGHT, CELL_WIDTH};
use neoteletext_core::glyphs::conformance::{FONT_SET_COUNT, FONT_SETS};
use neoteletext_core::glyphs::{JOINED_SETS, Joining, JoiningClass, joining_in};
use regis::{JoinedForm, ProvidedSet};

use crate::engine::GLYPHS;

/// The first and last codes of a set: 2/0 and 7/15.
pub(crate) const FIRST_SET_CODE: u8 = 0x20;
/// See [`FIRST_SET_CODE`].
pub(crate) const LAST_SET_CODE: u8 = 0x7F;

// REQ-6668 and PAR-12: the engine's sets are each renderer's, one for one,
// in one order, and every position of a set is a code from 2/0 to 7/15.
const _: () = assert!(FONT_SET_COUNT == naplps::TELETEXT_SET_COUNT);
const _: () = assert!(FONT_SET_COUNT == regis::limits::PROVIDED_SETS as usize);
const _: () = assert!((LAST_SET_CODE - FIRST_SET_CODE + 1) as usize == naplps::bitmap::BITMAP_SET_CODES);
const _: () = assert!((LAST_SET_CODE - FIRST_SET_CODE + 1) as usize == regis::limits::PROVIDED_SET_POSITIONS);

/// The joining data of code `code` of set `set`, when the set is one of
/// the plain Arabic-script sets (PAR-3).
fn joining_of(set: u8, code: u8) -> Option<Joining> {
    JOINED_SETS.contains(&set).then(|| joining_in(&GLYPHS, set, code)).flatten()
}

/// The engine's sets as the NAPLPS renderer takes them (REQ-6668): for
/// each, in the order of the engine's `FONT_SETS`, which is the renderer's
/// `TELETEXT_SETS`, the glyph the teletext renderer draws for each code,
/// and for sets 44 to 51 each code's joining type and its final, initial
/// and medial forms (PAR-14).
pub(crate) fn naplps_sets() -> &'static [BitmapSet] {
    static SETS: OnceLock<Vec<BitmapSet>> = OnceLock::new();
    SETS.get_or_init(|| {
        let (Ok(width), Ok(height)) = (u8::try_from(CELL_WIDTH), u8::try_from(CELL_HEIGHT)) else { return Vec::new() };
        (0_u8..)
            .zip(FONT_SETS.iter())
            .filter_map(|(number, (_, set))| {
                let mut bitmap = BitmapSet::new(width, height).ok()?;
                for code in FIRST_SET_CODE..=LAST_SET_CODE {
                    // A glyph of the engine's own cell always fits; one that
                    // did not would leave its code blank.
                    let _ = bitmap.set_glyph(code, &GLYPHS.glyph(CharRef::new(*set, code)));
                    let Some(joining) = joining_of(number, code) else { continue };
                    let _ = bitmap.set_joining(code, naplps_joining(joining.class));
                    for (form, rows) in [(JoinForm::Final, &joining.final_form), (JoinForm::Initial, &joining.initial), (JoinForm::Medial, &joining.medial)] {
                        let _ = bitmap.set_form_glyph(code, form, rows);
                    }
                }
                Some(bitmap)
            })
            .collect()
    })
}

/// The engine's sets as the ReGIS renderer takes them, alphabets 100 to
/// 154 (PAR-12), with the joining data of sets 44 to 51 (PAR-13).
pub(crate) fn regis_sets() -> &'static [ProvidedSet] {
    static SETS: OnceLock<Vec<ProvidedSet>> = OnceLock::new();
    SETS.get_or_init(|| {
        let (Ok(width), Ok(height)) = (u8::try_from(CELL_WIDTH), u8::try_from(CELL_HEIGHT)) else { return Vec::new() };
        (0_u8..)
            .zip(FONT_SETS.iter())
            .filter_map(|(number, (_, set))| {
                let mut provided = ProvidedSet::new(width, height).ok()?;
                for code in FIRST_SET_CODE..=LAST_SET_CODE {
                    let _ = provided.set_glyph(code, &GLYPHS.glyph(CharRef::new(*set, code)));
                    let Some(joining) = joining_of(number, code) else { continue };
                    let _ = provided.set_joining(code, regis_joining(joining.class));
                    for (form, rows) in
                        [(JoinedForm::Final, &joining.final_form), (JoinedForm::Initial, &joining.initial), (JoinedForm::Medial, &joining.medial)]
                    {
                        let _ = provided.set_form(code, form, rows);
                    }
                }
                Some(provided)
            })
            .collect()
    })
}

/// The engine's joining class as the NAPLPS renderer names it.
const fn naplps_joining(class: JoiningClass) -> naplps::JoiningType {
    match class {
        JoiningClass::None => naplps::JoiningType::None,
        JoiningClass::Right => naplps::JoiningType::Right,
        JoiningClass::Dual => naplps::JoiningType::Dual,
        JoiningClass::Transparent => naplps::JoiningType::Transparent,
    }
}

/// The engine's joining class as the ReGIS renderer names it.
const fn regis_joining(class: JoiningClass) -> regis::JoiningType {
    match class {
        JoiningClass::None => regis::JoiningType::None,
        JoiningClass::Right => regis::JoiningType::Right,
        JoiningClass::Dual => regis::JoiningType::Dual,
        JoiningClass::Transparent => regis::JoiningType::Transparent,
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test reports a failure by panicking, and indexes the 55 sets it made")]

    use super::*;

    /// PAR-13, PAR-14: the plain Arabic-script sets carry their joining
    /// data in both renderers' form, and no other set is joined.
    #[test]
    fn the_plain_arabic_script_sets_are_joined_in_both_renderers_and_no_other() {
        assert_eq!((naplps_sets().len(), regis_sets().len()), (FONT_SET_COUNT, FONT_SET_COUNT));
        for number in 0..FONT_SET_COUNT {
            let joined = JOINED_SETS.contains(&u8::try_from(number).unwrap());
            assert_eq!(naplps_sets()[number].is_joined(), joined, "NAPLPS set {number}");
            assert_eq!(regis_sets()[number].is_joined(), joined, "ReGIS set {number}");
        }
        // Set 51 is G0-Arabic-Plain: some code of it is dual-joining, with
        // a medial form the engine gives, in both renderers.
        let (set, number) = (51_u8, 51_usize);
        let code = (FIRST_SET_CODE..=LAST_SET_CODE).find(|&code| joining_of(set, code).is_some_and(|joining| joining.class == JoiningClass::Dual)).unwrap();
        let joining = joining_of(set, code).unwrap();
        assert_eq!(naplps_sets()[number].joining(code), naplps::JoiningType::Dual);
        assert_eq!(regis_sets()[number].joining(code), Some(regis::JoiningType::Dual));
        assert_eq!(naplps_sets()[number].form_glyph(code, JoinForm::Medial).unwrap(), joining.medial.as_slice());
        assert_eq!(regis_sets()[number].form(code, JoinedForm::Medial).unwrap(), joining.medial.as_slice());
    }
}
