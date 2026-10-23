//! The channels' decoders and fetchers (decisions 22, 47 and 53): what each
//! decoder is told, which run, which sleep, a store grown, and what each
//! process is sent before the next event.

use std::io::Write;

use protocol::console::Level;
use protocol::decoder::{FIRST_STORE_PAGES, MAX_STORE_PAGES, ToDecoder, write_to_decoder};

use super::Core;
use super::processes::{Reach, end, start_decoder, start_fetcher, stop_decoder};

impl Core {
    pub(super) fn tell(&mut self, channel: u32, message: ToDecoder) {
        if let Some(decoder) = self.decoders.get_mut(&channel) {
            // A decoder that cannot be written to has stopped; its reader
            // reports that, and its pages stay as they were.
            let _ = write_to_decoder(&mut decoder.input, &message);
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
            if awake && !self.decoders.contains_key(id) {
                if let Some(problem) = problem {
                    self.log(Level::Warning, None, problem);
                }
                self.generation = self.generation.wrapping_add(1);
                let pages = self.store_pages.get(id).copied().unwrap_or(FIRST_STORE_PAGES);
                let mut decoder = start_decoder(self.broker()?, *id, self.generation, pages, self.events.clone())?;
                for message in self.sleep.kept(*id) {
                    let _ = write_to_decoder(&mut decoder.input, message);
                }
                self.note(Level::Info, Some(*id), &format!("decoder: confined: {}", decoder.child.report));
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
                self.note(Level::Info, Some(*id), &format!("fetcher: confined: {}", fetcher.child.report));
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
            let _ = write_to_decoder(&mut decoder.input, message);
        }
        if self.showing == Some(id) {
            let _ = write_to_decoder(&mut decoder.input, &ToDecoder::Showing(self.shown_pages.clone()));
        }
        self.decoders.insert(id, decoder);
        Ok(())
    }

    pub(super) fn stop_fetcher(&mut self, id: u32) {
        if let Some(mut fetcher) = self.fetchers.remove(&id) {
            // Closing its input ends a fetcher.
            drop(fetcher.child.stdin.take());
            drop(fetcher.input.take());
            end(&mut fetcher.child);
        }
    }

    /// A decoder that broke the protocol is closed; its channel's pages stay
    /// as they were, and its sounds go.
    pub(super) fn decoder_ended(&mut self, channel: u32, generation: u64, reason: Option<String>) {
        // A decoder put to sleep, or one before it, has nothing more to say.
        if self.decoders.get(&channel).map(|decoder| decoder.generation) != Some(generation) {
            return;
        }
        if let Some(reason) = reason {
            self.log(Level::Error, Some(channel), &format!("the decoder for channel {channel} stopped: {reason}"));
            // A decoder that cannot confine itself ends at once: the
            // version is not healthy (section 3.2 of the installation design).
            self.updates.helper_failed(&format!("the decoder stopped: {reason}"));
            let at = self.now();
            self.console.log(Some(channel), Level::Error, at, &format!("decoder: stopped: {reason}"));
        }
        if let Some(mut decoder) = self.decoders.remove(&channel) {
            end(&mut decoder.child);
        }
        let sends = self.hearing.channel_gone(channel);
        self.deliver(sends);
    }

    /// Sends what the last event produced to each process, before waiting
    /// for the next. A decoder that has stopped reading is replaced, with
    /// what it was given (decision 47); a fetcher's or the phone's lost
    /// input is theirs to notice; the interface is the browser.
    pub(super) fn flush(&mut self) -> Result<(), String> {
        let stuck: Vec<u32> = self.decoders.iter_mut().filter_map(|(&id, decoder)| decoder.input.flush().is_err().then_some(id)).collect();
        for id in stuck {
            self.log(Level::Error, None, &format!("the decoder for channel {id} stopped reading and is replaced"));
            self.put_to_sleep(id);
        }
        for fetcher in self.fetchers.values_mut() {
            if let Some(input) = fetcher.input.as_mut() {
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
        if self.decoders.len() < self.session.tab_count() {
            self.match_decoders()?;
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::super::processes::Decoder;
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
        core.decoders.insert(1, Decoder { child, input, generation: 1 });
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
}
