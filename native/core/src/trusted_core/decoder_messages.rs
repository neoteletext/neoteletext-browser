//! What a decoder says, untrusted (`architecture.md` section 6): pages
//! arrived and changed, frames checked and sent, links, sub-pages, typing
//! and sound.

use interface_core::canvas::PageKey;
use protocol::console::{ConsoleTab, Level};
use protocol::decoder::FromDecoder;
use protocol::{ToInterface, clean};

use super::Core;

impl Core {
    pub(super) fn decoded(&mut self, channel: u32, message: FromDecoder) -> Result<(), String> {
        #[cfg(feature = "timed-test")]
        timed_marks(&message);
        let active = self.active_channel();
        match message {
            // Its first message: an end from now on is not a failure to
            // start (finding F-12). Only the channel's current decoder's
            // messages reach here.
            FromDecoder::Confined => {
                if let Some(decoder) = self.decoders.get_mut(&channel) {
                    decoder.hear();
                }
            }
            // Published in the channel's Log tab (the engine's REQ-AUD-017).
            FromDecoder::AudioReceiver(receiver) => self.note(Level::Info, Some(channel), &neoteletext::console::audio_receiver_line(&receiver)),
            FromDecoder::StoreFull => self.grow_decoder(channel)?,
            // The open page's rule table (page-rules.md RULE-52).
            message @ (FromDecoder::RulesOpen(_)
            | FromDecoder::Report(_)
            | FromDecoder::PageState { .. }
            | FromDecoder::RulesFault { .. }
            | FromDecoder::RuleCounters(_)) => self.rules_decoded(channel, message)?,
            #[cfg(feature = "interaction")]
            FromDecoder::Missing(pages) => self.wall_missing(channel, pages)?,
            #[cfg(not(feature = "interaction"))]
            FromDecoder::Missing(_) => {}
            // Only a decoder started to read bookmarks reports them; from a
            // channel's they change nothing.
            // The channel's own log, for its author: in its Log tab only,
            // cleaned and marked as the channel's (decision 70).
            FromDecoder::ChannelLog { level, page, text } => self.channel_log(channel, level, page, &text),
            FromDecoder::Bookmark(_) | FromDecoder::BookmarksRead { .. } => {
                self.log(Level::Warning, Some(channel), "the channel's decoder reported bookmarks, which it may not; ignored");
            }
            // A channel's decoder took a font for pictures, or refused it (PAR-35).
            FromDecoder::FontChecked { refusal, .. } => self.font_taken(channel, refusal.as_deref()),
            // Only a decoder started to convert a picture makes one.
            FromDecoder::PictureConverted { .. } | FromDecoder::PictureRefused(_) | FromDecoder::PictureOutput { .. } => {
                self.log(Level::Warning, Some(channel), "the channel's decoder reported a converted picture, which it may not; ignored");
            }
            FromDecoder::Arrived { page, title } => self.arrived(channel, page, &title)?,
            FromDecoder::PictureTitle { page, title } => self.picture_titled(channel, page, &title)?,
            FromDecoder::Updated { page } => {
                let at = self.now();
                self.console.page(channel, false);
                self.console.add(channel, ConsoleTab::Pages, at, &format!("page {page:03X} changed"));
                if active == Some(channel) {
                    self.send(&ToInterface::Updated { channel, page })?;
                }
            }
            FromDecoder::Frame { page, palette, screen, indices } => {
                let key = PageKey { channel, page };
                self.decoded.insert(key, super::DecodedFrame { palette: palette.clone(), screen, dots: indices.clone() });
                self.frame(key, palette, screen, indices)?;
            }
            FromDecoder::FrameArea { page, palette, screen, area, indices } => self.frame_area(PageKey { channel, page }, palette, screen, area, &indices)?,
            // A page drawn in ReGIS (DEC-6335): its pick record stays here,
            // for PICK, and its picture goes to the interface, which shows
            // it in the page's place.
            FromDecoder::Regis { page, sub, picture, pick } => {
                let key = PageKey { channel, page };
                self.regis.hold(key, sub, picture.commits, picture.language, pick);
                self.send(&ToInterface::Regis { key, picture })?;
                // What lies under a still pointer may have changed (REQ-6374 item 5).
                self.regis_repoint()?;
            }
            // The drawings of a ReGIS sub-page that are links (REQ-6376):
            // decided as every link is, and the pointer's place looked at again.
            #[cfg(feature = "links")]
            FromDecoder::RegisLinks { page, sub, links } => {
                let (key, now) = (PageKey { channel, page }, self.now());
                self.with_channel(channel, |table, link_channel| table.found_drawings(key, sub, &links, link_channel, now));
                self.regis_repoint()?;
            }
            #[cfg(feature = "links")]
            FromDecoder::Links { page, with_image, candidates, numbers, controls, fastext } => {
                let key = PageKey { channel, page };
                self.links.found(key, candidates, numbers, controls, fastext);
                if !with_image {
                    self.redecide(key)?;
                    self.send_controls()?;
                }
            }
            FromDecoder::SubPages { page, step } => self.subpages(PageKey { channel, page }, step)?,
            FromDecoder::SubPageCount { page, count } => {
                let changed = self.channel_subpages(channel).and_then(|subpages| subpages.stored(page, count));
                if let Some(count) = changed
                    && active == Some(channel)
                {
                    self.send(&ToInterface::SubPageCount { channel, page, count })?;
                }
            }
            // The rows the sub-page list asked for, from what the decoder
            // holds, in answer to the request passed to it.
            FromDecoder::SubPageRows { page, rows } => self.rows_from_decoder(PageKey { channel, page }, rows)?,
            #[cfg(feature = "interaction")]
            FromDecoder::Cursor { body } => {
                let sends = self.typing.cursor(channel, body);
                self.deliver_typing(sends)?;
            }
            #[cfg(feature = "interaction")]
            FromDecoder::InputControl { body } => {
                let sends = self.typing.control(channel, &body, self.now_us());
                self.deliver_typing(sends)?;
            }
            FromDecoder::Audio(audio) => {
                let now_us = self.now_us();
                if let Some(message) = self.hearing.decoded(channel, audio, now_us) {
                    self.tell_sound(&message);
                }
                // What the budget of sound commands dropped is told as
                // fault 10 for the open page (RULE-52 item 5).
                let dropped = self.hearing.take_commands_dropped(channel);
                if dropped != 0 {
                    self.sound_commands_dropped(channel, dropped)?;
                }
            }
        }
        Ok(())
    }

    /// A line of the channel's log, at `level`, of `page` if it names one.
    pub(super) fn channel_log(&mut self, channel: u32, level: Level, page: Option<u16>, text: &str) {
        let text = clean(text, protocol::decoder::MAX_CHANNEL_LOG);
        let line = match page {
            Some(page) => format!("channel, page {page:03X}: {text}"),
            None => format!("channel: {text}"),
        };
        let at = self.now();
        self.console.log(Some(channel), level, at, &line);
    }

    /// A page of `channel` arrived, with its title: said in the console,
    /// and to the interface when the channel is the active one.
    fn arrived(&mut self, channel: u32, page: u16, title: &str) -> Result<(), String> {
        let active = self.active_channel();
        let Some(tab) = self.session.tabs.iter_mut().find(|tab| tab.channel.id == channel) else { return Ok(()) };
        let title = tab.channel.arrived(page, title);
        if let Some(title) = &title {
            let at = self.now();
            self.console.page(channel, true);
            self.console.add(channel, ConsoleTab::Pages, at, &format!("page {page:03X} arrived: {title}"));
        }
        if let Some(title) = title
            && active == Some(channel)
        {
            self.send(&ToInterface::Page { channel, page, title })?;
            // A number on a page shown becomes a link once its page exists.
            #[cfg(feature = "links")]
            self.redecide_channel(channel)?;
        }
        Ok(())
    }

    /// A page that has arrived has a picture's title (DEC-6638 item 3,
    /// TODO-6748): it names the page in the page list in place of a title
    /// from its rows, never in place of the program's index's.
    fn picture_titled(&mut self, channel: u32, page: u16, title: &str) -> Result<(), String> {
        let active = self.active_channel();
        let Some(tab) = self.session.tabs.iter_mut().find(|tab| tab.channel.id == channel) else { return Ok(()) };
        let Some(title) = tab.channel.picture_titled(page, title) else { return Ok(()) };
        let at = self.now();
        self.console.add(channel, ConsoleTab::Pages, at, &format!("page {page:03X} is titled: {title}"));
        if active == Some(channel) {
            self.send(&ToInterface::Page { channel, page, title })?;
        }
        Ok(())
    }

    /// A page's image: sent when it is new and its page's interval is up.
    fn frame(&mut self, key: PageKey, palette: Vec<[u8; 3]>, screen: u8, indices: Vec<u8>) -> Result<(), String> {
        match self.frames.accept(key, palette, screen, indices, std::time::Instant::now()) {
            Some((version, image)) => self.send_frame(key, version, image)?,
            // The same image again, or one held until its page's
            // interval is up: only its links may have changed.
            #[cfg(feature = "links")]
            None => self.redecide(key)?,
            #[cfg(not(feature = "links"))]
            None => {}
        }
        // Its Fastext keys may lead elsewhere now.
        self.send_controls()
    }

    /// ANIM-5: the rectangle of a page that changed, laid over the frame
    /// the decoder last sent of it; one with no such frame, or in other
    /// colours, is the decoder's fault and is dropped.
    fn frame_area(&mut self, key: PageKey, palette: Vec<[u8; 3]>, screen: u8, area: protocol::decoder::PageArea, indices: &[u8]) -> Result<(), String> {
        let Some(held) = self.decoded.get_mut(&key).filter(|held| held.palette == palette && held.screen == screen) else {
            self.note(Level::Warning, Some(key.channel), &format!("page {:03X}: a changed rectangle of no frame held, dropped", key.page));
            return Ok(());
        };
        lay_area(&mut held.dots, area, indices);
        let whole = held.dots.clone();
        self.frame(key, palette, screen, whole)
    }

    /// Sends a page's frame to the interface (`architecture.md` section 10).
    pub(super) fn send_frame(&mut self, key: PageKey, version: u32, image: Vec<u8>) -> Result<(), String> {
        // The image's links first, so the interface has them when the
        // image arrives (`hyperlinks.md` section 8.1).
        #[cfg(feature = "links")]
        self.links_of_image(key, version)?;
        self.send(&ToInterface::Frame { key, version, image })?;
        // The page drew: its tab's visit is recorded (requirement 20.1).
        self.record_pending(key.channel)
    }
}

/// Lays `indices`, the dots of `area` row by row, over a page's `dots`.
/// The protocol's reader checked that the area lies in the page and that
/// the indices are its own.
fn lay_area(dots: &mut [u8], area: protocol::decoder::PageArea, indices: &[u8]) {
    let width = usize::from(interface_core::canvas::PAGE_WIDTH);
    let across = usize::from(area.width);
    for (row, line) in indices.chunks_exact(across.max(1)).enumerate() {
        let start = usize::from(area.y).saturating_add(row).saturating_mul(width).saturating_add(usize::from(area.x));
        if let Some(place) = dots.get_mut(start..start.saturating_add(across)) {
            place.copy_from_slice(line);
        }
    }
}

/// RULE-66's hops (page-rules.md): when the trusted core's thread took a
/// repaint or a `LocalMidi`, whose flush is then marked.
#[cfg(feature = "timed-test")]
fn timed_marks(message: &FromDecoder) {
    let now = std::time::Instant::now();
    match message {
        FromDecoder::Regis { picture, .. } => {
            let area = picture.area;
            protocol::timed_test::mark("core", "regis-read", now, format_args!("{},{},{},{}", area.x, area.y, area.width, area.height));
            protocol::timed_test::owe_flush();
        }
        FromDecoder::Audio(protocol::audio::Audio::LocalMidi { status, data1, .. }) => {
            protocol::timed_test::mark("core", "local-midi-read", now, format_args!("{status:02X} {data1}"));
            protocol::timed_test::owe_flush();
        }
        _ => {}
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::arithmetic_side_effects, clippy::naive_bytecount, reason = "a test")]
mod area_tests {
    use super::super::test_support::test_core;
    use super::*;

    /// ANIM-5: a changed rectangle is laid over the frame the decoder last
    /// sent of the page, and dropped when there is none or its colours are
    /// other.
    #[test]
    fn a_changed_rectangle_is_laid_over_the_frame_held() {
        let mut core = test_core();
        let palette = vec![[0, 0, 0], [255, 255, 255]];
        let key = PageKey { channel: 1, page: 0x100 };
        let area = protocol::decoder::PageArea { x: 36, y: 40, width: 12, height: 20 };
        core.decoded(1, FromDecoder::FrameArea { page: 0x100, palette: palette.clone(), screen: 0, area, indices: vec![1; 240] }).unwrap();
        assert!(!core.decoded.contains_key(&key), "no frame to lay it over");
        core.decoded(1, FromDecoder::Frame { page: 0x100, palette: palette.clone(), screen: 0, indices: vec![0; protocol::frame::PAGE_DOTS] }).unwrap();
        core.decoded(1, FromDecoder::FrameArea { page: 0x100, palette: palette.clone(), screen: 0, area, indices: vec![1; 240] }).unwrap();
        let width = usize::from(interface_core::canvas::PAGE_WIDTH);
        let dots = &core.decoded[&key].dots;
        assert_eq!(dots.iter().filter(|&&dot| dot == 1).count(), 240);
        assert!((40..60).all(|y| (36..48).all(|x| dots[y * width + x] == 1)), "in its place");
        let other = vec![[0, 0, 0], [255, 0, 0]];
        core.decoded(1, FromDecoder::FrameArea { page: 0x100, palette: other, screen: 0, area, indices: vec![0; 240] }).unwrap();
        assert_eq!(core.decoded[&key].dots.iter().filter(|&&dot| dot == 1).count(), 240, "other colours are not laid over it");
    }
}
