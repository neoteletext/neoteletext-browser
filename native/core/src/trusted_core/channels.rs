//! The channels' decoders and fetchers (decisions 22, 47 and 53): what each
//! decoder is told, which run, which sleep, a store grown, and what each
//! process is sent before the next event.

use std::io::Write;

use protocol::console::Level;
use protocol::decoder::{FIRST_STORE_PAGES, MAX_STORE_PAGES, ToDecoder};

use super::Core;
use super::processes::{Decoder, Fetcher, Reach, start_decoder, start_fetcher, stop_decoder};

/// The most times a channel's decoder is replaced, for stopping reading,
/// in [`DECODER_RESTART_WINDOW`] before the channel is given up (finding
/// F-30): a decoder is replaced at once, with what it was given (decision
/// 47), and an honest one stalls once, not again and again.
const MAX_DECODER_RESTARTS: usize = 5;
/// The time in which those replacements are counted: the sleep check's
/// minute (`SLEEP_CHECK`).
const DECODER_RESTART_WINDOW: std::time::Duration = super::SLEEP_CHECK;

impl Core {
    pub(super) fn tell(&mut self, channel: u32, message: ToDecoder) {
        if let Some(decoder) = self.decoders.get_mut(&channel) {
            // A decoder that cannot be written to has stopped; its reader
            // reports that, and its pages stay as they were.
            let _ = decoder.tell(&message);
        }
        // Kept whether or not the channel is awake: a sleeping channel's
        // pushed pages are there when it wakes.
        self.sleep.keep(channel, message);
    }

    /// Starts a decoder for every awake channel that has none, giving it
    /// what the channel's last decoder was given; puts to sleep the
    /// decoders of channels that may sleep (decision 47); starts a fetcher
    /// for every open web channel; and closes the decoders and fetchers of
    /// channels no tab shows any more.
    pub(super) fn match_decoders(&mut self) -> Result<(), String> {
        let now = std::time::Instant::now();
        let active = self.active_channel();
        if let Some(channel) = active {
            self.sleep.seen(channel, now);
        }
        // Every window's active channel is in view, and stays awake.
        for window in &self.session.others {
            if let Some(tab) = window.tabs.get(window.active) {
                self.sleep.seen(tab.channel.id, now);
            }
        }
        let channels: Vec<(u32, Option<String>, Option<String>)> =
            self.session.all_tabs().map(|tab| (tab.channel.id, tab.channel.problem.clone(), tab.channel.web.clone())).collect();
        for (id, problem, web) in &channels {
            let awake = self.sleep.awake(*id, active, now);
            if awake && !self.decoders.contains_key(id) && !self.given_up.contains(id) {
                // A restored file is read as its tab is first shown
                // (finding F-53).
                if let Some(problem) = problem.clone().or_else(|| self.session.read_unread(*id)) {
                    self.log(Level::Warning, None, &problem);
                }
                self.generation = self.generation.wrapping_add(1);
                let pages = self.store_pages.get(id).copied().unwrap_or(FIRST_STORE_PAGES);
                let mut decoder = start_decoder(self.broker()?, *id, self.generation, pages, self.events.clone())?;
                for message in self.sleep.kept(*id) {
                    let _ = decoder.tell(message);
                }
                self.note(Level::Info, Some(*id), &format!("decoder: confined: {}", decoder.report()));
                self.decoders.insert(*id, decoder);
            } else if !awake && self.decoders.contains_key(id) {
                self.put_to_sleep(*id);
            }
            // A web channel's pages come from its own fetcher, which runs
            // for as long as its tab is open, asleep or not: fetching never
            // depends on viewing (decision 22).
            if let Some(address) = web
                && !self.fetchers.contains_key(id)
            {
                self.generation = self.generation.wrapping_add(1);
                let reach = Reach::of(&self.allow_local, &self.settings);
                let fetcher = start_fetcher(self.broker()?, address, &reach, *id, self.generation, self.events.clone())?;
                self.note(Level::Info, Some(*id), &format!("fetcher: confined: {}", fetcher.report()));
                self.fetchers.insert(*id, fetcher);
            }
        }
        let unused: Vec<u32> = self.decoders.keys().chain(self.fetchers.keys()).copied().filter(|id| !channels.iter().any(|(used, ..)| used == id)).collect();
        for id in unused {
            self.put_to_sleep(id);
            self.stop_fetcher(id);
            self.sleep.forget(id);
            self.console.forget(id);
        }
        let open = |id: &u32| channels.iter().any(|(used, ..)| used == id);
        self.given_up.retain(open);
        self.restarts.retain(|id, _| open(id));
        Ok(())
    }

    /// Closes a channel's decoder, and forgets what it showed: its links,
    /// its typing and its sounds.
    fn put_to_sleep(&mut self, id: u32) {
        #[cfg(feature = "links")]
        self.links.channel_gone(id);
        #[cfg(feature = "interaction")]
        self.typing.channel_gone(id);
        let sends = self.hearing.channel_gone(id);
        self.deliver(sends);
        if let Some(decoder) = self.decoders.remove(&id) {
            stop_decoder(decoder);
        }
    }

    /// A decoder's store is full: it is started again with twice the room,
    /// up to the most a store holds, and given what the channel's decoder
    /// was given and the pages shown (decision 47). A channel whose data
    /// was too much to keep keeps the decoder it has.
    pub(super) fn grow_decoder(&mut self, id: u32) -> Result<(), String> {
        let pages = self.store_pages.get(&id).copied().unwrap_or(FIRST_STORE_PAGES);
        if pages >= MAX_STORE_PAGES || !self.sleep.has_everything(id) {
            return Ok(());
        }
        let pages = pages.saturating_mul(2).min(MAX_STORE_PAGES);
        self.store_pages.insert(id, pages);
        if let Some(decoder) = self.decoders.remove(&id) {
            stop_decoder(decoder);
        }
        self.generation = self.generation.wrapping_add(1);
        let mut decoder = start_decoder(self.broker()?, id, self.generation, pages, self.events.clone())?;
        for message in self.sleep.kept(id) {
            let _ = decoder.tell(message);
        }
        if self.showing == Some(id) {
            let _ = decoder.tell(&ToDecoder::Showing(self.shown_pages.clone()));
        }
        self.decoders.insert(id, decoder);
        Ok(())
    }

    pub(super) fn stop_fetcher(&mut self, id: u32) {
        if let Some(fetcher) = self.fetchers.remove(&id) {
            fetcher.stop();
        }
    }

    /// A decoder that broke the protocol is closed; its channel's pages stay
    /// as they were, and its sounds go.
    pub(super) fn decoder_ended(&mut self, channel: u32, generation: u64, reason: Option<String>) {
        // A decoder put to sleep, or one before it, has nothing more to say.
        if self.decoders.get(&channel).map(Decoder::generation) != Some(generation) {
            return;
        }
        let heard = self.decoders.get(&channel).is_some_and(Decoder::heard);
        if let Some(reason) = &reason {
            self.log(Level::Error, Some(channel), &format!("the decoder for channel {channel} stopped: {reason}"));
            let at = self.now();
            self.console.log(Some(channel), Level::Error, at, &format!("decoder: stopped: {reason}"));
        } else if !heard {
            self.log(Level::Error, Some(channel), &format!("the decoder for channel {channel} stopped before it first spoke"));
        }
        // A decoder that breaks its protocol, or ends before it first
        // speaks, as one that cannot confine itself does, even with its
        // pipe closed cleanly, makes the version unhealthy (section 3.2 of
        // the installation design, finding F-12). The core's own ends of a
        // decoder were discarded above by its generation.
        if reason.is_some() || !heard {
            self.updates.helper_failed(&format!(
                "the decoder stopped{}",
                reason.map_or_else(|| String::from(" before it first spoke"), |reason| format!(": {reason}"))
            ));
        }
        if let Some(decoder) = self.decoders.remove(&channel) {
            decoder.end();
        }
        let sends = self.hearing.channel_gone(channel);
        self.deliver(sends);
    }

    /// The decoder of `channel` at `generation` said something.
    pub(super) fn decoder_heard(&mut self, channel: u32, generation: u64) {
        if let Some(decoder) = self.decoders.get_mut(&channel).filter(|decoder| decoder.generation() == generation) {
            decoder.hear();
        }
    }

    /// The fetcher of `channel` at `generation` said something.
    pub(super) fn fetcher_heard(&mut self, channel: u32, generation: u64) {
        if let Some(fetcher) = self.fetchers.get_mut(&channel).filter(|fetcher| fetcher.generation() == generation) {
            fetcher.hear();
        }
    }

    /// A fetcher ended. One the core did not end, that ended before it
    /// first spoke, as one that cannot confine itself does, fails the
    /// version (finding F-12); one that ends after speaking is not
    /// counted, so a hostile server that crashes it cannot force a
    /// rollback.
    pub(super) fn fetcher_ended(&mut self, channel: u32, generation: u64, reason: Option<String>) {
        if self.fetchers.get(&channel).map(Fetcher::generation) == Some(generation) {
            let stopped = format!("the fetcher for channel {channel} stopped{}", reason.map(|reason| format!(": {reason}")).unwrap_or_default());
            self.log(Level::Error, None, &stopped);
            if let Some(fetcher) = self.fetchers.remove(&channel) {
                if !fetcher.heard() {
                    self.updates.helper_failed(&format!("{stopped} before it first spoke"));
                }
                fetcher.reap();
            }
        }
    }

    /// Sends what the last event produced to each process, before waiting
    /// for the next. A decoder that has stopped reading is replaced, with
    /// what it was given (decision 47); a fetcher's or the phone's lost
    /// input is theirs to notice; the interface is the browser.
    pub(super) fn flush(&mut self) -> Result<(), String> {
        let stuck: Vec<u32> = self.decoders.iter_mut().filter_map(|(&id, decoder)| decoder.flush().is_err().then_some(id)).collect();
        let now = std::time::Instant::now();
        for id in stuck {
            // A decoder its channel's data keeps overwhelming is replaced
            // a few times, then the channel is given up until it is
            // reloaded, which makes a new channel (finding F-30).
            let restarts = self.restarts.entry(id).or_default();
            restarts.retain(|at| now.saturating_duration_since(*at) < DECODER_RESTART_WINDOW);
            restarts.push_back(now);
            if restarts.len() > MAX_DECODER_RESTARTS {
                self.given_up.insert(id);
                self.log(
                    Level::Error,
                    Some(id),
                    &format!("the decoder for channel {id} stopped reading too often and the channel is given up until it is reloaded"),
                );
            } else {
                self.log(Level::Error, None, &format!("the decoder for channel {id} stopped reading and is replaced"));
            }
            self.put_to_sleep(id);
        }
        for fetcher in self.fetchers.values_mut() {
            if let Some(input) = fetcher.input() {
                let _ = input.flush();
            }
        }
        #[cfg(feature = "phone-keyboard")]
        if let Some((_, input)) = self.phone.as_mut() {
            let _ = input.flush();
        }
        if let Some((_, input)) = self.sound.as_mut()
            && input.flush().is_err()
        {
            self.log(Level::Error, None, "no sound: the sound helper stopped reading");
            self.sound = None;
        }
        self.to_interface.flush().map_err(|error| format!("cannot write to the interface: {error}"))?;
        // A replaced decoder starts again at once if its tab is awake.
        if self.decoders.len().saturating_add(self.given_up.len()) < self.session.tab_count() {
            self.match_decoders()?;
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
#[allow(clippy::unwrap_used, clippy::arithmetic_side_effects, reason = "a test")]
mod tests {
    use super::super::Core;
    use super::super::test_support::test_core;
    use super::*;
    use confine::Role;
    use confine::broker::Helper;
    use neoteletext::outbox::Outbox;
    use protocol::decoder::MAX_CHUNK;
    use std::ffi::OsString;
    use std::path::PathBuf;
    use std::time::Duration;

    /// Decision 53: a decoder that stops reading is given up once 16 MB
    /// wait for it, and killed; the trusted core's thread never waits on
    /// it. The decoder here is `sleep`, which reads nothing. Before this,
    /// the core blocked on the pipe and every tab with it.
    #[cfg(unix)]
    #[test]
    fn a_decoder_that_stops_reading_is_killed_and_forgotten() {
        let mut core = test_core();
        let program = PathBuf::from("/bin/sleep");
        let arguments = [OsString::from("60")];
        let mut child = Helper::local(
            confine::spawn(&confine::Spawn {
                program: &program,
                arguments: &arguments,
                role: Role::Decoder,
                environment: &[],
                local_network: false,
                writable: None,
            })
            .unwrap(),
        );
        let input = Outbox::new(child.stdin.take().unwrap());
        core.decoders.insert(1, Decoder::new(child, input, 1));
        let started = std::time::Instant::now();
        let chunk = vec![0_u8; MAX_CHUNK - 1];
        let chunks = neoteletext::outbox::MAX_QUEUED_BYTES / chunk.len() + 2;
        for _ in 0..chunks {
            core.tell(1, ToDecoder::Data(chunk.clone()));
        }
        core.flush().unwrap();
        assert!(!core.decoders.contains_key(&1), "given up");
        assert!(started.elapsed() < Duration::from_secs(5), "never waited on it: {:?}", started.elapsed());
    }

    /// A helper of a test, confined as `role`: `program` with `arguments`.
    fn stand_in(program: &str, arguments: &[&str], role: Role) -> Helper {
        let program = PathBuf::from(program);
        let arguments: Vec<OsString> = arguments.iter().map(OsString::from).collect();
        Helper::local(
            confine::spawn(&confine::Spawn { program: &program, arguments: &arguments, role, environment: &[], local_network: false, writable: None }).unwrap(),
        )
    }

    /// Whether the version is healthy once its window has drawn and
    /// settled.
    fn healthy_once_settled(core: &mut Core) -> bool {
        core.updates.drawn();
        core.updates.health.healthy(std::time::Instant::now() + super::super::updates::HEALTH_SETTLE)
    }

    /// Finding F-12: a decoder that ends before it first speaks, as one
    /// that cannot confine itself does, marks the version unhealthy, even
    /// when its pipe closed cleanly; one that ends after speaking, as a
    /// hostile channel's could, is not counted.
    #[test]
    fn a_decoder_that_ends_before_it_speaks_fails_the_version() {
        let mut core = test_core();
        let mut child = stand_in("/usr/bin/false", &[], Role::Decoder);
        let input = Outbox::new(child.stdin.take().unwrap());
        core.decoders.insert(1, Decoder::new(child, input, 1));
        core.decoder_ended(1, 1, None);
        assert!(!healthy_once_settled(&mut core), "a decoder that never spoke failed the version");

        let mut core = test_core();
        let mut child = stand_in("/usr/bin/false", &[], Role::Decoder);
        let input = Outbox::new(child.stdin.take().unwrap());
        core.decoders.insert(1, Decoder::new(child, input, 1));
        core.decoder_heard(1, 1);
        core.decoder_ended(1, 1, None);
        assert!(healthy_once_settled(&mut core), "a decoder that spoke first is not counted");
    }

    /// Finding F-30: a channel whose decoder stops reading more than
    /// `MAX_DECODER_RESTARTS` times in `DECODER_RESTART_WINDOW` is given up
    /// until it is reloaded: no further decoder is started for it. Before
    /// this, each was replaced at once, for as long as its data came.
    #[test]
    fn a_decoder_replaced_too_often_leaves_its_channel_failed() {
        let mut core = test_core();
        core.session.open("/nowhere/news.t42").unwrap();
        let channel = core.session.tabs.last().unwrap().channel.id;
        let chunk = vec![0_u8; MAX_CHUNK - 1];
        let chunks = neoteletext::outbox::MAX_QUEUED_BYTES / chunk.len() + 2;
        for generation in 1..=6 {
            let mut child = stand_in("/bin/sleep", &["60"], Role::Decoder);
            let input = Outbox::new(child.stdin.take().unwrap());
            let mut decoder = Decoder::new(child, input, generation);
            decoder.hear();
            core.decoders.insert(channel, decoder);
            for _ in 0..chunks {
                core.tell(channel, ToDecoder::Data(chunk.clone()));
            }
            // With no broker, the restart this tries fails, as it may.
            let _ = core.flush();
            assert!(!core.decoders.contains_key(&channel), "replacement {generation} given up");
        }
        // With no broker, starting a decoder fails: none is tried now.
        assert_eq!(core.match_decoders(), Ok(()), "no decoder is started for a channel given up");
    }

    /// Finding F-12: a fetcher that ends before it first speaks, unasked,
    /// marks the version unhealthy.
    #[test]
    fn a_fetcher_that_ends_before_it_speaks_fails_the_version() {
        let mut core = test_core();
        let child = stand_in("/usr/bin/false", &[], Role::Network);
        core.fetchers.insert(1, Fetcher::new(child, 1, None));
        core.fetcher_ended(1, 1, None);
        assert!(!core.fetchers.contains_key(&1));
        assert!(!healthy_once_settled(&mut core), "a fetcher that never spoke failed the version");
    }
}
