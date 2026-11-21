//! The channels' decoders and fetchers (decisions 22, 47 and 53): what each
//! decoder is told, which run, which sleep, a store grown, and what each
//! process is sent before the next event.

use std::io::Write;

use interface_core::browser::Failed;
use protocol::ToInterface;
use protocol::console::Level;
use protocol::decoder::{FIRST_STORE_PAGES, MAX_STORE_PAGES, ToDecoder};

use super::Core;
use super::processes::{Decoder, Fetcher, FetcherKey, Reach, start_decoder, start_fetcher, stop_decoder};

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
        // A decoder forgets the frames of pages it stops showing, and sends
        // them whole when shown again: so does this (ANIM-5).
        if let ToDecoder::Showing(pages) = &message {
            self.decoded.retain(|key, _| key.channel != channel || pages.contains(&key.page));
        }
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
        let channels: Vec<(u32, Option<String>, Option<String>, bool)> =
            self.session.all_tabs().map(|tab| (tab.channel.id, tab.channel.problem.clone(), tab.channel.web.clone(), tab.private)).collect();
        // A channel no tab holds any more is ended first: a helper that then
        // cannot start, as when the broker refuses one, leaves none running
        // that no tab holds (REQ-683).
        let unused: Vec<u32> = self.decoders.keys().chain(self.fetchers.keys()).copied().filter(|id| !channels.iter().any(|(used, ..)| used == id)).collect();
        for id in unused {
            // The tab left the channel's pages: told with SELECT of page 0
            // before its connection closes (REQ-INT-140), as far as the
            // fetcher sends it before it ends.
            #[cfg(feature = "interaction")]
            {
                let sends = self.typing.select(id, 0, neoteletext_core::limits::SUBCODE_ANY, "", self.now_us());
                self.deliver_typing(sends)?;
            }
            self.put_to_sleep(id, true);
            self.stop_fetcher(id);
            self.sleep.forget(id);
            self.console.forget(id);
            self.invalid.remove(&id);
            self.naplps_gates.remove(&id);
        }
        let open = |id: &u32| channels.iter().any(|(used, ..)| used == id);
        self.given_up.retain(open);
        self.restarts.retain(|id, _| open(id));
        let mut started = false;
        for (id, problem, web, private) in &channels {
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
                // A new decoder is told that its sound does not reach the
                // viewer, when it does not, before any page it is given can
                // play one (page-rules.md RULE-27).
                if !self.sound_heard(*id) {
                    let _ = decoder.tell(&ToDecoder::SoundHeard(false));
                }
                for message in self.sleep.kept(*id) {
                    let _ = decoder.tell(message);
                }
                self.note(Level::Info, Some(*id), &format!("decoder: confined: {}", decoder.report()));
                // A new decoder is told which of its pages is the open page
                // of the active tab, whose table it evaluates (RULE-58).
                #[cfg(feature = "interaction")]
                if let Some(page) = self.typing.rules_open(*id) {
                    let _ = decoder.tell(&ToDecoder::RulesPage(Some(page)));
                }
                // And how the viewer has drawings painted, when not as a
                // decoder begins (animation.md ANIM-15).
                if !self.settings.paint_gradually {
                    let _ = decoder.tell(&ToDecoder::PaintGradually(false));
                }
                // And whether the platform asks for reduced motion, when it
                // does (renderer parity PAR-33).
                if self.reduced_motion {
                    let _ = decoder.tell(&ToDecoder::ReducedMotion(true));
                }
                // And the fonts the viewer chose for pictures, when not
                // those compiled in (renderer parity PAR-35).
                for message in self.fonts.messages() {
                    let _ = decoder.tell(&message);
                }
                self.rules_decoder_gone(*id);
                // A new decoder has sent no frame: a rectangle is laid only
                // over one it sent (ANIM-5).
                self.decoded.retain(|key, _| key.channel != *id);
                self.decoders.insert(*id, decoder);
                // And the interface's frames, to paint and tick just
                // before a composition (page-rules.md RULE-98 item 2).
                self.tell_frame_phase(*id);
                self.rewind_source(*id);
                started = true;
            } else if !awake && self.decoders.contains_key(id) {
                self.put_to_sleep(*id, false);
            }
            // A web channel's pages come from its tab's own place in the
            // fetcher process of its channel and jar (REQ-6980), which the
            // tab keeps for as long as it is open, asleep or not, and
            // which fetches a static channel's files only as they are
            // wanted (REQ-703).
            if let Some(address) = web
                && !self.fetchers.contains_key(id)
                && !self.given_up.contains(id)
            {
                self.generation = self.generation.wrapping_add(1);
                let key = FetcherKey::of(address, *private, &Reach::of(&self.allow_local, &self.settings));
                // A tab of a channel and jar another tab shows joins that
                // tab's process; the first starts one.
                let fetcher = match self.fetchers.values().find(|fetcher| fetcher.serves(&key)) {
                    Some(serving) => serving.join(*id, self.generation),
                    None => start_fetcher(self.broker()?, key, *id, self.generation, self.events.clone())?,
                };
                self.note(Level::Info, Some(*id), &format!("fetcher: confined: {}", fetcher.report()));
                self.fetchers.insert(*id, fetcher);
            }
        }
        if started {
            self.load();
        }
        Ok(())
    }

    /// A new decoder has the channel's file to read from its start.
    fn rewind_source(&mut self, id: u32) {
        for tab in self.session.all_tabs_mut().filter(|tab| tab.channel.id == id) {
            tab.channel.source.rewind();
        }
    }

    /// Closes a channel's decoder, and forgets what it showed: its links,
    /// its typing and its sounds. A channel whose tab stays (`gone` false)
    /// keeps its tab's selection (DEC-6358, fault 9).
    #[cfg_attr(not(feature = "interaction"), expect(unused_variables, reason = "the selection is input's, compiled out"))]
    fn put_to_sleep(&mut self, id: u32, gone: bool) {
        #[cfg(feature = "links")]
        self.links.channel_gone(id);
        #[cfg(feature = "interaction")]
        if gone {
            self.typing.channel_gone(id);
        } else {
            self.typing.channel_asleep(id);
        }
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
        self.rewind_source(id);
        self.load();
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
        let broke = reason.is_some();
        if let Some(reason) = &reason {
            self.log(Level::Error, Some(channel), &format!("the decoder for channel {channel} stopped: {reason}"));
            let at = self.now();
            self.console.log(Some(channel), Level::Error, at, &format!("decoder: stopped: {reason}"));
        } else if !heard {
            self.log(Level::Error, Some(channel), &format!("the decoder for channel {channel} stopped before it said it was confined"));
        }
        // A decoder that breaks its protocol, or ends before it says it is
        // confined, its first message, as one that cannot confine itself
        // does, even with its pipe closed cleanly, makes the version
        // unhealthy (section 3.2 of the installation design, finding
        // F-12). One that ends after that, before its channel gave it a
        // page, is not counted. The core's own ends of a decoder were
        // discarded above by its generation.
        if reason.is_some() || !heard {
            self.updates.helper_failed(&format!(
                "the decoder stopped{}",
                reason.map_or_else(|| String::from(" before it said it was confined"), |reason| format!(": {reason}"))
            ));
        }
        if let Some(decoder) = self.decoders.remove(&channel) {
            decoder.end();
        }
        self.rules_decoder_gone(channel);
        let sends = self.hearing.channel_gone(channel);
        self.deliver(sends);
        if broke {
            self.helper_broke(channel, Failed::Decoder);
        }
    }

    /// A helper of `channel` broke its protocol and was closed: it may be
    /// compromised, and a server can make one do so again, so none starts
    /// for the channel until its tab is reloaded, and the tab says why
    /// (REQ-683, finding F-30).
    fn helper_broke(&mut self, channel: u32, failed: Failed) {
        self.given_up.insert(channel);
        if let Some(tab) = self.session.all_tabs_mut().find(|tab| tab.channel.id == channel) {
            tab.channel.failed = Some((failed, None));
        }
        // An interface that cannot be written to has stopped; its reader
        // reports that.
        let _ = self.in_window_of(channel, |core| {
            if core.active_channel() == Some(channel) {
                core.send(&ToInterface::Strip(core.strip()))?;
            }
            Ok(())
        });
    }

    /// A fetcher ended. One the core did not end, that ended before it said
    /// it was confined, its first message, as one that cannot confine
    /// itself does, fails the version (finding F-12); one that ends after
    /// that is not counted, even before its channel answered, so a hostile
    /// server that crashes it cannot force a rollback.
    pub(super) fn fetcher_ended(&mut self, channel: u32, generation: u64, reason: Option<String>) {
        if self.fetchers.get(&channel).map(Fetcher::generation) == Some(generation) {
            let broke = reason.is_some();
            let stopped = format!("the fetcher for channel {channel} stopped{}", reason.map(|reason| format!(": {reason}")).unwrap_or_default());
            self.log(Level::Error, Some(channel), &stopped);
            if let Some(fetcher) = self.fetchers.remove(&channel) {
                if fetcher.ended_unheard() {
                    self.updates.helper_failed(&format!("{stopped} before it said it was confined"));
                }
                fetcher.end();
            }
            if broke {
                self.helper_broke(channel, Failed::Fetcher);
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
            self.put_to_sleep(id, false);
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
            self.tell_sound_heard_all();
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
    use super::super::test_support::{healthy_once_settled, stand_in, test_core};
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
                udp_port: None,
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

    /// Finding F-12: a decoder that ends before it says it is confined, as
    /// one that cannot confine itself does, marks the version unhealthy,
    /// even when its pipe closed cleanly; one that ends after that, as a
    /// hostile channel's could, is not counted.
    #[test]
    fn a_decoder_that_ends_before_it_is_confined_fails_the_version() {
        let mut core = test_core();
        let mut child = stand_in("/usr/bin/false", &[], Role::Decoder);
        let input = Outbox::new(child.stdin.take().unwrap());
        core.decoders.insert(1, Decoder::new(child, input, 1));
        core.decoder_ended(1, 1, None);
        assert!(!healthy_once_settled(&mut core), "a decoder that never said it was confined failed the version");

        let mut core = test_core();
        let mut child = stand_in("/usr/bin/false", &[], Role::Decoder);
        let input = Outbox::new(child.stdin.take().unwrap());
        core.decoders.insert(1, Decoder::new(child, input, 1));
        core.decoded(1, protocol::decoder::FromDecoder::Confined).unwrap();
        core.decoder_ended(1, 1, None);
        assert!(healthy_once_settled(&mut core), "a decoder that said it was confined is not counted");
    }

    /// The engine's REQ-AUD-017: a port publishes its audio receiver's
    /// figures. What a channel's decoder reports of its receiver is shown
    /// in that channel's Log tab.
    #[test]
    fn a_decoders_audio_receiver_is_published_in_its_log() {
        let mut core = test_core();
        core.console.set_open(true);
        let receiver = protocol::decoder::AudioReceiver { store_bytes: 4_194_304, objects: 256, records_in_flight: 8 };
        core.decoded(1, protocol::decoder::FromDecoder::AudioReceiver(receiver)).unwrap();
        let lines: Vec<String> = core.console.updates(Some(1)).into_iter().flat_map(|update| update.entries).map(|entry| entry.text).collect();
        assert!(
            lines.contains(&"decoder: audio receiver: a store of 4,194,304 bytes, 256 objects, 8 records in flight".to_owned()),
            "the receiver's figures in the channel's log: {lines:?}"
        );
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

    /// Finding F-12: a fetcher that ends before it says it is confined,
    /// unasked, marks the version unhealthy.
    #[test]
    fn a_fetcher_that_ends_before_it_is_confined_fails_the_version() {
        let mut core = test_core();
        let child = stand_in("/usr/bin/false", &[], Role::Network);
        core.fetchers.insert(1, Fetcher::new(child, 1, None));
        core.fetcher_ended(1, 1, None);
        assert!(!core.fetchers.contains_key(&1));
        assert!(!healthy_once_settled(&mut core), "a fetcher that never said it was confined failed the version");
    }

    // REQ-683, REQ-675: a channel no tab holds any more has its helpers ended
    // even when a helper for another channel cannot be started, as when the
    // broker refuses one. Before, the first refusal returned from
    // match_decoders before the unused channels were reached, and a tab that
    // went Back to another channel left the old channel's fetcher running.
    #[test]
    fn a_helper_that_cannot_start_leaves_no_unused_helper_running() {
        use super::super::processes::Fetcher;
        use super::super::test_support::sleeping_helper;
        let mut core = test_core();
        core.session.open("https://old.invalid/teletext/").unwrap();
        let old = core.session.tabs[0].channel.id;
        let (child, _output) = sleeping_helper(Role::Network);
        core.fetchers.insert(old, Fetcher::new(child, 1, None));
        core.session.navigate("https://new.invalid/teletext/", false).unwrap();
        assert_ne!(core.session.tabs[0].channel.id, old, "another channel");
        assert!(core.match_decoders().is_err(), "this core has no broker to start the new channel's helpers");
        assert!(!core.fetchers.contains_key(&old), "the old channel's fetcher is ended all the same");
    }

    /// The fetcher processes the stand-in broker was asked for, each as
    /// its arguments.
    fn fetcher_processes(stand_in: &confine::broker::stand_in::StandIn) -> Vec<Vec<String>> {
        stand_in.started().into_iter().filter(|helper| helper.program == confine::broker::Program::Fetcher).map(|helper| helper.arguments).collect()
    }

    /// The fetcher processes the core has asked the broker to kill.
    fn fetchers_killed(stand_in: &confine::broker::stand_in::StandIn, killed: &mut Vec<u32>) -> usize {
        use confine::broker::Request;
        killed.extend(stand_in.take_requests().iter().filter_map(|request| if let Request::Kill(id) = request { Some(*id) } else { None }));
        let fetchers: Vec<u32> =
            stand_in.started().into_iter().filter(|helper| helper.program == confine::broker::Program::Fetcher).map(|helper| helper.id).collect();
        killed.iter().filter(|id| fetchers.contains(id)).count()
    }

    // REQ-6980, DEC-6979 items 1 and 6: one fetcher process serves every
    // tab of one channel in one jar. A second ordinary tab of a channel
    // joins the first's process; a private tab of the same channel has a
    // process of its own, and so has a tab of another channel; each tab
    // keeps a place of its own; and the process is ended only when its last
    // tab has left. Before, every tab started a process, and closing a tab
    // ended one.
    #[test]
    fn tabs_of_one_channel_and_jar_share_one_fetcher_process() {
        use super::super::test_support::core_on;
        const CHANNEL: &str = "https://a.invalid/teletext/";
        const OTHER: &str = "https://b.invalid/teletext/";
        let (mut core, stand_in) = core_on(CHANNEL);
        core.session.open(CHANNEL).unwrap();
        core.session.open_private(CHANNEL).unwrap();
        core.session.open(OTHER).unwrap();
        core.match_decoders().unwrap();
        let started = fetcher_processes(&stand_in);
        assert_eq!(
            started,
            [vec![CHANNEL.to_owned()], vec![CHANNEL.to_owned()], vec![OTHER.to_owned()]],
            "the channel's ordinary jar, its private jar, the other channel"
        );
        assert_eq!(core.fetchers.len(), 4, "each tab has its place");
        let channels: Vec<u32> = core.session.tabs.iter().map(|tab| tab.channel.id).collect();
        let key = |address: &str, private: bool| FetcherKey::of(address, private, &Reach::of(&[], &core.settings));
        assert!(core.fetchers[&channels[0]].serves(&key(CHANNEL, false)) && core.fetchers[&channels[1]].serves(&key(CHANNEL, false)));
        assert!(!core.fetchers[&channels[2]].serves(&key(CHANNEL, false)), "REQ-881: a private tab never shares an ordinary tab's process");
        assert!(core.fetchers[&channels[2]].serves(&key(CHANNEL, true)));
        assert!(!core.fetchers[&channels[3]].serves(&key(CHANNEL, false)), "REQ-712: nor does another channel's tab");

        // A change of the setting reaches the tabs opened after it
        // (REQ-6982): such a tab joins no process started before it.
        core.settings.share_connections = false;
        core.session.open(CHANNEL).unwrap();
        core.match_decoders().unwrap();
        let started = fetcher_processes(&stand_in);
        assert_eq!(started.len(), 4, "a process of its own");
        assert_eq!(started[3], [CHANNEL, "--no-http3"], "with sharing off");
        core.settings.share_connections = true;

        // The first tab leaves: its process still serves the second.
        let mut killed = Vec::new();
        core.session.close(0).unwrap();
        core.match_decoders().unwrap();
        assert!(!core.fetchers.contains_key(&channels[0]) && core.fetchers.contains_key(&channels[1]));
        std::thread::sleep(protocol::fetcher::FETCHER_END_WAIT * 2);
        assert_eq!(fetchers_killed(&stand_in, &mut killed), 0, "no process ended while a tab of it stays");
        // A third tab of the channel joins the process still running.
        core.session.open(CHANNEL).unwrap();
        core.match_decoders().unwrap();
        assert_eq!(fetcher_processes(&stand_in).len(), 4, "none started");
        // Its last tabs leave: the process is ended.
        let third = core.session.tabs.last().unwrap().channel.id;
        for channel in [channels[1], third] {
            let index = core.session.tabs.iter().position(|tab| tab.channel.id == channel).unwrap();
            core.session.close(index).unwrap();
        }
        core.match_decoders().unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while fetchers_killed(&stand_in, &mut killed) == 0 {
            assert!(std::time::Instant::now() < deadline, "the process was never ended");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(fetchers_killed(&stand_in, &mut killed), 1, "that process and no other");
    }
}
