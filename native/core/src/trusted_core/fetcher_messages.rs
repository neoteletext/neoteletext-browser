//! What a fetcher says, untrusted (`architecture.md` section 6): a
//! channel's contents and pages, its notices, a refused certificate
//! (`certificate-errors.md`), and the console's view of it all.

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
                    let pushed = matches!(origin, Origin::PushedPackets | Origin::PushedPage);
                    self.tell(channel, ToDecoder::Whole { packets, pushed, bytes });
                }
            }
            FromFetcher::Notice(text) => self.log(Level::Warning, Some(channel), &format!("channel {channel}: {}", clean(&text, MAX_NOTICE))),
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
                let sends = self.typing.input(channel, open, secure, self.now_us());
                self.deliver_typing(sends)?;
            }
            #[cfg(feature = "interaction")]
            FromFetcher::RoundTrip { microseconds } => {
                let sends = self.typing.round_trip(channel, microseconds);
                self.deliver_typing(sends)?;
            }
            FromFetcher::SubPages { page, part } => self.catalogue_part(channel, page, part)?,
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
                self.catalogue_arrived(interface_core::canvas::PageKey { channel, page })
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
                let pushed = matches!(origin, Origin::PushedPackets | Origin::PushedPage);
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
}
