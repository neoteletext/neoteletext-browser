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
        let active = self.active_channel();
        match message {
            FromDecoder::StoreFull => self.grow_decoder(channel)?,
            // Only a decoder started to read bookmarks reports them; from a
            // channel's they change nothing.
            // The channel's own log, for its author: in its Log tab only,
            // cleaned and marked as the channel's (decision 70).
            FromDecoder::ChannelLog { level, page, text } => self.channel_log(channel, level, page, &text),
            FromDecoder::Bookmark(_) | FromDecoder::BookmarksRead { .. } => {
                self.log(Level::Warning, Some(channel), "the channel's decoder reported bookmarks, which it may not; ignored");
            }
            FromDecoder::Arrived { page, title } => self.arrived(channel, page, &title)?,
            FromDecoder::Updated { page } => {
                let at = self.now();
                self.console.add(channel, ConsoleTab::Pages, at, &format!("page {page:03X} changed"));
                if active == Some(channel) {
                    self.send(&ToInterface::Updated { channel, page })?;
                }
            }
            FromDecoder::Frame { page, palette, screen, indices } => self.frame(PageKey { channel, page }, palette, screen, indices)?,
            #[cfg(feature = "links")]
            FromDecoder::Links { page, with_image, candidates, numbers, fastext } => {
                let key = PageKey { channel, page };
                self.links.found(key, candidates, numbers, fastext);
                if !with_image {
                    self.redecide(key)?;
                    self.send_controls()?;
                }
            }
            FromDecoder::SubPages { page, subcodes, shown } => self.subpages(PageKey { channel, page }, subcodes, shown)?,
            #[cfg(feature = "interaction")]
            FromDecoder::Cursor { body } => {
                let sends = self.typing.cursor(channel, body, self.now());
                self.deliver_typing(sends)?;
            }
            #[cfg(feature = "interaction")]
            FromDecoder::InputControl { body } => {
                let sends = self.typing.control(channel, &body, self.now());
                self.deliver_typing(sends)?;
            }
            FromDecoder::Audio(audio) => {
                if let Some(message) = self.hearing.decoded(channel, audio) {
                    self.tell_sound(&message);
                }
            }
        }
        Ok(())
    }

    /// A line of the channel's log, at `level`, of `page` if it names one.
    fn channel_log(&mut self, channel: u32, level: Level, page: Option<u16>, text: &str) {
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
