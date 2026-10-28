//! The open page's controls (`page-controls.md`, decision 67): the keys of
//! the engine's own viewer while a page is open. The trusted core says
//! what the page has; every key goes to it as strip input. No control is
//! drawn over the page: its sub-pages are stepped with the arrows or
//! chosen in the sub-page list.

use super::{Browser, Key, Output};
use crate::controls::{FastextKey, PageControl, PageControls};

/// The open page's controls, as the trusted core sent them.
#[derive(Debug, Default)]
pub(super) struct ControlsUi {
    pub(super) controls: PageControls,
}

impl Browser {
    /// The open page's controls, as the trusted core sent them.
    pub fn set_page_controls(&mut self, controls: PageControls) {
        self.page_controls.controls = controls;
    }

    /// The open page's controls, for the remote control and the tests.
    #[must_use]
    pub fn page_controls(&self) -> PageControls {
        self.page_controls.controls
    }

    /// Asks for `control` on the open page.
    pub(super) fn ask_control(&mut self, control: PageControl) {
        self.outputs.push(Output::PageControl(control));
    }

    /// Whether the controls are those of the page open now.
    pub(super) fn controls_open(&self) -> bool {
        self.wall.zoomed().is_some_and(|key| self.page_controls.controls.page == Some(key.page))
    }

    /// A key while a page is open: the engine viewer's keys for its
    /// controls (the engine's user guide, "The viewer"); `None` when it is
    /// not one.
    pub(super) fn control_key(&mut self, key: Key<'_>) -> Option<bool> {
        if !self.controls_open() {
            return None;
        }
        let control = match key {
            Key::Text("r") => PageControl::Fastext(FastextKey::Red),
            Key::Text("g") => PageControl::Fastext(FastextKey::Green),
            Key::Text("y") => PageControl::Fastext(FastextKey::Yellow),
            Key::Text("c") => PageControl::Fastext(FastextKey::Cyan),
            Key::Text("i") => PageControl::Fastext(FastextKey::Index),
            Key::Text("?") => PageControl::Reveal,
            Key::Text("H") => PageControl::Hold,
            Key::Text("m") => PageControl::Mix,
            // On a page with sub-pages the arrows step through them, as in
            // the engine's viewer; on others they size the pages.
            Key::Left | Key::Right if self.page_controls.controls.subpages > 1 => PageControl::SubPage(key == Key::Right),
            _ => return None,
        };
        self.ask_control(control);
        Some(true)
    }

    /// A coloured key pressed on the keyboard's own key for it (F1 to F4),
    /// while a page is open; returns whether it was taken.
    pub fn fastext_key(&mut self, key: FastextKey) -> bool {
        if !self.controls_open() {
            return false;
        }
        self.ask_control(PageControl::Fastext(key));
        true
    }
}

#[cfg(test)]
mod tests;
