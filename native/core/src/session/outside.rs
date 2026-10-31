//! Files and links that open from outside a page: dropped on a window
//! (REQ-1700), or named by the operating system (REQ-1703).

use interface_core::browser::DropPlace;

use super::Session;

impl Session {
    /// Opens `files` where they were dropped. The current tab is replaced
    /// only by a drop on it; else each file has a new tab, in the order
    /// dropped, the first active (DEC-1549 items 2 and 3). Returns why any
    /// could not be opened.
    pub fn open_dropped(&mut self, files: &[String], place: DropPlace) -> Vec<String> {
        let (at, replaced) = match place {
            DropPlace::NewTabs => (self.tabs.len(), None),
            DropPlace::Before(tab) => (usize::from(tab), None),
            DropPlace::After(tab) => (usize::from(tab).saturating_add(1), None),
            DropPlace::Replace(tab) => (usize::from(tab).saturating_add(1), Some(usize::from(tab))),
        };
        let mut at = at.min(self.tabs.len());
        let mut files = files.iter();
        let mut refused = Vec::new();
        let replaced = replaced.filter(|tab| *tab < self.tabs.len());
        if let (Some(tab), Some(file)) = (replaced, files.as_slice().first()) {
            self.active = tab;
            files.next();
            if let Err(reason) = self.navigate(file, false) {
                refused.push(reason);
            }
        }
        let first = at;
        for file in files {
            match self.open(file) {
                Ok(_) => {
                    self.move_tab(self.tabs.len().saturating_sub(1), at);
                    at = at.saturating_add(1);
                }
                Err(reason) => refused.push(reason),
            }
        }
        self.active = replaced.unwrap_or(first).min(self.tabs.len().saturating_sub(1));
        refused
    }

    /// The most recently active normal window: the one at hand unless it is
    /// private, else the latest set aside; none when every window is private
    /// (DEC-1549 item 8).
    #[must_use]
    pub fn normal_window(&self) -> Option<u32> {
        if !self.private_window {
            return Some(self.window);
        }
        self.others.iter().rev().find(|window| !window.private).map(|window| window.id)
    }

    /// Opens `addresses` as new tabs at the end of the window at hand, the
    /// first active and the rest behind it. On macOS a window showing only
    /// the start page gives way to them (DEC-1549 item 8). Returns why any
    /// could not be opened.
    pub fn open_at_end(&mut self, addresses: &[String]) -> Vec<String> {
        let lone_start_page =
            cfg!(target_os = "macos") && matches!(self.tabs.as_slice(), [tab] if tab.address().is_some_and(neoteletext::address::Address::is_start_page));
        let first = self.tabs.len();
        let mut refused = Vec::new();
        let mut opened = false;
        for address in addresses {
            match self.open(address) {
                Ok(_) => opened = true,
                Err(reason) => refused.push(reason),
            }
        }
        self.active = first.min(self.tabs.len().saturating_sub(1));
        if lone_start_page && opened {
            self.close(0);
            self.active = 0;
        }
        refused
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use neoteletext::bookmarks::Bookmarks;

    fn session(tabs: &[&str]) -> Session {
        let mut session = Session::new(Bookmarks::default());
        for tab in tabs {
            session.open(tab).unwrap();
        }
        session.active = 0;
        session
    }

    fn hosts(session: &Session) -> Vec<String> {
        session.tabs.iter().map(|tab| tab.address().unwrap().host.clone()).collect()
    }

    const FILES: [&str; 2] = ["one.invalid", "two.invalid"];

    fn dropped(session: &mut Session, place: DropPlace) -> Vec<String> {
        let files: Vec<String> = FILES.map(str::to_owned).to_vec();
        session.open_dropped(&files, place)
    }

    /// DEC-1549 item 2: on the page, the current tab stays, and the first
    /// file is in front.
    #[test]
    fn a_drop_on_the_page_opens_new_tabs_and_replaces_nothing() {
        let mut session = session(&["a.invalid", "b.invalid"]);
        assert_eq!(dropped(&mut session, DropPlace::NewTabs), Vec::<String>::new());
        assert_eq!(hosts(&session), ["a.invalid", "b.invalid", "one.invalid", "two.invalid"]);
        assert_eq!(session.active, 2);
    }

    /// DEC-1549 item 3: each place on the tab strip.
    #[test]
    fn a_drop_on_a_tab_goes_where_its_quarter_says() {
        let mut before = session(&["a.invalid", "b.invalid"]);
        dropped(&mut before, DropPlace::Before(1));
        assert_eq!((hosts(&before), before.active), (vec!["a.invalid".to_owned(), "one.invalid".into(), "two.invalid".into(), "b.invalid".into()], 1));

        let mut after = session(&["a.invalid", "b.invalid"]);
        dropped(&mut after, DropPlace::After(0));
        assert_eq!((hosts(&after), after.active), (vec!["a.invalid".to_owned(), "one.invalid".into(), "two.invalid".into(), "b.invalid".into()], 1));

        let mut replaced = session(&["a.invalid", "b.invalid"]);
        dropped(&mut replaced, DropPlace::Replace(0));
        assert_eq!((hosts(&replaced), replaced.active), (vec!["one.invalid".to_owned(), "two.invalid".into(), "b.invalid".into()], 0));
        assert_eq!(replaced.tabs[0].history.len(), 2, "the replaced tab keeps what it had to go back to");
    }

    /// A tab's place the interface made up changes nothing it should not.
    #[test]
    fn a_drop_past_the_last_tab_goes_at_the_end() {
        let mut session = session(&["a.invalid"]);
        dropped(&mut session, DropPlace::Replace(9));
        assert_eq!(hosts(&session), ["a.invalid", "one.invalid", "two.invalid"]);
        assert_eq!(session.active, 1);
    }

    #[test]
    fn what_cannot_be_opened_is_told_and_leaves_the_rest() {
        let mut session = session(&["a.invalid"]);
        let refused = session.open_dropped(&["ftp://x.invalid/".to_owned(), "two.invalid".to_owned()], DropPlace::NewTabs);
        assert_eq!(refused.len(), 1);
        assert_eq!(hosts(&session), ["a.invalid", "two.invalid"]);
    }

    /// DEC-1549 item 8: a new active tab at the end, the rest behind it.
    #[test]
    fn what_the_system_opens_goes_at_the_end_with_the_first_in_front() {
        let mut session = session(&["a.invalid", "b.invalid"]);
        assert_eq!(session.open_at_end(&FILES.map(str::to_owned)), Vec::<String>::new());
        assert_eq!(hosts(&session), ["a.invalid", "b.invalid", "one.invalid", "two.invalid"]);
        assert_eq!(session.active, 2);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_lone_start_page_gives_way() {
        let mut session = session(&[neoteletext::address::START_PAGE]);
        session.open_at_end(&["one.invalid".to_owned()]);
        assert_eq!(hosts(&session), ["one.invalid"]);
        assert_eq!(session.active, 0);
    }

    /// DEC-1549 item 8: never a private window.
    #[test]
    fn the_most_recent_normal_window_is_found() {
        let mut session = session(&["a.invalid"]);
        let normal = session.window;
        assert_eq!(session.normal_window(), Some(normal));
        session.new_window(true).unwrap();
        session.open("b.invalid").unwrap();
        assert_eq!(session.normal_window(), Some(normal), "past the private one at hand");
        let mut private = Session::new(Bookmarks::default());
        private.private_window = true;
        assert_eq!(private.normal_window(), None);
    }
}
