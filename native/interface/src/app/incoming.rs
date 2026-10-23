//! What the trusted core said, given to the interface core of the window
//! at hand.

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
            Incoming::Tabs(tabs, active) => {
                self.view.browser.set_tabs(tabs.clone(), active);
                self.view.tabs = tabs;
                self.view.active = active;
                self.show_tabs();
            }
            // The sound helper said which channels sound, through the
            // trusted core (decision 55).
            Incoming::Sounding(channels) => {
                if self.view.browser.set_sounding(channels) {
                    self.show_tabs();
                    self.request_redraw();
                }
                return;
            }
            Incoming::Dock(choice) => {
                self.dock_chosen(event_loop, choice);
                return;
            }
            Incoming::Menu(item) => {
                self.menu_chosen(event_loop, item);
                self.act(event_loop);
            }
            Incoming::Strip(strip) => self.view.browser.set_strip(strip),
            Incoming::Settings(view) => self.view.browser.set_settings(view),
            Incoming::Update(view) => self.view.browser.set_update(view),
            Incoming::Console(update) => self.view.browser.console_update(update),
            Incoming::Library(view) => match view {
                Library::History(view) => self.view.browser.set_history(view),
                Library::Cookies(view) => self.view.browser.set_cookies(view),
                Library::PageControls(controls) => self.view.browser.set_page_controls(controls),
                Library::Remote(place) => self.view.browser.set_remote(place),
                Library::Suggestions(suggestions) => self.view.browser.set_suggestions(suggestions),
                Library::TabList(list) => self.view.browser.set_tab_list(list),
                Library::Bookmarks(view) => self.view.browser.set_bookmarks(view, now),
                Library::Star(view) => self.view.browser.set_star(view),
                Library::Bar(view) => self.view.browser.set_bar(view),
            },
            Incoming::Channel(channel) => self.view.browser.set_channel(channel),
            Incoming::Arrived { channel, page, title } => self.view.browser.add_page(channel, page, title, now),
            Incoming::Updated { channel, page } => {
                if self.trace_frames {
                    eprintln!("neoteletext-interface: at {now} ms: page {page:03X} of channel {channel} updated");
                }
                self.view.browser.page_updated(channel, page, now);
            }
            Incoming::Page { key, version, image } => {
                if self.trace_frames {
                    eprintln!("neoteletext-interface: at {now} ms: frame {version} of page {:03X} of channel {}", key.page, key.channel);
                }
                if self.show_frame(event_loop, key, version, &image) {
                    self.act(event_loop);
                }
            }
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
            Incoming::Link(message) => {
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
}
