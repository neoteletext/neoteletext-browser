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
        // The open page's controls say what it has brought (REQ-6352).
        if matches!(message, FromFetcher::Page { .. }) && self.selection_shown(channel).is_some() {
            self.send_controls()?;
        }
        match message {
            // As a decoder's (`decoded`).
            FromFetcher::Confined => {
                if let Some(fetcher) = self.fetchers.get_mut(&channel) {
                    fetcher.hear();
                }
            }
            FromFetcher::Contents(entries) => self.contents(channel, &entries)?,
            // It lists nothing, and the Log has said why.
            FromFetcher::NoContents => {}
            FromFetcher::BundleSettled => self.bundle_settled(channel),
            FromFetcher::Refresh(seconds) => self.refresh_declared(channel, seconds),
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
            FromFetcher::Refused { file, status } => {
                if let protocol::fetcher::RefusedFile::Page(page) = file {
                    self.page_refused(channel, page);
                }
                self.refused(channel, file, status)?;
            }
            FromFetcher::Redirected(location) => self.redirected(channel, &location)?,
            FromFetcher::PageRedirect { page, sub, query } => self.page_redirected(channel, page, sub, query.as_deref())?,
            FromFetcher::ProgramRedirect(location) => self.program_redirected(channel, &location)?,
            // Said in the trusted core's own words, of the channel's host as
            // the strip shows it: a fetcher cannot write the strip.
            FromFetcher::CertificateRefused(problem) => self.certificate_refused(channel, problem)?,
            FromFetcher::ProxyFailed => self.proxy_failed(channel)?,
            FromFetcher::TimedOut => self.timed_out(channel)?,
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
            FromFetcher::SubPages { page, version, part } => self.catalogue_part(channel, page, &version, part)?,
            // A LOG record's line goes to the Log as a TTI `LG` line's does
            // (DEC-633 version 2).
            FromFetcher::ChannelLog { level, page, text } => self.channel_log(channel, level, page, &text),
            FromFetcher::InvalidPackets { total } => self.invalid_packets(channel, total),
            FromFetcher::InitialPage(page) => self.initial_page(channel, page)?,
            FromFetcher::Region(region) => self.tell(channel, ToDecoder::Region(region)),
            FromFetcher::Connecting(connecting) => self.connecting(channel, connecting)?,
            FromFetcher::SubPagesMissing { page, part } => {
                let changed = self.channel_subpages(channel).and_then(|subpages| subpages.missing(page, part));
                self.subpage_count_changed(channel, page, changed)?;
                self.answer_rows_wanted(PageKey { channel, page }, true)?;
            }
            // It stays asked until its page opens again; rows waiting for it
            // are answered with what is held.
            FromFetcher::SubPagesFailed { page, .. } => {
                if let Some(subpages) = self.channel_subpages(channel) {
                    subpages.failed(page);
                }
                self.answer_rows_wanted(PageKey { channel, page }, true)?;
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

    /// A part of a page's list of sub-pages (REQ-1888): held with the
    /// others of its version, the page's count told to the interface, and
    /// the open page stepped through it from now on; the parts of another
    /// version held before are asked for again (DOC-1869), as each was
    /// asked for the open page or the rows the list shows. One not the
    /// shape of a part is refused, said in the log, and the page listed from
    /// what was held and what its decoder holds.
    fn catalogue_part(&mut self, channel: u32, page: u16, version: &str, part: protocol::subpages::SubPageRows) -> Result<(), String> {
        let Some(subpages) = self.channel_subpages(channel) else { return Ok(()) };
        let key = PageKey { channel, page };
        match subpages.part(page, version, part) {
            Ok(arrived) => {
                self.subpage_count_changed(channel, page, arrived.count)?;
                self.ask_parts(key, &arrived.again);
                self.catalogue_arrived(key)
            }
            Err(neoteletext::subpages::Refused::Misshapen) => {
                self.log(
                    Level::Warning,
                    Some(channel),
                    &format!("channel {channel}: a part of the list of sub-pages of page {page:03X} was not the shape of a part; ignored"),
                );
                self.answer_rows_wanted(key, true)
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
        self.connection_failed(channel, |host| format!("The proxy refused the connection to {host}"))
    }

    /// A connection of `channel` did not open in time (TODO-1277): said in
    /// the trusted core's own words, as the proxy's failure is, and not as
    /// a certificate refused.
    fn timed_out(&mut self, channel: u32) -> Result<(), String> {
        self.connection_failed(channel, |host| format!("{host} did not answer in time"))
    }

    /// A connection of `channel` failed, as `text` says of its host: its
    /// channel's Log, and the strip when the tab is in front.
    fn connection_failed(&mut self, channel: u32, text: impl FnOnce(&str) -> String) -> Result<(), String> {
        let host = self.session.tabs.iter().find(|tab| tab.channel.id == channel).and_then(|tab| tab.address()).map(|address| address.host.clone());
        if let Some(host) = host {
            let text = text(&host);
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
                // What the open page or sub-page has brought (REQ-6352).
                self.console.downloaded(channel, self.selection_shown(channel), bytes.len());
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
    use protocol::fetcher::ToFetcher;

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

    /// TODO-1277: a connection that did not open in time is said in the
    /// strip and the channel's Log as the channel not answering in time,
    /// and no certificate is shown as refused.
    #[test]
    fn a_connection_out_of_time_is_said_and_refuses_no_certificate() {
        use protocol::fetcher::FromFetcher;
        let mut test = Test::new("timed out");
        test.core.session.open("news.invalid").unwrap();
        let channel = test.core.session.tabs[0].channel.id;
        test.core.fetched(channel, FromFetcher::TimedOut).unwrap();
        let strip = test.core.strip();
        assert_eq!(strip.notice, "news.invalid did not answer in time");
        assert_eq!(strip.refused, None, "no certificate refused");
        test.core.console.set_open(true);
        let logged =
            test.core.console.updates(Some(channel)).iter().flat_map(|update| update.entries.iter()).any(|entry| entry.text.contains("did not answer in time"));
        assert!(logged, "said in the channel's Log");
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

    // TODO-1531 item 3, DOC-1956: a LOG record's line reaches the console's
    // Log of its own channel only, marked as the channel's, with its level and
    // page, and cleaned of what the console does not show.
    #[test]
    fn a_channels_log_line_is_in_its_own_log_marked_as_the_channels() {
        use interface_core::console::Level;
        use protocol::fetcher::FromFetcher;
        let mut test = Test::new("channel log");
        test.core.session.open("https://news.invalid/teletext/").unwrap();
        let channel = test.core.session.tabs[0].channel.id;
        test.core.session.open("https://other.invalid/teletext/").unwrap();
        let other = test.core.session.tabs[1].channel.id;
        test.core.fetched(channel, FromFetcher::ChannelLog { level: Level::Warning, page: Some(0x302), text: "Feed\u{202E} late".into() }).unwrap();
        test.core.fetched(channel, FromFetcher::ChannelLog { level: Level::Info, page: None, text: "Started".into() }).unwrap();
        test.core.console.set_open(true);
        let logged = |test: &mut Test, channel| -> Vec<(Level, String)> {
            test.core
                .console
                .updates(Some(channel))
                .iter()
                .flat_map(|update| update.entries.clone())
                .filter(|entry| entry.tab == ConsoleTab::Log && entry.text.starts_with("channel"))
                .map(|entry| (entry.level, entry.text))
                .collect()
        };
        assert_eq!(logged(&mut test, channel), [(Level::Warning, "channel, page 302: Feed late".to_owned()), (Level::Info, "channel: Started".to_owned())]);
        assert_eq!(logged(&mut test, other), [], "another channel's Log has none of it");
    }

    // DEC-1527, REQ-714, TODO-1534 item 3: a tab whose connection is lost
    // and opens again starts a session of its own on its own fetcher; the
    // other tab on the same channel is told nothing, keeps its selection and
    // its session, and stays live.
    #[cfg(all(unix, feature = "interaction"))]
    #[test]
    fn a_tabs_reconnection_leaves_the_other_tab_on_its_channel_untouched() {
        use super::super::test_support::{core_on, input_items, sent_to_fetcher, wire};
        use protocol::fetcher::FromFetcher;
        const LIVE: &str = "https://live.invalid/teletext/";
        // The stand-in broker starts the decoders a live tab wakes.
        let (mut core, _stand_in) = core_on(LIVE);
        let first = core.session.tabs[0].channel.id;
        core.session.open(LIVE).unwrap();
        let second = core.session.tabs[1].channel.id;
        assert_ne!(first, second, "a channel number each");
        let _ = core.typing.select(first, 0x200, neoteletext_core::limits::SUBCODE_ANY, "", 0);
        let _ = core.typing.select(second, 0x500, 2, "", 0);
        let (first_wire, second_wire) = (wire(&mut core, first), wire(&mut core, second));
        core.fetched(second, FromFetcher::Pushing(true)).unwrap();
        // Long after it was last seen, a tab is awake only while it is live.
        let later = std::time::Instant::now() + 2 * neoteletext::sleep::SLEEP_AFTER;
        let second_before = (core.typing.selection(second), core.sleep.awake(second, None, later));
        assert!(second_before.1, "live");

        for (pushing, open) in [(false, false), (true, true)] {
            core.fetched(first, FromFetcher::Pushing(pushing)).unwrap();
            core.fetched(first, FromFetcher::Input { open, secure: true }).unwrap();
        }
        let hello = input_items(&sent_to_fetcher(&mut core, first, &first_wire));
        assert!(matches!(hello.as_slice(), [hello] if hello.starts_with("Event(Hello")), "the first tab's new session: {hello:?}");
        assert_eq!(sent_to_fetcher(&mut core, second, &second_wire), [], "nothing to the second tab's connection");
        assert_eq!((core.typing.selection(second), core.sleep.awake(second, None, later)), second_before, "its selection, and live");
        assert_eq!(core.typing.selection(first), Some((0x200, neoteletext_core::limits::SUBCODE_ANY)), "the first keeps its own");
        let sends = core.typing.select(second, 0x500, 3, "", 0);
        core.deliver_typing(sends).unwrap();
        let said = input_items(&sent_to_fetcher(&mut core, second, &second_wire));
        assert_eq!(said, ["Event(Select(Select { page: 1280, sub: 3 }, \"\"))"], "its session goes on, welcomed");
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

    /// REQ-2402, TODO-1251 (decision 78): on Windows, where the fetcher's container
    /// reaches this computer only once an administrator exempts it, a
    /// channel on this computer that nothing answered is said in the strip
    /// to be blocked by Windows, with the command; the settings panel is
    /// given the command too. A channel elsewhere is said as before, and
    /// with no command (macOS, Linux) nothing changes. Before, the strip
    /// said nothing of why and the panel had no command.
    #[test]
    fn a_channel_on_this_computer_refused_on_windows_is_said_with_the_exemption() {
        use protocol::fetcher::RefusedFile;
        let command = String::from("CheckNetIsolation.exe LoopbackExempt -a -p=S-1-15-2-1-2-3-4-5-6-7");
        let refused = |address: &str, exemption: Option<&String>| {
            let mut test = Test::new("loopback-refused");
            test.core.loopback_exemption = exemption.cloned();
            test.core.session.open(address).unwrap();
            let channel = test.core.active_channel().unwrap();
            test.core.fetched(channel, FromFetcher::Refused { file: RefusedFile::Channel, status: None }).unwrap();
            test.core.send_settings().unwrap();
            let sent = test.sent();
            let panel =
                sent.iter().rev().find_map(|message| if let ToInterface::Settings(view) = message { Some(view.loopback_exemption.clone()) } else { None });
            (test.core.strip().notice, panel.flatten())
        };
        let (notice, panel) = refused("http://127.0.0.1:8080/teletext/", Some(&command));
        assert_eq!(notice, format!("Windows blocks channels on this computer until an administrator runs {command}"));
        assert_eq!(panel, Some(command.clone()));
        let (notice, _) = refused("https://news.invalid/teletext/", Some(&command));
        assert!(!notice.contains("CheckNetIsolation"), "{notice}");
        let (notice, panel) = refused("http://127.0.0.1:8080/teletext/", None);
        assert!(!notice.contains("CheckNetIsolation"), "{notice}");
        assert_eq!(panel, None);
    }

    /// DEC-2364, REQ-847: when nothing answers for the home address, as
    /// with no network, the built-in start page opens in its place, and
    /// Back tries the home again; another address that nothing answers
    /// stays, saying so, and so does a home that answered with a refusal.
    /// On the code before, the home channel's tab said "Channel not
    /// reached" and offered nothing.
    #[cfg(unix)]
    #[test]
    fn an_unreached_home_opens_the_start_page_in_its_place() {
        use interface_core::browser::Failed;
        use protocol::fetcher::RefusedFile;
        let home = neoteletext::address::HOME_CHANNEL;
        for (address, status, falls_back) in [(home, None, true), (home, Some(503), false), ("https://news.invalid/teletext/", None, false)] {
            let (mut core, _stand_in) = super::super::test_support::core_on(address);
            let channel = core.active_channel().unwrap();
            core.fetched(channel, FromFetcher::Refused { file: RefusedFile::Channel, status }).unwrap();
            let shown = core.session.active_tab().unwrap().address().unwrap().clone();
            assert_eq!(shown.is_start_page(), falls_back, "{address} {status:?}");
            if falls_back {
                core.intent(protocol::ToCore::Back).unwrap();
                assert_eq!(core.session.active_tab().unwrap().address().unwrap().text, home, "Back tries the home again");
            } else {
                assert_eq!(core.strip().failed, Some(Failed::Channel(status)));
            }
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

    /// What the core asked of fetchers since last asked, taken.
    fn asked(test: &mut Test) -> Vec<(u32, ToFetcher)> {
        core::mem::take(&mut test.core.controls.told_fetchers)
    }

    /// REQ-703: a static channel's page is asked for once its bundle has
    /// settled, and not before, since the bundle may hold it; then each page
    /// opened, once until it arrives or is refused; and its listing once,
    /// after its page, as its tab is in front. On the code before, the
    /// contents file was fetched as the channel opened, and a page was
    /// asked for only once the contents file had answered.
    #[test]
    fn a_static_channels_wanted_pages_are_asked_for_once_its_bundle_settles() {
        use protocol::fetcher::RefusedFile;
        let mut test = Test::new("wanted");
        let channel = static_tab(&mut test, 0x100);
        assert_eq!(asked(&mut test), [], "not before the bundle has settled");
        test.core.fetched(channel, FromFetcher::NoContents).unwrap();
        assert_eq!(asked(&mut test), [], "a contents file says nothing of the bundle");
        test.core.fetched(channel, FromFetcher::BundleSettled).unwrap();
        assert_eq!(asked(&mut test), [(channel, ToFetcher::FetchPage { page: 0x100 }), (channel, ToFetcher::FetchContents)]);
        test.core.fetched(channel, FromFetcher::BundleSettled).unwrap();
        assert_eq!(asked(&mut test), [], "each once");
        test.core.intent(protocol::ToCore::Opened(Some(PageKey { channel, page: 0x200 }))).unwrap();
        assert_eq!(asked(&mut test), [(channel, ToFetcher::FetchPage { page: 0x200 })], "selecting a page fetches it");
        test.core.intent(protocol::ToCore::Opened(Some(PageKey { channel, page: 0x100 }))).unwrap();
        assert_eq!(asked(&mut test), [], "asked already, and on its way");
        test.core.fetched(channel, FromFetcher::Refused { file: RefusedFile::Page(0x200), status: Some(503) }).unwrap();
        test.core.intent(protocol::ToCore::Opened(Some(PageKey { channel, page: 0x200 }))).unwrap();
        assert_eq!(asked(&mut test), [(channel, ToFetcher::FetchPage { page: 0x200 })], "a refused page is asked again when wanted");
        test.core.session.all_tabs_mut().next().unwrap().channel.arrived(0x300, "Sport");
        test.core.intent(protocol::ToCore::Opened(Some(PageKey { channel, page: 0x300 }))).unwrap();
        assert_eq!(asked(&mut test), [], "it has arrived");
    }

    /// REQ-703: a page the contents file lists and nothing wants is never
    /// asked for; the pages the interface shows of the channel, the wall's
    /// thumbnails, are, each once. On the code before, every listed page was
    /// fetched in turn, and a thumbnail never.
    #[test]
    fn a_listed_page_nothing_wants_is_not_asked_for_and_a_thumbnail_shown_is() {
        use protocol::fetcher::Entry;
        let mut test = Test::new("listed");
        let channel = static_tab(&mut test, 0x100);
        test.core.fetched(channel, FromFetcher::BundleSettled).unwrap();
        asked(&mut test);
        let listed = (0x100..0x110).map(|page| Entry::Page { page, title: format!("Page {page:03X}"), subpages: 0 }).collect();
        test.core.fetched(channel, FromFetcher::Contents(listed)).unwrap();
        assert_eq!(asked(&mut test), [], "listed is not wanted");
        let shown = |pages: &[u16]| protocol::ToCore::Visible(pages.iter().map(|&page| PageKey { channel, page }).collect());
        test.core.intent(shown(&[0x100, 0x101, 0x102])).unwrap();
        assert_eq!(asked(&mut test), [(channel, ToFetcher::FetchPage { page: 0x101 }), (channel, ToFetcher::FetchPage { page: 0x102 })]);
        test.core.intent(shown(&[0x101, 0x102, 0x103])).unwrap();
        assert_eq!(asked(&mut test), [(channel, ToFetcher::FetchPage { page: 0x103 })], "each once");
    }

    /// REQ-703: a tab behind another has the page it shows asked for when
    /// its bundle settles, and its listing only once it comes to the front.
    #[test]
    fn a_tab_behind_asks_for_its_listing_only_when_it_comes_to_the_front() {
        let mut test = Test::new("behind");
        let behind = static_tab(&mut test, 0x100);
        test.core.session.open("https://other.invalid/teletext/").unwrap();
        asked(&mut test);
        test.core.fetched(behind, FromFetcher::BundleSettled).unwrap();
        assert_eq!(asked(&mut test), [(behind, ToFetcher::FetchPage { page: 0x100 })]);
        let index = test.core.session.tabs.iter().position(|tab| tab.channel.id == behind).unwrap();
        // As `send_all` does when the tab is chosen; this core has no
        // broker to start its helpers with.
        test.core.session.active = index;
        test.core.ask_wanted(behind);
        assert!(asked(&mut test).contains(&(behind, ToFetcher::FetchContents)), "its listing is wanted now");
    }

    /// REQ-687 as amended by DEC-997 and DEC-2356: the interface's reports
    /// of pages open and shown ask for pages of the active tab's channel
    /// alone, by page number, and nothing it names of another tab's channel
    /// reaches that channel's fetcher. It held on the code before for the
    /// page open, and the wall's pages were not asked for at all.
    #[test]
    fn the_interface_asks_for_pages_of_the_active_tabs_channel_alone() {
        let mut test = Test::new("active-alone");
        let behind = static_tab(&mut test, 0x100);
        test.core.session.open("https://front.invalid/teletext/").unwrap();
        let front = test.core.active_channel().unwrap();
        for channel in [behind, front] {
            test.core.fetched(channel, FromFetcher::BundleSettled).unwrap();
        }
        asked(&mut test);
        for page in [0x200, 0x201] {
            test.core.intent(protocol::ToCore::Opened(Some(PageKey { channel: behind, page }))).unwrap();
            test.core.intent(protocol::ToCore::Visible(vec![PageKey { channel: behind, page }, PageKey { channel: front, page }])).unwrap();
        }
        test.core.intent(protocol::ToCore::Opened(Some(PageKey { channel: front, page: 0x300 }))).unwrap();
        let asked = asked(&mut test);
        assert!(asked.iter().all(|(channel, _)| *channel == front), "{asked:?}");
        assert_eq!(
            asked.into_iter().map(|(_, message)| message).collect::<Vec<_>>(),
            [ToFetcher::FetchPage { page: 0x200 }, ToFetcher::FetchPage { page: 0x201 }, ToFetcher::FetchPage { page: 0x300 }]
        );
    }

    /// DEC-1467, DEC-1482, TODO-1470 item 4: a live channel's contents
    /// come from its index, and a later index replaces the one before at
    /// once: a page it no longer lists leaves the contents, a retitled page
    /// shows its new title, and the interface is sent the channel each time.
    #[test]
    fn a_live_channels_later_index_replaces_its_contents_at_once() {
        use interface_core::channel::page_index;
        use protocol::fetcher::Entry;
        let mut test = Test::new("live-index");
        let channel = static_tab(&mut test, 0x100);
        test.core.fetched(channel, FromFetcher::Connecting(true)).unwrap();
        test.sent();
        let index = |pages: &[(u16, &str)]| {
            let mut entries = vec![Entry::Section("News".into())];
            entries.extend(pages.iter().map(|&(page, title)| Entry::Page { page, title: title.into(), subpages: 0 }));
            FromFetcher::Contents(entries)
        };
        test.core.fetched(channel, index(&[(0x100, "Headlines"), (0x101, "Weather")])).unwrap();
        test.core.fetched(channel, index(&[(0x100, "Top stories")])).unwrap();
        let views: Vec<_> = test.sent().into_iter().filter_map(|message| if let ToInterface::Channel(view) = message { Some(view) } else { None }).collect();
        assert_eq!(views.len(), 2, "each index at once");
        let title = |view: &interface_core::channel::Channel, page: u16| view.pages[page_index(page).unwrap()].as_ref().map(|info| info.title.clone());
        assert_eq!(title(&views[0], 0x101).as_deref(), Some("Weather"));
        assert_eq!(title(&views[1], 0x100).as_deref(), Some("Top stories"), "retitled");
        assert_eq!(title(&views[1], 0x101), None, "no longer listed");
        assert_eq!(views[1].banks[0], "News");
    }

    /// DEC-1467: a live channel's pages and listing come on its connection
    /// alone.
    #[test]
    fn a_live_channels_pages_are_never_asked_for() {
        let mut test = Test::new("live-wanted");
        let channel = static_tab(&mut test, 0x100);
        test.core.fetched(channel, FromFetcher::Connecting(true)).unwrap();
        test.core.fetched(channel, FromFetcher::BundleSettled).unwrap();
        test.core.intent(protocol::ToCore::Opened(Some(PageKey { channel, page: 0x200 }))).unwrap();
        test.core.intent(protocol::ToCore::Visible(vec![PageKey { channel, page: 0x300 }])).unwrap();
        assert_eq!(test.core.controls.told_fetchers, []);
    }

    /// REQ-704: a declared `refresh` asks again for the page on screen at
    /// its interval and for nothing else, and the time follows the page
    /// open; no `refresh`, no asking; a live channel's does nothing. On the
    /// code before, nothing read it.
    #[test]
    fn a_refresh_asks_again_for_the_page_on_screen_alone() {
        use std::time::{Duration, Instant};
        const SECONDS: u32 = 5;
        let every = Duration::from_secs(u64::from(SECONDS));
        let mut test = Test::new("refresh");
        let channel = static_tab(&mut test, 0x100);
        let later = |by: Duration| Instant::now().checked_add(by).unwrap();
        test.core.refresh_tick(later(every * 2));
        assert_eq!(test.core.refresh_due_at(), None, "none declared");
        assert_eq!(test.core.controls.told_fetchers, [], "no asking");
        test.core.fetched(channel, FromFetcher::Refresh(SECONDS)).unwrap();
        let due = test.core.refresh_due_at().unwrap();
        assert!(due > Instant::now() && test.core.next_wake().is_some_and(|wake| wake <= due), "the core wakes for it");
        test.core.refresh_tick(due.checked_sub(Duration::from_millis(1)).unwrap());
        assert_eq!(test.core.controls.told_fetchers, [], "not yet");
        test.core.refresh_tick(later(every));
        assert_eq!(core::mem::take(&mut test.core.controls.told_fetchers), [(channel, ToFetcher::FetchPage { page: 0x100 })]);
        assert!(test.core.refresh_due_at().is_some(), "and again");
        test.core.intent(protocol::ToCore::Opened(None)).unwrap();
        assert_eq!(test.core.refresh_due_at(), None, "no page on screen");
        test.core.refresh_tick(later(every * 2));
        assert_eq!(test.core.controls.told_fetchers, []);

        let mut live = Test::new("refresh-live");
        let channel = static_tab(&mut live, 0x100);
        live.core.fetched(channel, FromFetcher::Connecting(true)).unwrap();
        live.core.fetched(channel, FromFetcher::Refresh(SECONDS)).unwrap();
        assert_eq!(live.core.refresh_due_at(), None, "a live channel's connection brings its changes");
    }

    /// REQ-704: the page on screen in every window is asked for again, each
    /// at its own channel's interval, and a tab behind another is not. On
    /// the code before, only the page open in the window last used was.
    #[test]
    fn a_refresh_asks_again_for_the_page_on_screen_in_every_window() {
        use std::time::{Duration, Instant};
        const SECONDS: u32 = 5;
        let later = Instant::now().checked_add(Duration::from_secs(u64::from(SECONDS) * 2)).unwrap();
        let mut test = Test::new("refresh-windows");
        let first = static_tab(&mut test, 0x100);
        test.core.session.open("https://behind.invalid/teletext/").unwrap();
        let behind = test.core.active_channel().unwrap();
        assert!(test.core.session.page_chosen(0x300));
        test.core.session.active = 0;
        test.core.session.new_window(false).unwrap();
        test.core.session.open("https://second.invalid/teletext/").unwrap();
        let second = test.core.active_channel().unwrap();
        assert!(test.core.session.page_chosen(0x200));
        for channel in [first, behind, second] {
            test.core.fetched(channel, FromFetcher::Refresh(SECONDS)).unwrap();
        }
        test.core.refresh_tick(later);
        let mut asked = core::mem::take(&mut test.core.controls.told_fetchers);
        asked.sort_by_key(|(channel, _)| *channel);
        assert_eq!(asked, [(first, ToFetcher::FetchPage { page: 0x100 }), (second, ToFetcher::FetchPage { page: 0x200 })]);
    }
}
