//! The fonts a ReGIS receiver draws a script page's text with, built as
//! the browser's decoder builds them (`native/decoder/src/provided.rs` and
//! `native/decoder/src/regis.rs`): the renderer has no font of its own and
//! is given them as data. A script page's text is drawn in the engine's 55
//! provided character sets, alphabets 100 to 154, whose glyphs are the
//! Noto Teletext glyphs the engine has compiled in, with the joining data
//! of the plain Arabic-script sets; alphabet 0, which only shading with a
//! character uses, is the VT340 font of the crate `regis-fonts`.

use neoteletext_core::charset::CharRef;
use neoteletext_core::consts::{CELL_HEIGHT, CELL_WIDTH};
use neoteletext_core::glyphs::conformance::FONT_SETS;
use neoteletext_core::glyphs::{GlyphSet, JOINED_SETS, JoiningClass, joining_in};
use regis::{Font, JoinedForm, JoiningType, ProvidedSet, Terminal};

/// The first and last codes of a set: 2/0 and 7/15.
const FIRST_SET_CODE: u8 = 0x20;
const LAST_SET_CODE: u8 = 0x7F;

/// Gives `terminal` the fonts the decoder gives a page's terminal.
pub fn give(terminal: &mut Terminal) {
    static GLYPHS: GlyphSet<'static> = GlyphSet::noto_teletext();
    terminal.set_font(Font::parse(regis_fonts::VT340).expect("the compiled font parses"));
    let (width, height) = (u8::try_from(CELL_WIDTH).unwrap(), u8::try_from(CELL_HEIGHT).unwrap());
    for (number, (_, set)) in (0_u8..).zip(FONT_SETS.iter()) {
        let mut provided = ProvidedSet::new(width, height).unwrap();
        for code in FIRST_SET_CODE..=LAST_SET_CODE {
            let _ = provided.set_glyph(code, &GLYPHS.glyph(CharRef::new(*set, code)));
            let Some(joining) = JOINED_SETS.contains(&number).then(|| joining_in(&GLYPHS, number, code)).flatten() else { continue };
            let class = match joining.class {
                JoiningClass::None => JoiningType::None,
                JoiningClass::Right => JoiningType::Right,
                JoiningClass::Dual => JoiningType::Dual,
                JoiningClass::Transparent => JoiningType::Transparent,
            };
            let _ = provided.set_joining(code, class);
            for (form, rows) in [(JoinedForm::Final, &joining.final_form), (JoinedForm::Initial, &joining.initial), (JoinedForm::Medial, &joining.medial)] {
                let _ = provided.set_form(code, form, rows);
            }
        }
        terminal.set_provided_set(number, provided).unwrap();
    }
}
