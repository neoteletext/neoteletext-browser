//! The fonts the viewer chose for pictures (renderer parity PAR-35): a ReGIS
//! font for alphabet 0 and a NAPLPS stroke font, each replacing the font
//! compiled in for every picture of its language, or going back to it. The
//! broker reads the chosen file within its language's bound; a decoder
//! started for it alone parses it (requirement 3.2), and only a font that
//! parsed is kept and given to every channel's decoder, now and as each
//! starts. A font that does not parse is refused with the parser's
//! message, and the font in force is kept. The trusted core never parses
//! a font. The provided sets are not replaceable (DEC-6669).
//!
//! A chosen font lasts for the run: it is not written to the settings
//! file (the browser's DEC-6802 reading, recorded with the to-do that
//! would keep it).

use std::sync::Arc;

use interface_core::canvas::PictureLanguage;
use protocol::console::Level;
use protocol::decoder::ToDecoder;
use protocol::regis::font_parts;

use super::Core;
use super::processes::start_font_checker;

/// A font the viewer chose, and its file's name for the panel.
#[derive(Clone)]
struct Chosen {
    name: String,
    bytes: Arc<Vec<u8>>,
}

/// What the trusted core holds of the fonts for pictures.
#[derive(Default)]
pub(super) struct PictureFonts {
    /// The ReGIS font in force, or none for the one compiled in.
    regis: Option<Chosen>,
    /// The NAPLPS stroke font in force, likewise.
    naplps: Option<Chosen>,
    /// The font being checked, of which check, while its decoder parses it.
    checking: Option<(u64, PictureLanguage, Chosen)>,
    /// Counts checks, so an older answer never replaces a newer.
    generation: u64,
}

impl PictureFonts {
    fn in_force(&mut self, language: PictureLanguage) -> &mut Option<Chosen> {
        match language {
            PictureLanguage::Regis => &mut self.regis,
            PictureLanguage::Naplps => &mut self.naplps,
        }
    }

    /// The file name of each language's font in force, empty for the one
    /// compiled in: ReGIS, then NAPLPS.
    pub(super) fn names(&self) -> (String, String) {
        let name = |chosen: &Option<Chosen>| chosen.as_ref().map(|chosen| chosen.name.clone()).unwrap_or_default();
        (name(&self.regis), name(&self.naplps))
    }

    /// What a decoder is told to have the fonts in force: nothing for a
    /// language whose font is the one compiled in, which a decoder begins
    /// with.
    pub(super) fn messages(&self) -> Vec<ToDecoder> {
        [(PictureLanguage::Regis, &self.regis), (PictureLanguage::Naplps, &self.naplps)]
            .into_iter()
            .filter_map(|(language, chosen)| chosen.as_ref().map(|chosen| (language, chosen)))
            .flat_map(|(language, chosen)| font_parts(language, &chosen.bytes))
            .map(ToDecoder::PictureFont)
            .collect()
    }
}

/// The language a font dialog's kind chooses for.
pub(super) const fn language_of(naplps: bool) -> PictureLanguage {
    if naplps { PictureLanguage::Naplps } else { PictureLanguage::Regis }
}

/// The language's name, for the panel's words.
const fn language_name(language: PictureLanguage) -> &'static str {
    match language {
        PictureLanguage::Regis => "ReGIS",
        PictureLanguage::Naplps => "NAPLPS",
    }
}

impl Core {
    /// The font the user chose in the font dialog for `language`: the
    /// broker reads it within its bound, and a decoder checks it.
    pub(super) fn font_chosen(&mut self, language: PictureLanguage, path: &str) -> Result<(), String> {
        let path = std::path::Path::new(path.trim());
        let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let bytes = match self.broker().and_then(|broker| broker.read_font(path)) {
            Ok(bytes) => Arc::new(bytes),
            Err(reason) => {
                self.log(Level::Warning, None, &format!("a {} font was not read: {reason}", language_name(language)));
                self.refusal = Some(format!("That {} font was not read: {reason}", language_name(language)));
                return self.send_settings();
            }
        };
        self.fonts.generation = self.fonts.generation.wrapping_add(1);
        let chosen = Chosen { name, bytes: Arc::clone(&bytes) };
        self.fonts.checking = Some((self.fonts.generation, language, chosen));
        let (generation, events) = (self.fonts.generation, self.events.clone());
        if let Err(reason) = self.broker().and_then(|broker| start_font_checker(broker, language, bytes, generation, events)) {
            self.fonts.checking = None;
            self.refusal = Some(format!("That {} font was not checked: {reason}", language_name(language)));
            return self.send_settings();
        }
        Ok(())
    }

    /// The decoder that checked a font answered: a font that parsed is in
    /// force for every channel's decoder, now and as each starts; one that
    /// did not is refused with the parser's message, cleaned as the panel
    /// shows it, and the font in force kept.
    pub(super) fn font_checked(&mut self, generation: u64, refusal: Result<(), String>) -> Result<(), String> {
        let Some((checked, language, chosen)) = self.fonts.checking.take() else { return Ok(()) };
        if checked != generation {
            self.fonts.checking = Some((checked, language, chosen));
            return Ok(());
        }
        match refusal {
            Ok(()) => {
                let parts = font_parts(language, &chosen.bytes);
                *self.fonts.in_force(language) = Some(chosen);
                self.refusal = None;
                self.tell_every_decoder(&parts);
            }
            Err(reason) => self.refusal = Some(format!("That {} font was refused: {reason}", language_name(language))),
        }
        self.send_settings()
    }

    /// The panel's "Built in": pictures of `language` are drawn in the font
    /// compiled in again.
    pub(super) fn font_built_in(&mut self, language: PictureLanguage) {
        // A check under way would put the font back: it is forgotten.
        if self.fonts.checking.as_ref().is_some_and(|(_, checking, _)| *checking == language) {
            self.fonts.checking = None;
        }
        if self.fonts.in_force(language).take().is_some() {
            self.tell_every_decoder(&font_parts(language, &[]));
        }
    }

    /// A channel's decoder took the font it was given, which a checker had
    /// already parsed; one that refused it keeps the font in force, which
    /// is only logged, its words cleaned.
    pub(super) fn font_taken(&mut self, channel: u32, refusal: Option<&str>) {
        if let Some(reason) = refusal {
            let text = format!("the channel's decoder refused a font for pictures: {}", protocol::clean(reason, interface_core::picture::MAX_STATUS));
            self.log(Level::Warning, Some(channel), &text);
        }
    }

    /// Gives every channel's decoder a font's parts.
    fn tell_every_decoder(&mut self, parts: &[protocol::regis::FontPart]) {
        let channels: Vec<u32> = self.decoders.keys().copied().collect();
        for channel in channels {
            for part in parts {
                self.tell(channel, ToDecoder::PictureFont(part.clone()));
            }
        }
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::super::test_support::{core_watched, sent_to_interface};
    use super::*;
    use protocol::ToInterface;

    fn last_settings(sent: &[ToInterface]) -> interface_core::settings::SettingsView {
        sent.iter().rev().find_map(|message| if let ToInterface::Settings(view) = message { Some(view.clone()) } else { None }).unwrap()
    }

    /// PAR-35: a font that parsed is in force, named in the panel, and
    /// given to a decoder as it starts; one refused keeps the font in force
    /// and the panel says the parser's message; "Built in" goes back.
    #[test]
    fn a_checked_font_is_in_force_and_a_refused_one_changes_nothing() {
        let (mut core, _broker, written) = core_watched("a.invalid");
        let font = Chosen { name: "mine.regisfont".into(), bytes: Arc::new(b"[strike 20]\n".to_vec()) };
        core.fonts.checking = Some((1, PictureLanguage::Regis, font));
        core.font_checked(1, Ok(())).unwrap();
        assert_eq!(last_settings(&sent_to_interface(&mut core, &written)).regis_font, "mine.regisfont");
        assert!(matches!(core.fonts.messages().as_slice(), [ToDecoder::PictureFont(part)] if part.language == PictureLanguage::Regis && part.total == 12));

        let bad = Chosen { name: "bad.regisfont".into(), bytes: Arc::new(b"41 FF\n".to_vec()) };
        core.fonts.checking = Some((2, PictureLanguage::Regis, bad));
        core.font_checked(2, Err("line 1 of the font is malformed\u{1b}[2J".into())).unwrap();
        let view = last_settings(&sent_to_interface(&mut core, &written));
        assert_eq!(view.regis_font, "mine.regisfont", "the font in force is kept");
        let refusal = view.refusal.unwrap();
        assert!(refusal.starts_with("That ReGIS font was refused: line 1") && !refusal.contains('\u{1b}'), "{refusal}");

        core.font_built_in(PictureLanguage::Regis);
        assert!(core.fonts.messages().is_empty(), "the font compiled in again");
        assert_eq!(core.fonts.names(), (String::new(), String::new()));
    }

    /// An answer to an older check changes nothing.
    #[test]
    fn an_older_check_is_ignored() {
        let (mut core, _broker, _written) = core_watched("a.invalid");
        let font = Chosen { name: "late.strokes".into(), bytes: Arc::new(b"[primary]\n".to_vec()) };
        core.fonts.checking = Some((5, PictureLanguage::Naplps, font));
        core.font_checked(4, Ok(())).unwrap();
        assert!(core.fonts.naplps.is_none() && core.fonts.checking.is_some());
    }
}
