//! What opens from outside a page (DEC-1549, DEC-1551, REQ-1703): the file
//! dialog's choice, files dropped on a window, and what the operating system
//! or a second start names. The broker has already decided what may be read;
//! this decides where it opens.

use std::fs::File;
use std::sync::Arc;

use confine::broker::{Broker, ChooseKind, Opening, read_opening};
use interface_core::browser::{DropPlace, OpenKind};
use neoteletext::address::{is_outside_link, outside_link, typed_path};
use protocol::MAX_FILES_AT_ONCE;
use protocol::ToInterface;
use protocol::console::Level;

use super::{Core, Event};

const _: () = assert!(MAX_FILES_AT_ONCE == confine::broker::MAX_FILES_AT_ONCE, "the protocol's bound and the broker's are one bound");

impl Core {
    /// Asks the broker for its pipe of what opens from outside, and hears it
    /// on a thread of its own, as events.
    pub(super) fn hear_openings(&mut self, broker: &Arc<Broker>) {
        match broker.openings() {
            Ok(pipe) => {
                let events = self.events.clone();
                std::thread::spawn(move || hear_openings(pipe, &events));
            }
            Err(error) => self.log(Level::Warning, None, &format!("nothing can open from outside: {error}")),
        }
    }

    /// The file dialog is asked for (REQ-1699). A private tab's choice is for
    /// the run alone.
    pub(super) fn choose(&mut self, kind: OpenKind, parent: &str) -> Result<(), String> {
        let keep = self.session.active_tab().is_some_and(|tab| !tab.private);
        let kind = match kind {
            OpenKind::File => ChooseKind::File,
            OpenKind::Folder => ChooseKind::Folder,
            OpenKind::Bookmarks => ChooseKind::Bookmarks,
        };
        match self.broker().and_then(|broker| broker.choose(kind, parent, keep)) {
            Ok(()) => Ok(()),
            Err(reason) => self.say(reason),
        }
    }

    /// What arrived on the broker's pipe.
    pub(super) fn opening(&mut self, opening: Opening) -> Result<(), String> {
        match opening {
            Opening::Chosen { kind: ChooseKind::Bookmarks, path } => self.import_chosen(&path)?,
            Opening::Chosen { path, .. } => {
                if let Err(reason) = self.session.navigate(&path, true) {
                    self.log(Level::Warning, self.log_place(), &reason);
                }
                self.send_all()?;
            }
            Opening::Outside { links, files } => self.open_outside(&links, &files)?,
        }
        self.record_drawn()?;
        self.keep_session();
        Ok(())
    }

    /// Files dropped on the window at hand (REQ-1700).
    pub(super) fn dropped(&mut self, files: &[String], place: DropPlace, left_out: u16) -> Result<(), String> {
        for reason in self.session.open_dropped(files, place) {
            self.log(Level::Warning, self.log_place(), &reason);
        }
        self.send_all()?;
        if left_out > 0 {
            self.say(format!("{left_out} not opened: one drop opens at most {MAX_FILES_AT_ONCE} files"))?;
        }
        Ok(())
    }

    /// What macOS asked the application to open: a claim the interface
    /// passed on, whose files it declared, whose links are checked here.
    pub(super) fn opened_by_system(&mut self, opened: &[String]) -> Result<(), String> {
        let (links, files): (Vec<String>, Vec<String>) = opened.iter().cloned().partition(|text| is_outside_link(text));
        let files: Vec<String> = files.iter().filter_map(|file| typed_path(file, None)).collect();
        self.open_outside(&links, &files)
    }

    /// `links` and `files` open in the most recently active normal window,
    /// or a new one, at the end of its tabs (DEC-1549 item 8, REQ-1672 item
    /// 7). Links beyond the rate limit are dropped, not queued, and a
    /// refusal is logged by its reason, never with the link (item 12).
    fn open_outside(&mut self, links: &[String], files: &[String]) -> Result<(), String> {
        let mut addresses = Vec::new();
        for link in links {
            if !self.outside_links.admit(std::time::Instant::now()) {
                self.log(Level::Warning, None, "a link from outside was dropped: too many at once");
                continue;
            }
            match outside_link(link) {
                Ok(address) => addresses.push(address.text),
                Err(reason) => self.log(Level::Warning, None, &format!("a link from outside was refused: {reason}")),
            }
        }
        addresses.extend(files.iter().cloned());
        if addresses.is_empty() {
            return Ok(());
        }
        match self.session.normal_window() {
            Some(window) => {
                self.switch_window(window);
                for reason in self.session.open_at_end(&addresses) {
                    self.log(Level::Warning, None, &reason);
                }
                self.send_all()?;
                self.send(&ToInterface::Raise)
            }
            None => self.open_window_at(&addresses),
        }
    }
}

/// What the broker's pipe says, until it closes.
fn hear_openings(mut pipe: File, events: &std::sync::mpsc::Sender<Event>) {
    while let Ok(opening) = read_opening(&mut pipe) {
        if events.send(Event::Opening(opening)).is_err() {
            return;
        }
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::super::test_support::{core_on, core_watched, sent_to_interface};
    use super::*;
    use neoteletext::launch::MAX_OUTSIDE_LINKS;

    const LINK: &str = "neoteletext://https://news.invalid/302";

    fn open_links(core: &mut Core, links: &[&str]) {
        core.opening(Opening::Outside { links: links.iter().map(|link| (*link).to_owned()).collect(), files: Vec::new() }).unwrap();
    }

    fn addresses(core: &Core) -> Vec<String> {
        core.session.tabs.iter().map(|tab| tab.address().unwrap().text.clone()).collect()
    }

    /// REQ-1672 items 6 and 7: at once, in a new active tab at the end.
    #[test]
    fn a_link_opens_at_once_in_a_new_active_tab() {
        let (mut core, _broker) = core_on("a.invalid");
        open_links(&mut core, &[LINK]);
        assert_eq!(addresses(&core), ["https://a.invalid/", "https://news.invalid/302"]);
        assert_eq!(core.session.active, 1);
        assert!(core.session.tabs.iter().all(|tab| !tab.private));
    }

    /// REQ-1703 item 9: what opens from outside comes to the front.
    #[test]
    fn what_opens_from_outside_raises_the_window() {
        let (mut core, _broker, written) = core_watched("a.invalid");
        open_links(&mut core, &[LINK]);
        assert!(sent_to_interface(&mut core, &written).contains(&ToInterface::Raise));
    }

    /// REQ-1672 items 2 and 12: what a second start sends is checked again;
    /// a refused link opens nothing.
    #[test]
    fn a_hostile_link_opens_nothing() {
        let (mut core, _broker) = core_on("a.invalid");
        open_links(&mut core, &["neoteletext://file:///etc/passwd", "https://a.invalid/", "neoteletext://home"]);
        assert_eq!(addresses(&core), ["https://a.invalid/"]);
    }

    /// REQ-1672 item 11: a burst opens only what the limit allows.
    #[test]
    fn a_burst_of_links_opens_only_what_the_limit_allows() {
        let (mut core, _broker) = core_on("a.invalid");
        for _ in 0..3 * MAX_OUTSIDE_LINKS {
            open_links(&mut core, &[LINK]);
        }
        assert_eq!(core.session.tabs.len(), 1 + MAX_OUTSIDE_LINKS);
    }

    /// REQ-1672 item 7: never a private window; a new normal one instead.
    #[test]
    fn with_only_a_private_window_a_normal_one_opens() {
        let (mut core, _broker) = core_on("a.invalid");
        core.session.private_window = true;
        open_links(&mut core, &[LINK]);
        assert_eq!(core.session.window_count(), 2);
        assert!(!core.session.private_window, "the new window is at hand, and normal");
        assert_eq!(addresses(&core), ["https://news.invalid/302"]);
        assert!(!core.session.tabs.iter().any(|tab| tab.private));
    }

    /// DEC-1549 item 8: what macOS names opens as a link or a file does; the
    /// interface's word goes through the same checks.
    #[test]
    fn what_macos_names_goes_the_same_way() {
        let (mut core, _broker) = core_on("a.invalid");
        core.opened_by_system(&[LINK.to_owned(), "file:///no/such/page.tti".to_owned(), "neoteletext://file:///etc/passwd".to_owned()]).unwrap();
        assert_eq!(addresses(&core), ["https://a.invalid/", "https://news.invalid/302", "file:///no/such/page.tti"]);
        assert_eq!(core.session.active, 1);
    }

    /// DEC-1549 item 1: a choice opens in the current tab, a visit like any
    /// other.
    #[test]
    fn a_choice_opens_in_the_current_tab() {
        let (mut core, _broker) = core_on("a.invalid");
        core.opening(Opening::Chosen { kind: ChooseKind::File, path: "/no/such/page.tti".into() }).unwrap();
        assert_eq!(addresses(&core), ["file:///no/such/page.tti"]);
        assert_eq!(core.session.tabs[0].history.len(), 2);
    }

    /// DEC-1549 item 9: the rest of a drop is refused with a notice.
    #[test]
    fn what_a_drop_leaves_out_is_said() {
        let (mut core, _broker) = core_on("a.invalid");
        core.dropped(&["b.invalid".to_owned()], DropPlace::NewTabs, 3).unwrap();
        assert_eq!(addresses(&core), ["https://a.invalid/", "https://b.invalid/"]);
        assert!(core.notice.as_ref().is_some_and(|(text, _)| text.starts_with("3 not opened")));
    }
}
