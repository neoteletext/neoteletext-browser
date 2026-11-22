//! What the trusted core said, given to the interface core of the window
//! at hand.

use std::collections::BTreeSet;

use interface_core::browser::TabInfo;
use interface_core::canvas::PageKey;
use winit::event_loop::ActiveEventLoop;

use crate::incoming::{Incoming, Library};

use super::App;

/// The page a thumbnail is drawn through to make its still: of channel 0,
/// which the trusted core never numbers a channel (its numbers start at 1
/// and skip 0 when they wrap).
const THUMBNAIL_PAGE: PageKey = PageKey { channel: 0, page: 0 };

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
            Incoming::OpenedBySystem(opened) => return self.opened_by_system(event_loop, opened),
            Incoming::Menu(item) => {
                self.menu_chosen(event_loop, item);
                self.act(event_loop);
            }
            Incoming::Strip(strip) => self.strip_changed(strip),
            Incoming::Settings(view) => {
                dock_menu::set_aspect(view.aspect);
                self.view.painter.set_regis_contain(view.regis_contain);
                self.view.browser.set_settings(view);
            }
            Incoming::PictureStatus(status) => self.view.browser.set_picture_status(status),
            Incoming::Update(view) => {
                // The About panel shows the version the trusted core reports.
                dock_menu::set_about(&view.version, interface_core::COPYRIGHT);
                self.view.browser.set_update(view);
            }
            Incoming::Console(update) => self.view.browser.console_update(update),
            Incoming::Recording(view) => self.view.browser.set_recording(view),
            #[cfg(feature = "interaction")]
            Incoming::InputTrace(line) => self.view.browser.set_input_trace(line),
            Incoming::Library(view) => self.library(view, now),
            Incoming::Channel(channel) => {
                self.view.browser.set_channel(channel);
                self.view.browser.show_listed(now);
            }
            Incoming::Arrived { channel, page, title } => self.view.browser.add_page(channel, page, title, now),
            Incoming::SubPageCount { channel, page, count } => self.view.browser.set_subpage_count(channel, page, count),
            Incoming::SubPageRows { channel, page, request, rows } => self.view.browser.set_subpage_rows(channel, page, request, rows.first, rows.rows),
            Incoming::Updated { channel, page } => self.page_updated(channel, page, now),
            Incoming::Show(key) => {
                self.view.browser.show(key, now);
                self.act(event_loop);
            }
            Incoming::Raise => {
                if let Some(graphics) = &self.view.graphics {
                    graphics.window.focus_window();
                }
            }
            Incoming::ShowWall(channel) => {
                self.view.browser.show_wall(channel, now);
                self.act(event_loop);
            }
            Incoming::ShowContents(channel) => {
                self.view.browser.show_contents(channel, now);
                self.act(event_loop);
            }
            Incoming::Page { key, version, image } => self.page_frame(event_loop, key, version, &image, now),
            Incoming::Thumbnail { channel, page, sub, image } => self.thumbnail(channel, page, sub, &image),
            Incoming::Regis { key, picture } => {
                if !self.regis_picture(event_loop, key, &picture, now) {
                    return;
                }
            }
            Incoming::RegisOver { key, over } => {
                self.view.browser.set_regis_over(key, over);
                // The pointer's shape follows at the next frame.
                self.request_redraw();
            }
            // The keyboard reaches a picture's targets with the links'
            // focus ring, which hyperlinks bring (PAR-30).
            #[cfg(feature = "links")]
            Incoming::PictureTargets { key, targets } => {
                self.view.browser.set_picture_targets(key, targets);
                self.request_redraw();
            }
            #[cfg(not(feature = "links"))]
            Incoming::PictureTargets { .. } => {}
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
        self.view.pace.changed(now);
    }

    /// What the trusted strip shows. REQ-7164 item 1 and REQ-1686 item 5:
    /// the strip that clears the page area of the channel left, for the
    /// words of a change of channel or for a redirect's notice, is drawn as
    /// it arrives. A paced frame would leave that channel's page drawn for
    /// up to `CHANGE_FRAME_MILLISECONDS` after the last frame that changed
    /// the window, which the key or click that left the channel, or a page
    /// that animates, has just drawn (TODO-7206, TODO-7222). The strip that
    /// ends a redirect's notice is drawn as it arrives too, or the same
    /// pace would lengthen the notice's one second (REQ-1686, TODO-7232).
    fn strip_changed(&mut self, strip: interface_core::browser::StripState) {
        let at_once = self.view.browser.drawn_as_it_arrives(&strip);
        self.view.browser.set_strip(strip);
        if at_once {
            self.view.pace.awaited();
            self.request_redraw();
        }
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
            Library::Known(view) => self.view.browser.set_known(view),
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

    /// The picture of a page drawn in ReGIS arrived: held by the painter,
    /// which finds what it changed, and drawn. False when it was no
    /// picture, which ends the interface.
    fn regis_picture(&mut self, event_loop: &ActiveEventLoop, key: PageKey, picture: &protocol::regis::RegisPicture, now: u64) -> bool {
        if self.trace_frames {
            eprintln!("neoteletext-interface: at {now} ms: picture {} of page {:03X} of channel {}", picture.commits, key.page, key.channel);
        }
        let area = picture.area;
        // RULE-66's hops: when the window's thread took the repaint.
        #[cfg(feature = "timed-test")]
        protocol::timed_test::mark("interface", "regis-taken", std::time::Instant::now(), format_args!("{},{},{},{}", area.x, area.y, area.width, area.height));
        let part = draw_raster::RegisPart {
            language: picture.language,
            number: picture.number,
            base: picture.base,
            x: area.x,
            y: area.y,
            width: area.width,
            height: area.height,
        };
        if let Err(error) = self.view.painter.set_regis(key, &picture.palette, part, &picture.packed) {
            self.exit = Some(format!("bad ReGIS picture from the trusted core: {error:?}"));
            event_loop.exit();
            return false;
        }
        self.view.browser.set_regis(key, picture.commits, picture.work, picture.language);
        // The picture of the page the viewer types into or points at is
        // drawn as it arrives, with whatever else waited (REQ-6400).
        if self.awaited(key) {
            self.view.pace.awaited();
        }
        self.request_redraw();
        true
    }

    /// A picture of a tab that has no still (REQ-6883): made a still of the
    /// tab, at the still's size, through a page of no channel's that is
    /// dropped at once, and the image dropped; nothing of it is kept but the
    /// still. One for a tab gone or moved, or that has a still as seen this
    /// run, is dropped whole; one the painter refuses keeps the card.
    fn thumbnail(&mut self, channel: u32, page: u16, sub: u16, image: &protocol::ThumbnailImage) {
        use interface_core::canvas::Canvas as _;
        if !self.view.browser.thumbnail_wanted(channel, page, sub) {
            return;
        }
        let painter = &mut self.view.painter;
        let drawn = match image {
            protocol::ThumbnailImage::Frame(bytes) => {
                protocol::frame::decode(bytes, None).is_ok_and(|frame| painter.set_page(THUMBNAIL_PAGE, &frame.palette, frame.screen, &frame.indices).is_ok())
            }
            protocol::ThumbnailImage::Regis(picture) => {
                let part = draw_raster::RegisPart {
                    language: picture.language,
                    number: picture.number,
                    base: picture.base,
                    x: picture.area.x,
                    y: picture.area.y,
                    width: picture.area.width,
                    height: picture.area.height,
                };
                // The picture's page beneath it: a black page, as a ReGIS
                // page's teletext page shows none of its own.
                let blank = vec![0; protocol::frame::PAGE_DOTS];
                painter.set_page(THUMBNAIL_PAGE, &[[0, 0, 0]], 0, &blank).is_ok()
                    && painter.set_regis(THUMBNAIL_PAGE, &picture.palette, part, &picture.packed).is_ok()
            }
        };
        let kept = drawn && painter.keep_still(channel, THUMBNAIL_PAGE);
        painter.retain_pages(|key| *key != THUMBNAIL_PAGE);
        let regis = matches!(image, protocol::ThumbnailImage::Regis(picture) if picture.language == interface_core::canvas::PictureLanguage::Regis);
        self.view.browser.thumbnail_kept(channel, page, regis, kept);
        self.request_redraw();
    }

    /// A frame of a page shown.
    fn page_frame(&mut self, event_loop: &ActiveEventLoop, key: PageKey, version: u32, image: &[u8], now: u64) {
        if self.trace_frames {
            eprintln!("neoteletext-interface: at {now} ms: frame {version} of page {:03X} of channel {}", key.page, key.channel);
        }
        if self.show_frame(event_loop, key, version, image) {
            self.act(event_loop);
            // The page the viewer types into or points at is drawn as its
            // frame arrives, however soon after the last (REQ-RCH-001).
            if self.awaited(key) {
                self.view.pace.awaited();
                self.request_redraw();
            }
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
            Message::Aliases { channel, numbers } => self.view.browser.set_aliases(channel, numbers),
        }
        self.act(event_loop);
    }
}
