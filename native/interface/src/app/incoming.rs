//! What the trusted core said, given to the interface core of the window
//! at hand.

use std::collections::BTreeSet;

use interface_core::browser::TabInfo;
use interface_core::canvas::PageKey;
use winit::event_loop::ActiveEventLoop;

use crate::incoming::{Incoming, Library};

use super::App;

impl App {
    pub(super) fn incoming(&mut self, event_loop: &ActiveEventLoop, incoming: Incoming) {
        let now = self.now();
        match incoming {
            // Used when the window opens; it opens with the first message.
            // The windows' own messages are taken before they come here.
            Incoming::Window(_) | Incoming::Target(_) | Incoming::OpenWindow { .. } | Incoming::CloseWindow(_) => return,
            Incoming::Tabs(tabs, active) => self.tabs_changed(tabs, active),
            // The sound helper said which channels sound, through the
            // trusted core (decision 55).
            Incoming::Sounding(channels) => return self.sounding(channels),
            Incoming::Dock(choice) => return self.dock_chosen(event_loop, choice),
            Incoming::Menu(item) => {
                self.menu_chosen(event_loop, item);
                self.act(event_loop);
            }
            Incoming::Strip(strip) => self.view.browser.set_strip(strip),
            Incoming::Settings(view) => self.view.browser.set_settings(view),
            Incoming::Update(view) => {
                // The About panel shows the version the trusted core reports.
                dock_menu::set_about(&view.version, interface_core::COPYRIGHT);
                self.view.browser.set_update(view);
            }
            Incoming::Console(update) => self.view.browser.console_update(update),
            #[cfg(feature = "interaction")]
            Incoming::InputTrace(line) => self.view.browser.set_input_trace(line),
            Incoming::Library(view) => self.library(view, now),
            Incoming::Channel(channel) => self.view.browser.set_channel(channel),
            Incoming::Arrived { channel, page, title } => self.view.browser.add_page(channel, page, title, now),
            Incoming::SubPages { channel, page, subpages } => self.view.browser.set_subpages(channel, page, subpages),
            Incoming::Updated { channel, page } => self.page_updated(channel, page, now),
            Incoming::Page { key, version, image } => self.page_frame(event_loop, key, version, &image, now),
            #[cfg(feature = "phone-keyboard")]
            Incoming::Phone(view) => {
                self.show_phone(view);
                self.request_redraw();
            }
            #[cfg(feature = "phone-keyboard")]
            Incoming::PhoneKey(key) => {
                if self.phone_key(key) {
                    self.request_redraw();
                }
                self.act(event_loop);
            }
            #[cfg(feature = "interaction")]
            Incoming::Typing(typing) => {
                self.show_typing(&typing, now);
                self.act(event_loop);
            }
            #[cfg(feature = "links")]
            Incoming::Link(message) => self.link_message(event_loop, message, now),
            Incoming::End(reason) => {
                self.exit = reason;
                event_loop.exit();
                return;
            }
        }
        // Drawn with the next paced frame: pages load and flash many times
        // a second, and one frame shows all that arrived since the last.
        self.view.dirty = true;
    }

    /// The window's tabs, and which is active.
    fn tabs_changed(&mut self, tabs: Vec<TabInfo>, active: usize) {
        self.view.browser.set_tabs(tabs.clone(), active);
        self.view.tabs = tabs;
        self.view.active = active;
        self.show_tabs();
    }

    /// The channels that sound: the strip is drawn again only when that
    /// changed what it shows.
    fn sounding(&mut self, channels: BTreeSet<u32>) {
        if self.view.browser.set_sounding(channels) {
            self.show_tabs();
            self.request_redraw();
        }
    }

    /// One of the library's views, given to its panel.
    fn library(&mut self, view: Library, now: u64) {
        match view {
            Library::History(view) => self.view.browser.set_history(view),
            Library::Cookies(view) => self.view.browser.set_cookies(view),
            Library::PageControls(controls) => self.view.browser.set_page_controls(controls),
            Library::Remote(place) => self.view.browser.set_remote(place),
            Library::Suggestions(suggestions) => self.view.browser.set_suggestions(suggestions),
            Library::TabList(list) => self.view.browser.set_tab_list(list),
            Library::Bookmarks(view) => self.view.browser.set_bookmarks(view, now),
            Library::Star(view) => self.view.browser.set_star(view),
            Library::Bar(view) => self.view.browser.set_bar(view),
        }
    }

    /// A page of the channel was received again.
    fn page_updated(&mut self, channel: u32, page: u16, now: u64) {
        if self.trace_frames {
            eprintln!("neoteletext-interface: at {now} ms: page {page:03X} of channel {channel} updated");
        }
        self.view.browser.page_updated(channel, page, now);
    }

    /// A frame of a page shown.
    fn page_frame(&mut self, event_loop: &ActiveEventLoop, key: PageKey, version: u32, image: &[u8], now: u64) {
        if self.trace_frames {
            eprintln!("neoteletext-interface: at {now} ms: frame {version} of page {:03X} of channel {}", key.page, key.channel);
        }
        if self.show_frame(event_loop, key, version, image) {
            self.act(event_loop);
        }
    }

    /// What the trusted core decided of the open page's links.
    #[cfg(feature = "links")]
    fn link_message(&mut self, event_loop: &ActiveEventLoop, message: protocol::links::Message, now: u64) {
        use protocol::links::Message;
        match message {
            Message::Page { key, links } => self.view.browser.set_links(key, links),
            Message::Pointing(pointing) => self.view.browser.set_pointing(pointing),
            Message::Prompt(prompt) => self.view.browser.set_prompt(prompt, now),
            Message::Show(key) => self.view.browser.show(key, now),
            Message::Aliases { channel, numbers } => self.view.browser.set_aliases(channel, numbers),
        }
        self.act(event_loop);
    }
}
