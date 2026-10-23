//! Hyperlinks (`hyperlinks.md`): the trusted core decides every link and
//! follows it by its own rules.

use interface_core::canvas::PageKey;
use neoteletext::links::{Channel as LinkChannel, Follow, Links, Refusal};
use protocol::console::Level;
use protocol::links::{LinkKind, Message as LinkMessage, Pointing};
use protocol::{ToCore, ToInterface};

use super::Core;

impl Core {
    pub(super) fn link(&mut self, message: LinkMessage) -> Result<(), String> {
        self.send(&ToInterface::Link(message))
    }

    /// Runs `decide` with the link table and channel `id` as links see it.
    fn with_channel<T>(&mut self, id: u32, decide: impl FnOnce(&mut Links, &LinkChannel<'_>) -> T) -> Option<T> {
        let Self { session, links, .. } = self;
        let tab = session.all_tabs().find(|tab| tab.channel.id == id)?;
        let address = tab.address()?;
        let exists = |page: u16| tab.channel.has_page(page);
        Some(decide(links, &LinkChannel { id, address, exists: &exists }))
    }

    /// A new image of a page was sent: its links, for the interface.
    pub(super) fn links_of_image(&mut self, key: PageKey, version: u32) -> Result<(), String> {
        let now = self.now();
        if let Some(links) = self.with_channel(key.channel, |table, channel| table.image(key, version, channel, now)) {
            self.link(LinkMessage::Page { key, links })?;
        }
        self.repoint()
    }

    /// Decides a page's links again, and sends them when they changed.
    pub(super) fn redecide(&mut self, key: PageKey) -> Result<(), String> {
        let now = self.now();
        if let Some(Some(links)) = self.with_channel(key.channel, |table, channel| table.redecide(key, channel, now)) {
            self.link(LinkMessage::Page { key, links })?;
        }
        self.repoint()
    }

    /// Decides again the links of every page on screen of `channel`.
    pub(super) fn redecide_channel(&mut self, channel: u32) -> Result<(), String> {
        for key in self.links.pages_of(channel) {
            self.redecide(key)?;
        }
        Ok(())
    }

    /// The strip's destination follows a change of links.
    fn repoint(&mut self) -> Result<(), String> {
        match self.links.repoint() {
            Some(pointing) => self.link(LinkMessage::Pointing(pointing)),
            None => Ok(()),
        }
    }

    /// After a change of tab or channel: no page's links, no question in
    /// the strip, the channel's aliases, and the page its address names.
    pub(super) fn send_links_afresh(&mut self) -> Result<(), String> {
        self.links.clear_pages();
        self.links.forget_prompt();
        self.link(LinkMessage::Prompt(None))?;
        self.link(LinkMessage::Pointing(None))?;
        let Some(channel) = self.active_channel() else { return Ok(()) };
        self.link(LinkMessage::Aliases { channel, numbers: self.links.aliases(channel) })?;
        let page = self.session.active_tab().and_then(|tab| tab.address()).and_then(|address| address.page);
        match page {
            Some(page) => self.link(LinkMessage::Show(PageKey { channel, page })),
            None => Ok(()),
        }
    }

    pub(super) fn link_intent(&mut self, intent: &ToCore) -> Result<(), String> {
        let now = self.now();
        match intent {
            ToCore::Point(spot) => {
                let pointing = self.links.point(*spot);
                self.link(LinkMessage::Pointing(pointing))?;
            }
            ToCore::Typed { number, opened } => {
                let followed = self.links.typed(*number, *opened, self.active_channel(), now);
                self.follow(followed, "a typed page number")?;
            }
            ToCore::Confirm(id) => {
                let followed = self.links.confirm(*id, now);
                self.follow(followed, "a confirmation")?;
            }
            // Declining has no effect but to close the question (requirement 8.6).
            ToCore::Cancel(id) if self.links.cancel(*id) => self.link(LinkMessage::Prompt(None))?,
            _ => {}
        }
        Ok(())
    }

    /// Carries out what a click, a typed number or a confirmation led to.
    pub(super) fn follow(&mut self, followed: Result<Follow, Refusal>, what: &str) -> Result<(), String> {
        match followed {
            Err(Refusal::NotALink) => Ok(()),
            Err(refusal) => {
                self.log(Level::Warning, None, &format!("{what} was not followed: {refusal:?}"));
                Ok(())
            }
            Ok(Follow::Page(page)) => {
                let Some(channel) = self.active_channel() else { return Ok(()) };
                // A link to the page shown reloads it, as in Chrome.
                let shown = self.session.active_tab().and_then(|tab| tab.address()).is_some_and(|shown| shown.page == Some(page));
                if shown {
                    return self.reload();
                }
                if self.session.show_page(page).is_some() {
                    self.send(&ToInterface::Strip(self.strip()))?;
                }
                self.link(LinkMessage::Show(PageKey { channel, page }))
            }
            Ok(Follow::Ask(prompt)) => self.link(LinkMessage::Prompt(Some(prompt))),
            Ok(Follow::Leave(address)) => {
                self.session.follow(address)?;
                self.send_all()
            }
            Ok(Follow::HandOff(address)) => {
                self.link(LinkMessage::Prompt(None))?;
                self.hand_off(&address)
            }
        }
    }

    /// Hands a web page to the system's web browser, logged with the page
    /// it came from and the time (requirement 17.9).
    fn hand_off(&mut self, address: &str) -> Result<(), String> {
        let from = self.links.opened_page().map(|key| format!("page {:03X} of channel {}", key.page, key.channel)).unwrap_or_default();
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |since| since.as_secs());
        match self.broker()?.open_web(address) {
            Ok(()) => {
                self.log(Level::Info, None, &format!("handed {address} to the web browser, from {from}, at {at} seconds past 1970"));
                Ok(())
            }
            Err(error) => {
                self.log(Level::Warning, None, &format!("{address}: {error}"));
                self.link(LinkMessage::Pointing(Some(Pointing { kind: LinkKind::Web, text: "Could not open your web browser".into() })))
            }
        }
    }
}
