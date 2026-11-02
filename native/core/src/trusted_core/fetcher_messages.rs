//! What a fetcher says, untrusted (`architecture.md` section 6): a
//! channel's contents and pages, its notices, a refused certificate
//! (`certificate-errors.md`), and the console's view of it all.

use interface_core::canvas::PageKey;
use neoteletext::console::exchange_text;
use protocol::console::{ConsoleTab, Level};
use protocol::decoder::{MAX_CHUNK, MAX_WHOLE, ToDecoder};
use protocol::fetcher::{FromFetcher, MAX_NOTICE, Origin};
#[cfg(feature = "links")]
use protocol::links::Message as LinkMessage;
use protocol::{ToInterface, clean};

use super::Core;

impl Core {
    /// A connection of `channel` was refused because of its certificate
    /// (`certificate-errors.md`): the channel is marked refused, said once in
    /// the trusted core's words, and its tab's strip shows the screen.
    fn certificate_refused(&mut self, channel: u32, problem: interface_core::browser::CertificateProblem) -> Result<(), String> {
        let Some(tab) = self.session.tabs.iter_mut().find(|tab| tab.channel.id == channel) else { return Ok(()) };
        if tab.channel.refused == Some(problem) {
            return Ok(());
        }
        tab.channel.refused = Some(problem);
        let host = tab.address().map(|address| address.host.clone()).unwrap_or_default();
        self.log(Level::Warning, Some(channel), &format!("The certificate of {host} was refused: {}", problem.sentence()));
        if self.active_channel() == Some(channel) {
            self.send(&ToInterface::Strip(self.strip()))?;
        }
        Ok(())
    }

    pub(super) fn fetched(&mut self, channel: u32, message: FromFetcher) -> Result<(), String> {
        self.console_fetched(channel, &message);
        match message {
            // As a decoder's (`decoded`).
            FromFetcher::Confined => {
                if let Some(fetcher) = self.fetchers.get_mut(&channel) {
                    fetcher.hear();
                }
            }
            FromFetcher::Contents(entries) => self.contents(channel, &entries)?,
            FromFetcher::Page { origin: Origin::Fetched, bytes } => {
                // Untouched, a piece at a time; each page file ends its last
                // line, so the next one starts cleanly.
                for piece in bytes.chunks(MAX_CHUNK - 1) {
                    self.tell(channel, ToDecoder::Data(piece.to_vec()));
                }
                self.tell(channel, ToDecoder::Data(b"\n".to_vec()));
            }
            // A file or a pushed message of a form the fetcher knows goes
            // whole and at once, so its pages show as it arrives
            // (`interaction.md` section 10).
            FromFetcher::Page { origin, bytes } => {
                if bytes.len() <= MAX_WHOLE {
                    let packets = matches!(origin, Origin::PushedPackets | Origin::FetchedPackets);
                    let pushed = matches!(origin, Origin::PushedPackets);
                    self.tell(channel, ToDecoder::Whole { packets, pushed, bytes });
                }
            }
            FromFetcher::Notice(text) => self.log(Level::Warning, Some(channel), &format!("channel {channel}: {}", clean(&text, MAX_NOTICE))),
            FromFetcher::Denied { status, place } => self.denied(channel, status, &place)?,
            FromFetcher::Refused { file, status } => self.refused(channel, file, status)?,
            FromFetcher::Redirected(location) => self.redirected(channel, &location)?,
            FromFetcher::PageRedirect { page, sub, query } => self.page_redirected(channel, page, sub, query.as_deref())?,
            FromFetcher::ProgramRedirect(location) => self.program_redirected(channel, &location)?,
            // Said in the trusted core's own words, of the channel's host as
            // the strip shows it: a fetcher cannot write the strip.
            FromFetcher::CertificateRefused(problem) => self.certificate_refused(channel, problem)?,
            FromFetcher::ProxyFailed => self.proxy_failed(channel)?,
            FromFetcher::Exchange { .. } => {}
            message @ (FromFetcher::CookieQuestion { .. }
            | FromFetcher::ReachedQuestion { .. }
            | FromFetcher::CookieSet { .. }
            | FromFetcher::ClearSiteData(_)) => {
                self.cookie_message(channel, message)?;
            }
            FromFetcher::Pushing(open) => {
                self.sleep.set_pushing(channel, open);
                // A live channel wakes at once, not at the next look.
                if open {
                    self.match_decoders()?;
                }
            }
            #[cfg(feature = "interaction")]
            FromFetcher::Input { open, secure } => {
                if open && self.typing.selection(channel).is_none() {
                    self.first_selection(channel)?;
                }
                let sends = self.typing.input(channel, open, secure, self.now_us());
                self.deliver_typing(sends)?;
            }
            #[cfg(feature = "interaction")]
            FromFetcher::RoundTrip { microseconds } => {
                let sends = self.typing.round_trip(channel, microseconds);
                self.deliver_typing(sends)?;
            }
            FromFetcher::SubPages { page, part } => self.catalogue_part(channel, page, part)?,
            // A LOG record's line goes to the Log as a TTI `LG` line's does
            // (DEC-633 version 2).
            FromFetcher::ChannelLog { level, page, text } => self.channel_log(channel, level, page, &text),
            FromFetcher::InvalidPackets { total } => self.invalid_packets(channel, total),
            FromFetcher::InitialPage(page) => self.initial_page(channel, page)?,
            FromFetcher::Region(region) => self.tell(channel, ToDecoder::Region(region)),
            FromFetcher::Connecting(connecting) => self.connecting(channel, connecting)?,
            FromFetcher::SubPagesMissing { page } => {
                let changed = self.channel_subpages(channel).and_then(|subpages| subpages.missing(page));
                self.subpage_count_changed(channel, page, changed)?;
            }
            #[cfg(feature = "links")]
            FromFetcher::Links { aliases, services } => {
                self.links.descriptor(channel, &aliases, &services);
                if self.active_channel() == Some(channel) {
                    self.link(LinkMessage::Aliases { channel, numbers: self.links.aliases(channel) })?;
                }
                self.redecide_channel(channel)?;
            }
        }
        Ok(())
    }

    /// A tab never shown yet selects what its address names, or else the
    /// page its channel opens at, so its session's start tells the channel
    /// (REQ-INT-140).
    #[cfg(feature = "interaction")]
    fn first_selection(&mut self, channel: u32) -> Result<(), String> {
        let Some(tab) = self.session.all_tabs().find(|tab| tab.channel.id == channel) else { return Ok(()) };
        let address = tab.address();
        let sends = if address.is_some_and(|address| address.wall || address.contents) {
            self.typing.selects_nothing(channel, self.now_us())
        } else if let Some(page) = address.and_then(|address| address.page).or(tab.channel.initial_page) {
            let sub = address.and_then(|address| address.subpage).unwrap_or(neoteletext_core::limits::SUBCODE_ANY);
            let parameters = address.map(|address| address.parameters_of(page, sub).to_owned()).unwrap_or_default();
            self.typing.select(channel, page, sub, &parameters, self.now_us())
        } else {
            return Ok(());
        };
        self.deliver_typing(sends)
    }

    /// `channel` gave its initial page: a tab at its bare address arrives
    /// there, and the page opens (REQ-1693 item 3).
    fn initial_page(&mut self, channel: u32, page: u16) -> Result<(), String> {
        if !self.session.arrived(channel, page) {
            return Ok(());
        }
        self.in_window_of(channel, |core| {
            if core.active_channel() != Some(channel) {
                return Ok(());
            }
            core.send(&ToInterface::Strip(core.strip()))?;
            core.open_page(PageKey { channel, page }, None)
        })
    }

    /// Part of a page's catalogue of sub-pages (the engine's REQ-WEB-034):
    /// held once whole, its count told to the interface, and the open page
    /// stepped through it from now on. One out of order is refused, said
    /// in the log, and the page listed from what its decoder holds.
    fn catalogue_part(&mut self, channel: u32, page: u16, part: protocol::subpages::SubPageRows) -> Result<(), String> {
        let Some(subpages) = self.channel_subpages(channel) else { return Ok(()) };
        match subpages.part(page, part) {
            Ok(None) => Ok(()),
            Ok(Some(count)) => {
                self.send_subpage_count(channel, page, count)?;
                self.catalogue_arrived(PageKey { channel, page })
            }
            Err(neoteletext::subpages::Refused::OutOfOrder) => {
                subpages.unanswered(page);
                self.log(Level::Warning, Some(channel), &format!("channel {channel}: the catalogue of page {page:03X} came out of order; ignored"));
                Ok(())
            }
        }
    }

    /// A page's count of sub-pages to show, when it changed: to the
    /// interface while its channel is the active one.
    fn subpage_count_changed(&mut self, channel: u32, page: u16, changed: Option<u16>) -> Result<(), String> {
        match changed {
            Some(count) => self.send_subpage_count(channel, page, count),
            None => Ok(()),
        }
    }

    /// Tells the interface of a page's count, while its channel is active.
    fn send_subpage_count(&mut self, channel: u32, page: u16, count: u16) -> Result<(), String> {
        if self.active_channel() == Some(channel) {
            self.send(&ToInterface::SubPageCount { channel, page, count })?;
        }
        Ok(())
    }

    /// A channel's contents arrived: a connection succeeded.
    fn contents(&mut self, channel: u32, entries: &[protocol::fetcher::Entry]) -> Result<(), String> {
        let Some(tab) = self.session.tabs.iter_mut().find(|tab| tab.channel.id == channel) else { return Ok(()) };
        tab.channel.set_contents(entries);
        // A connection succeeded: the certificate-error screen goes.
        let was_refused = tab.channel.refused.take().is_some();
        if was_refused && self.active_channel() == Some(channel) {
            self.send(&ToInterface::Strip(self.strip()))?;
        }
        if self.active_channel() == Some(channel) {
            let view = self.session.active_tab().map(|tab| tab.channel.view());
            if let Some(view) = view {
                self.send(&ToInterface::Channel(view))?;
                self.send_subpages()?;
            }
        }
        Ok(())
    }

    /// A live channel is opening its connection, or its first page arrived
    /// (DEC-1467): the strip says it is connecting until then, never that
    /// its page was not found.
    fn connecting(&mut self, channel: u32, connecting: bool) -> Result<(), String> {
        let Some(tab) = self.session.all_tabs_mut().find(|tab| tab.channel.id == channel) else { return Ok(()) };
        tab.channel.live = true;
        tab.channel.connecting = connecting;
        if self.active_channel() == Some(channel) {
            self.send(&ToInterface::Strip(self.strip()))?;
        }
        Ok(())
    }

    /// The proxy refused a connection of `channel`: said in the strip when
    /// it is the active channel, logged when not.
    fn proxy_failed(&mut self, channel: u32) -> Result<(), String> {
        let host = self.session.tabs.iter().find(|tab| tab.channel.id == channel).and_then(|tab| tab.address()).map(|address| address.host.clone());
        if let Some(host) = host {
            let text = format!("The proxy refused the connection to {host}");
            self.console.log(Some(channel), Level::Warning, self.now(), &text);
            if self.active_channel() == Some(channel) {
                self.say(text)?;
            } else {
                self.log(Level::Warning, Some(channel), &text);
            }
        }
        Ok(())
    }

    /// What a fetcher said, as the console's entries: exchanges and push
    /// events, packets, notices (requirement 19.9).
    fn console_fetched(&mut self, channel: u32, message: &FromFetcher) {
        let at = self.now();
        match message {
            FromFetcher::Page { origin, bytes } => {
                let pushed = matches!(origin, Origin::PushedPackets);
                let packets = matches!(origin, Origin::PushedPackets | Origin::FetchedPackets);
                self.console.received(channel, pushed, packets, bytes);
                if packets {
                    self.console.packets(channel, at, pushed, bytes);
                }
                if pushed {
                    let form = if packets { "packets" } else { "a page" };
                    self.console.add(channel, ConsoleTab::Network, at, &format!("push: {form}, {} bytes", bytes.len()));
                }
            }
            FromFetcher::Pushing(open) => {
                if *open {
                    self.console.connected(channel);
                }
                self.console.add(channel, ConsoleTab::Network, at, if *open { "push: connected" } else { "push: lost" });
            }
            FromFetcher::Exchange { method, url, status, bytes, elapsed_us } => {
                self.console.fetched(channel, *bytes);
                self.console.add(channel, ConsoleTab::Network, at, &exchange_text(method, url, *status, *bytes, *elapsed_us));
            }
            FromFetcher::Notice(text) => self.console.log(Some(channel), Level::Warning, at, &format!("fetcher: {text}")),
            _ => {}
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "a test")]
mod tests {
    use super::super::test_support::Test;
    use super::*;

    // REQ-INT-140, TODO-1536 item 3: a tab whose address names no page
    // selects the page its descriptor opens at, shown or not.
    #[cfg(feature = "interaction")]
    #[test]
    fn a_tab_never_shown_selects_its_channels_initial_page() {
        use protocol::fetcher::FromFetcher;
        let mut test = Test::new("initial page");
        test.core.session.open("news.invalid").unwrap();
        let channel = test.core.session.tabs[0].channel.id;
        test.core.fetched(channel, FromFetcher::InitialPage(0x302)).unwrap();
        test.core.fetched(channel, FromFetcher::Input { open: true, secure: true }).unwrap();
        assert_eq!(test.core.typing.selection(channel), Some((0x302, neoteletext_core::limits::SUBCODE_ANY)));
    }

    /// Requirement 7.13: a channel's refused certificate reaches its strip
    /// and the Log once, and goes when the channel's contents arrive.
    #[test]
    fn a_refused_certificate_shows_until_the_contents_arrive() {
        use interface_core::browser::CertificateProblem;
        use protocol::fetcher::FromFetcher;
        let mut test = Test::new("certificate");
        test.core.session.open("news.invalid").unwrap();
        let channel = test.core.session.tabs[0].channel.id;
        test.sent();
        test.core.fetched(channel, FromFetcher::CertificateRefused(CertificateProblem::Expired)).unwrap();
        test.core.fetched(channel, FromFetcher::CertificateRefused(CertificateProblem::Expired)).unwrap();
        let strips: Vec<_> = test.sent().into_iter().filter_map(|message| if let ToInterface::Strip(strip) = message { Some(strip) } else { None }).collect();
        assert_eq!(strips.len(), 1, "once, however often the agent retries");
        assert_eq!((strips[0].refused, strips[0].identity.as_str()), (Some(CertificateProblem::Expired), "news.invalid"));
        test.core.console.set_open(true);
        let logged =
            test.core.console.updates(Some(channel)).iter().flat_map(|update| update.entries.iter()).filter(|entry| entry.text.contains("was refused")).count();
        assert_eq!(logged, 1, "said once in the Log");
        test.core.fetched(channel, FromFetcher::Contents(Vec::new())).unwrap();
        let strips: Vec<_> = test.sent().into_iter().filter_map(|message| if let ToInterface::Strip(strip) = message { Some(strip) } else { None }).collect();
        assert_eq!(strips.last().map(|strip| strip.refused), Some(None));
    }

    /// A tab on a static channel, with the page `open` open; its channel.
    fn static_tab(test: &mut Test, open: u16) -> u32 {
        test.core.session.open("https://news.invalid/teletext/").unwrap();
        let channel = test.core.active_channel().unwrap();
        test.core.intent(protocol::ToCore::Opened(Some(PageKey { channel, page: open }))).unwrap();
        channel
    }

    /// REQ-683: a decoder or a fetcher that broke its protocol fails its
    /// tab, in the strip's state and so on its page area, and none starts
    /// for the channel again, so a hostile server cannot crash one for
    /// ever; a reload makes a new channel, which has neither. One that
    /// merely ended fails nothing.
    #[cfg(unix)]
    #[test]
    fn a_helper_that_broke_its_protocol_fails_its_tab_until_it_is_reloaded() {
        use super::super::processes::Fetcher;
        use super::super::test_support::{decoder_watched, stand_in};
        use interface_core::browser::Failed;
        let mut test = Test::new("helper-broke");
        let channel = static_tab(&mut test, 0x100);
        let fetcher = |test: &mut Test| {
            test.core.fetchers.insert(channel, Fetcher::new(stand_in("/bin/sleep", &["60"], confine::Role::Network), 1, None));
        };
        fetcher(&mut test);
        test.core.fetcher_ended(channel, 1, None);
        assert_eq!(test.core.strip().failed, None, "it ended, and may start again");
        assert!(!test.core.given_up.contains(&channel));
        fetcher(&mut test);
        test.core.fetcher_ended(channel, 1, Some("a malformed message".into()));
        assert_eq!(test.core.strip().failed, Some(Failed::Fetcher));
        assert!(test.core.given_up.contains(&channel), "no new fetcher");
        test.core.match_decoders().unwrap();
        assert!(test.core.fetchers.is_empty(), "none started at the next look");
        test.core.session.page_drawn(channel);
        assert_eq!(test.core.strip().failed, Some(Failed::Fetcher), "a page drawn does not mend it");

        let mut test = Test::new("decoder-broke");
        let channel = static_tab(&mut test, 0x100);
        let _written = decoder_watched(&mut test.core, channel);
        test.core.decoder_ended(channel, 1, Some("a malformed message".into()));
        assert_eq!(test.core.strip().failed, Some(Failed::Decoder));
        assert!(test.core.given_up.contains(&channel));
        test.core.session.reload();
        assert_eq!(test.core.strip().failed, None, "a reload is a new channel");
    }

    /// REQ-708: `channel.ttx` refused, whatever came back, leaves the tab
    /// saying so with the status. On the code before, a line in the Log,
    /// and the tab stayed empty.
    #[test]
    fn a_refused_channel_is_said_in_the_tab_with_its_status() {
        use interface_core::browser::Failed;
        use protocol::fetcher::RefusedFile;
        for status in [Some(200), Some(403), Some(500), None] {
            let mut test = Test::new("refused-channel");
            let channel = static_tab(&mut test, 0x100);
            test.core.fetched(channel, FromFetcher::Refused { file: RefusedFile::Channel, status }).unwrap();
            assert_eq!(test.core.strip().failed, Some(Failed::Channel(status)));
        }
    }

    /// REQ-708: a refused page is said for the page open and for no other,
    /// and never over a page that has arrived; another page takes it away,
    /// and a page drawn ends it. A bundle or a contents file is missed, not
    /// needed: the Log alone says so.
    #[test]
    fn a_refused_page_is_said_for_the_page_open_alone() {
        use interface_core::browser::Failed;
        use protocol::fetcher::RefusedFile;
        let mut test = Test::new("refused-page");
        let channel = static_tab(&mut test, 0x100);
        let refused = |test: &mut Test, file, status| test.core.fetched(channel, FromFetcher::Refused { file, status }).unwrap();
        refused(&mut test, RefusedFile::Bundle, Some(500));
        refused(&mut test, RefusedFile::Contents, Some(403));
        refused(&mut test, RefusedFile::Page(0x200), Some(500));
        assert_eq!(test.core.strip().failed, None);
        refused(&mut test, RefusedFile::Page(0x100), Some(200));
        assert_eq!(test.core.strip().failed, Some(Failed::Page(Some(200))), "an answer that was not teletext");
        test.core.intent(protocol::ToCore::Opened(Some(PageKey { channel, page: 0x101 }))).unwrap();
        assert_eq!(test.core.strip().failed, None, "another page open");
        test.core.intent(protocol::ToCore::Opened(Some(PageKey { channel, page: 0x100 }))).unwrap();
        assert_eq!(test.core.strip().failed, Some(Failed::Page(Some(200))), "it stands for its page");
        test.core.session.page_drawn(channel);
        assert_eq!(test.core.strip().failed, None, "a page drew");
        test.core.session.all_tabs_mut().next().unwrap().channel.arrived(0x100, "Headlines");
        refused(&mut test, RefusedFile::Page(0x100), Some(500));
        assert_eq!(test.core.strip().failed, None, "a page that arrived is not disturbed");
    }
}
