//! The fonts the viewer chose for pictures (renderer parity PAR-35): the
//! trusted core reads a font file within its language's bound and gives its
//! bytes here in parts; the confined decoder puts them together and parses
//! them with the renderer's own bounded parser, never the trusted core. A
//! font that parses replaces the one compiled in for every new terminal of
//! its language; one that does not is refused with the parser's message
//! and the font in force kept. The provided sets are not replaceable
//! (DEC-6669): only alphabet 0 of ReGIS and the NAPLPS stroke font are.

use interface_core::canvas::PictureLanguage;
use protocol::regis::{FontPart, MAX_NAPLPS_FONT_BYTES, MAX_REGIS_FONT_BYTES, max_font_bytes};

const _: () = assert!(MAX_REGIS_FONT_BYTES == regis::limits::MAX_FONT_BYTES, "the trusted core reads what the ReGIS renderer parses");
const _: () = assert!(MAX_NAPLPS_FONT_BYTES == naplps::limits::MAX_STROKE_FONT_BYTES, "the trusted core reads what the NAPLPS renderer parses");

/// A font for one language's pictures, parsed.
#[derive(Clone, Debug)]
pub enum ChosenFont {
    /// Alphabet 0 of every ReGIS terminal.
    Regis(Box<regis::Font>),
    /// The stroke font of every NAPLPS terminal.
    Naplps(naplps::StrokeFont),
}

/// What a whole font came to: its language, and the font chosen, `None`
/// for the font compiled in; or the parser's refusal.
pub type Taken = (PictureLanguage, Result<Option<ChosenFont>, String>);

/// The parts of a font being received, one font at a time.
#[derive(Default)]
pub struct FontParts {
    /// The language, the whole font's length, and the bytes so far.
    receiving: Option<(PictureLanguage, usize, Vec<u8>)>,
}

impl FontParts {
    /// Takes one part. Returns the font when it is whole: parsed, or the
    /// parser's refusal; nothing while parts are owed. A part that does not
    /// follow the one before starts again from it if it is a first part,
    /// else it is dropped with what came before, which the protocol's
    /// reader made impossible for a trusted core that sends in order.
    pub fn take(&mut self, part: &FontPart) -> Option<Taken> {
        let total = usize::try_from(part.total).unwrap_or(usize::MAX);
        if total == 0 {
            self.receiving = None;
            return Some((part.language, Ok(None)));
        }
        if part.offset == 0 {
            self.receiving = Some((part.language, total, Vec::with_capacity(total.min(max_font_bytes(part.language)))));
        }
        let follows = self.receiving.as_ref().is_some_and(|(language, length, bytes)| {
            *language == part.language && *length == total && usize::try_from(part.offset).is_ok_and(|offset| offset == bytes.len())
        });
        if !follows {
            self.receiving = None;
            return None;
        }
        let (language, length, mut bytes) = self.receiving.take()?;
        bytes.extend_from_slice(&part.bytes);
        if bytes.len() < length {
            self.receiving = Some((language, length, bytes));
            return None;
        }
        Some((language, parse(language, &bytes).map(Some)))
    }
}

/// Puts a whole font in force in `engine`'s stores, or keeps the one in
/// force when it was refused; what the trusted core is told.
pub fn apply(engine: &mut crate::engine::Engine, (language, taken): Taken) -> protocol::decoder::FromDecoder {
    let refusal = match taken {
        Ok(Some(ChosenFont::Regis(font))) => {
            engine.regis().set_font(Some(*font));
            None
        }
        Ok(Some(ChosenFont::Naplps(font))) => {
            engine.naplps().set_font(Some(font));
            None
        }
        Ok(None) => {
            match language {
                PictureLanguage::Regis => engine.regis().set_font(None),
                PictureLanguage::Naplps => engine.naplps().set_font(None),
            }
            None
        }
        Err(reason) => Some(reason),
    };
    protocol::decoder::FromDecoder::FontChecked { language, refusal }
}

/// Parses a font file for `language` with its renderer's parser, whose
/// message is the refusal.
///
/// # Errors
/// The text is not UTF-8, or the parser refused it.
pub fn parse(language: PictureLanguage, bytes: &[u8]) -> Result<ChosenFont, String> {
    let text = core::str::from_utf8(bytes).map_err(|_| String::from("the font file is not UTF-8 text"))?;
    match language {
        PictureLanguage::Regis => regis::Font::parse(text).map(|font| ChosenFont::Regis(Box::new(font))).map_err(|error| error.to_string()),
        PictureLanguage::Naplps => naplps::StrokeFont::parse(text).map(ChosenFont::Naplps).map_err(|error| error.to_string()),
    }
}

/// A ReGIS font of one glyph, A, every row of it lit: unlike any glyph of
/// the font compiled in.
#[cfg(test)]
pub(crate) const BLOCK_A: &str = "[strike 20]\n41 FF FF FF FF FF FF FF FF FF FF FF FF FF FF FF FF FF FF FF FF\n";

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, reason = "a test")]
mod tests {
    use super::*;

    /// PAR-35: a font in parts is parsed when whole; a malformed one is
    /// refused with the parser's message, naming its line; a total of 0
    /// goes back to the font compiled in.
    #[test]
    fn a_font_is_parsed_when_whole_and_refused_with_the_parsers_message() {
        let mut parts = FontParts::default();
        let font = BLOCK_A.as_bytes();
        let (first, second) = font.split_at(10);
        let total = u32::try_from(font.len()).unwrap();
        assert!(parts.take(&FontPart { language: PictureLanguage::Regis, total, offset: 0, bytes: first.to_vec() }).is_none(), "owed");
        let taken = parts.take(&FontPart { language: PictureLanguage::Regis, total, offset: 10, bytes: second.to_vec() });
        assert!(matches!(taken, Some((PictureLanguage::Regis, Ok(Some(ChosenFont::Regis(_)))))), "{taken:?}");
        let bad = b"[strike 20]\n41 FF\n";
        let refused = parts.take(&FontPart { language: PictureLanguage::Regis, total: u32::try_from(bad.len()).unwrap(), offset: 0, bytes: bad.to_vec() });
        let Some((_, Err(message))) = refused else { panic!("refused: {refused:?}") };
        assert!(message.contains('2'), "the parser names line 2: {message}");
        let naplps = b"[primary]\n21 0 6 3,8,3,4\n";
        let taken = parts.take(&FontPart { language: PictureLanguage::Naplps, total: u32::try_from(naplps.len()).unwrap(), offset: 0, bytes: naplps.to_vec() });
        assert!(matches!(taken, Some((PictureLanguage::Naplps, Ok(Some(ChosenFont::Naplps(_)))))), "{taken:?}");
        assert!(matches!(parts.take(&FontPart { language: PictureLanguage::Naplps, total: 0, offset: 0, bytes: Vec::new() }), Some((_, Ok(None)))));
        assert!(parse(PictureLanguage::Naplps, &[0xFF, 0xFE]).is_err(), "not text");
    }
}
