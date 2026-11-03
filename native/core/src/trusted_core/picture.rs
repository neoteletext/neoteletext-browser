//! The Tools menu's picture converter (`docs/picture-tools.md`): the trusted
//! core keeps the chosen picture's bytes and the panel's choices, has a
//! decoder started for it alone convert the picture (requirement 3.2), shows
//! the page made at `neoteletext:picture`, and has the broker save it where
//! the user chose. It never parses the picture or the page.

use std::sync::Arc;

use confine::broker::{ChooseKind, MAX_SAVE};
use interface_core::picture::{MAX_STATUS, PictureChoice, PictureStatus};
use protocol::ToInterface;
use protocol::console::Level;
use protocol::picture::MAX_PICTURE_PAGE;

use super::Core;
use super::processes::start_picture_converter;

const _: () = assert!(MAX_PICTURE_PAGE <= MAX_SAVE, "the broker saves any page a decoder may make");

/// The page made opens at its page number: the engine's converter writes
/// page 100 (its `PAGE_NUMBER`), which a test holds this to.
const PICTURE_ADDRESS: &str = "neoteletext:picture#100";

/// What the trusted core holds of the picture converter.
#[derive(Default)]
pub(super) struct PictureState {
    /// The chosen picture's bytes, read by the broker, never parsed here.
    picture: Option<Arc<Vec<u8>>>,
    /// The chosen picture's file name, for the panel.
    name: String,
    choice: PictureChoice,
    /// The page made: TTI and T42, from a confined decoder.
    page: Option<(Vec<u8>, Vec<u8>)>,
    /// Counts conversions, so an older answer never replaces a newer.
    generation: u64,
}

impl Core {
    /// The panel's choices: the picture is converted again with them.
    pub(super) fn picture_choice(&mut self, choice: PictureChoice) -> Result<(), String> {
        // The protocol refused anything outside the lists already.
        if !choice.is_valid() || choice == self.picture.choice {
            return Ok(());
        }
        self.picture.choice = choice;
        self.convert_picture()
    }

    /// The picture the user chose in the picture dialog: the broker reads
    /// it, as a picture only, and a decoder converts it.
    pub(super) fn picture_chosen(&mut self, path: &str) -> Result<(), String> {
        let path = std::path::Path::new(path.trim());
        let read = self.broker().and_then(|broker| broker.read_picture(path));
        self.picture.name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        match read {
            Ok(bytes) => {
                self.picture.picture = Some(Arc::new(bytes));
                self.picture.page = None;
                self.convert_picture()
            }
            Err(reason) => {
                self.log(Level::Warning, None, &format!("a picture was not read: {reason}"));
                self.picture_status(false, "That file is not a picture this converter reads: PNG, JPEG, GIF, BMP or WebP.")
            }
        }
    }

    /// Converts the chosen picture with the panel's choices, in a decoder
    /// started for it; the answer arrives as an event.
    fn convert_picture(&mut self) -> Result<(), String> {
        let Some(picture) = self.picture.picture.clone() else {
            return self.picture_status(false, "Choose a picture to convert.");
        };
        self.picture.generation = self.picture.generation.wrapping_add(1);
        let (generation, choice, events) = (self.picture.generation, self.picture.choice, self.events.clone());
        match self.broker().and_then(|broker| start_picture_converter(broker, picture, choice, generation, events)) {
            Ok(()) => {
                let text = format!("Converting {}\u{2026}", self.picture.name);
                self.picture_status(self.picture.page.is_some(), &text)
            }
            Err(reason) => self.picture_status(false, &format!("Not converted: {reason}")),
        }
    }

    /// A picture decoder's answer: the page made is shown, or why none was.
    /// The decoder's words are untrusted and cleaned.
    pub(super) fn picture_converted(&mut self, generation: u64, result: Result<(Vec<u8>, Vec<u8>, String), String>) -> Result<(), String> {
        if generation != self.picture.generation {
            return Ok(());
        }
        match result {
            Ok((tti, t42, summary)) => {
                self.session.picture_page.clone_from(&tti);
                self.picture.page = Some((tti, t42));
                // The converted page opens in the active tab, or opens again
                // with the new page when it is shown already.
                if let Err(reason) = self.session.navigate(PICTURE_ADDRESS, false) {
                    self.log(Level::Warning, None, &reason);
                }
                self.send_all()?;
                let text = format!("{}: {}", self.picture.name, summary);
                self.picture_status(true, &text)
            }
            Err(reason) => {
                self.picture.page = None;
                self.picture_status(false, &format!("Not converted: {reason}"))
            }
        }
    }

    /// The user chose where to save the page: the broker writes it there.
    pub(super) fn picture_save(&mut self, kind: ChooseKind, path: &str) -> Result<(), String> {
        let Some((tti, t42)) = &self.picture.page else {
            return self.picture_status(false, "There is no converted page to save.");
        };
        let bytes = if kind == ChooseKind::SaveT42 { t42.clone() } else { tti.clone() };
        let name = std::path::Path::new(path).file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
        let form = if kind == ChooseKind::SaveT42 { "T42" } else { "TTI" };
        match self.broker().and_then(|broker| broker.save(kind, &bytes)) {
            Ok(()) => self.picture_status(true, &format!("Saved as {form}: {name}")),
            Err(reason) => self.picture_status(true, &format!("Not saved: {reason}")),
        }
    }

    /// Tells the panel, in one cleaned sentence.
    fn picture_status(&mut self, converted: bool, text: &str) -> Result<(), String> {
        let text = protocol::clean(text, MAX_STATUS);
        self.send(&ToInterface::PictureStatus(PictureStatus { converted, text }))
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used, reason = "a test")]
mod tests {
    use super::super::test_support::{core_watched, sent_to_interface};
    use super::*;

    const PAGE: &[u8] = b"PN,10000\nOL,1,Hello\n";

    fn statuses(sent: &[ToInterface]) -> Vec<PictureStatus> {
        sent.iter().filter_map(|message| if let ToInterface::PictureStatus(status) = message { Some(status.clone()) } else { None }).collect()
    }

    /// The page a picture decoder made opens at `neoteletext:picture`, its
    /// bytes the channel's, and the panel may save it.
    #[test]
    fn a_converted_page_opens_at_its_built_in_address() {
        let (mut core, _broker, written) = core_watched("a.invalid");
        core.picture.generation = 1;
        core.picture_converted(1, Ok((PAGE.to_vec(), vec![0; 42], "8 colours".into()))).unwrap();
        assert_eq!(core.session.active_tab().unwrap().address().unwrap().text, PICTURE_ADDRESS);
        assert_eq!(core.session.picture_page, PAGE);
        let said = statuses(&sent_to_interface(&mut core, &written));
        assert!(said.last().is_some_and(|status| status.converted), "{said:?}");
    }

    /// An answer to an older conversion changes nothing; a decoder's words
    /// reach the panel cleaned.
    #[test]
    fn an_older_answer_is_ignored_and_words_are_cleaned() {
        let (mut core, _broker, written) = core_watched("a.invalid");
        core.picture.generation = 2;
        core.picture_converted(1, Ok((PAGE.to_vec(), Vec::new(), String::new()))).unwrap();
        assert_eq!(core.session.picture_page, Vec::<u8>::new());
        core.picture_converted(2, Err("bad\u{1b}[2Jpicture\u{202e}".into())).unwrap();
        let said = statuses(&sent_to_interface(&mut core, &written));
        let last = said.last().unwrap();
        assert!(!last.converted);
        assert!(!last.text.contains('\u{1b}') && !last.text.contains('\u{202e}'), "{last:?}");
        assert_eq!(core.session.active_tab().unwrap().address().unwrap().text, "https://a.invalid/");
    }

    /// Nothing is saved before a page is made.
    #[test]
    fn nothing_is_saved_before_a_page_is_made() {
        let (mut core, _broker, written) = core_watched("a.invalid");
        core.picture_save(ChooseKind::SaveTti, "/home/a/page.tti").unwrap();
        let said = statuses(&sent_to_interface(&mut core, &written));
        assert!(said.last().is_some_and(|status| !status.converted && status.text.contains("no converted page")), "{said:?}");
    }

    /// The converted page's address is built in, kept out of history, and
    /// no other `neoteletext:` address opens.
    #[test]
    fn the_converted_pages_address_is_built_in() {
        let address = neoteletext::address::parse(PICTURE_ADDRESS).unwrap();
        assert!(address.is_picture() && address.is_built_in() && !address.is_start_page());
        assert_eq!(address.page, Some(0x100));
        assert!(neoteletext::history::recordable(PICTURE_ADDRESS).is_none());
        assert!(neoteletext::address::parse("neoteletext:other").is_err());
    }
}
