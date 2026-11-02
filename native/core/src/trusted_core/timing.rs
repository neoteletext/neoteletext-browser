//! The trusted core's timed work (decisions 22, 46 and 53): the loading
//! pace, the next wake, and what falls due.

use protocol::decoder::{MAX_CHUNK, ToDecoder};

use super::{Core, LOAD_INTERVAL, MOST_FILE_QUEUED, SLEEP_CHECK};
use crate::source::Next;

impl Core {
    /// Hands every running decoder as much of its channel's file as may be
    /// queued for it, whether or not it is on screen (decision 22), so a
    /// file the viewer opens is in its store at once (DEC-1762). What is
    /// handed over is not kept: the file is, and a new decoder is given it
    /// from its start.
    pub(super) fn load(&mut self) {
        for tab in self.session.all_tabs_mut() {
            let Some(decoder) = self.decoders.get_mut(&tab.channel.id) else { continue };
            let mut room = MOST_FILE_QUEUED.saturating_sub(decoder.queued());
            while room > 0 {
                let message = match tab.channel.source.next(MAX_CHUNK) {
                    Next::Data(bytes) => {
                        room = room.saturating_sub(bytes.len());
                        ToDecoder::Data(bytes.to_vec())
                    }
                    Next::End => ToDecoder::End,
                    Next::Done => break,
                };
                // A decoder that cannot be written to has stopped; its
                // reader reports that.
                let _ = decoder.tell(&message);
            }
        }
    }

    /// Whether anything waits on `LOAD_INTERVAL`: a channel's file not yet
    /// all queued, a channel being waited on to take keys, or the console
    /// open to be sent what comes.
    fn loading_wanted(&self) -> bool {
        let feeding = self.session.all_tabs().any(|tab| self.decoders.contains_key(&tab.channel.id) && tab.channel.source.has_more());
        #[cfg(feature = "interaction")]
        let typing = self.typing.waiting();
        #[cfg(not(feature = "interaction"))]
        let typing = false;
        feeding || typing || self.console.is_open()
    }

    /// When the trusted core next has timed work, if it has any: it waits
    /// for an event until then, and with none, for an event alone
    /// (decision 46).
    pub(super) fn next_wake(&self) -> Option<std::time::Instant> {
        let mut next = Some(self.next_sleep_check);
        if self.loading_wanted() {
            next = next.map(|at| at.min(self.next_load));
        }
        if let Some((_, until)) = &self.notice {
            next = next.map(|at| at.min(*until));
        }
        if let Some((_, _, still)) = self.window_moved {
            next = next.map(|at| at.min(still));
        }
        if let Some(due) = self.cookies.due {
            next = next.map(|at| at.min(due));
        }
        if let Some(due) = self.updates.due() {
            next = next.map(|at| at.min(due));
        }
        if let Some(due) = self.frames_due_at() {
            next = next.map(|at| at.min(due));
        }
        if let Some(due) = self.invalid_due_at() {
            next = next.map(|at| at.min(due));
        }
        if let Some(due) = self.redirect_due_at() {
            next = next.map(|at| at.min(due));
        }
        if let Some(due) = self.refresh_due_at() {
            next = next.map(|at| at.min(due));
        }
        #[cfg(feature = "interaction")]
        if let Some(due) = self.fill_due_at() {
            next = next.map(|at| at.min(due));
        }
        #[cfg(feature = "interaction")]
        if let Some(due) = self.moves_due_at() {
            next = next.map(|at| at.min(due));
        }
        #[cfg(feature = "phone-keyboard")]
        if let Some(due) = self.phone_ending_until() {
            next = next.map(|at| at.min(due));
        }
        next
    }

    /// The timed work now due.
    pub(super) fn tick(&mut self) -> Result<(), String> {
        let now = std::time::Instant::now();
        if now >= self.next_load && self.loading_wanted() {
            self.next_load = now.checked_add(LOAD_INTERVAL).unwrap_or(now);
            self.load();
            self.typing_tick()?;
            self.send_console()?;
        }
        self.notice_tick()?;
        self.refresh_tick(now);
        if now >= self.next_sleep_check {
            self.next_sleep_check = now.checked_add(SLEEP_CHECK).unwrap_or(now);
            self.match_decoders()?;
        }
        if self.window_moved.is_some_and(|(_, _, still)| now >= still) {
            self.keep_window();
        }
        if self.cookies.due.is_some_and(|due| now >= due) {
            self.keep_cookies();
        }
        if self.updates.due().is_some_and(|due| now >= due) {
            self.updates.tick(self.broker.as_deref(), now);
        }
        if self.frames_due_at().is_some_and(|due| now >= due) {
            self.send_due_frames(now)?;
        }
        if self.invalid_due_at().is_some_and(|due| now >= due) {
            self.invalid_tick(now);
        }
        if self.redirect_due_at().is_some_and(|due| now >= due) {
            self.finish_redirects(now)?;
        }
        #[cfg(feature = "interaction")]
        self.fill_tick(now)?;
        #[cfg(feature = "interaction")]
        if self.moves_due_at().is_some_and(|due| now >= due) {
            self.send_due_moves()?;
        }
        #[cfg(feature = "phone-keyboard")]
        self.phone_tick(now);
        Ok(())
    }

    /// Lets time pass for typing: a channel that has not answered HELLO
    /// in time takes no input (DOC-1921).
    #[cfg_attr(not(feature = "interaction"), allow(clippy::unnecessary_wraps, clippy::unused_self, reason = "only typing has a time"))]
    fn typing_tick(&mut self) -> Result<(), String> {
        #[cfg(feature = "interaction")]
        {
            let sends = self.typing.tick(self.now_us());
            self.deliver_typing(sends)?;
        }
        Ok(())
    }
}

#[cfg(all(test, unix))]
mod tests {
    #![allow(clippy::arithmetic_side_effects, clippy::unwrap_used, reason = "a test")]
    use super::super::test_support::{decoder_watched, test_core};
    use super::*;
    use crate::session::Session;
    use crate::source::Source;
    use neoteletext::bookmarks::Bookmarks;
    use neoteletext_core::packet::Packet;
    use neoteletext_formats::Item;
    use neoteletext_formats::detect::{AutoConfig, AutoReader, Format};
    use neoteletext_formats::item::Reader as _;
    use protocol::decoder::{MAX_STORE_PAGES, read_from_core};
    use std::collections::BTreeSet;
    use std::fmt::Write as _;
    use std::path::Path;
    use std::time::Duration;

    /// The recording of DEC-1762's test: a megabyte is about 80 seconds at
    /// the 13 kilobytes a second of broadcast pace.
    const RECORDING_BYTES: usize = 1024 * 1024;
    /// The most the whole of it may take, in a test build on a loaded
    /// machine. The broadcast pace gave its first 2,688 bytes in that time.
    const AT_ONCE: Duration = Duration::from_secs(5);

    /// A T42 recording of at least `bytes` bytes, and every page number in
    /// it: the 800 pages a viewer keys, in magazines 1 to 8, each with every
    /// row, and further sub-pages of them until there are enough.
    fn recording(bytes: usize) -> (Vec<u8>, BTreeSet<u16>) {
        const SUB_PAGES: u16 = 4;
        let mut text = String::new();
        let mut packets = 0_usize;
        let mut numbers = BTreeSet::new();
        'whole: for subcode in 1..=SUB_PAGES {
            for number in (1_u16..=8).flat_map(|magazine| (0_u16..100).map(move |page| (magazine << 8) | ((page / 10) << 4) | (page % 10))) {
                writeln!(text, "PN,{number:03X}{subcode:02}\nSC,{subcode:04}\nPS,8000").unwrap();
                for row in 1..24 {
                    writeln!(text, "OL,{row},Page {number:03X} row {row}").unwrap();
                }
                numbers.insert(number);
                packets += 24;
                if packets * 42 >= bytes {
                    break 'whole;
                }
            }
        }
        let mut file = Vec::new();
        let mut reader = AutoReader::new(AutoConfig { format: Some(Format::Tti), encode_pages: true, ..AutoConfig::default() });
        let mut take = |item: Item<'_>| {
            if let Item::Packet { packet, .. } = item {
                let packet: Packet = packet;
                file.extend_from_slice(&packet.bytes);
            }
        };
        reader.feed(text.as_bytes(), &mut take);
        reader.finish(&mut take);
        (file, numbers)
    }

    /// DEC-1762 item 2: one tick hands a decoder a whole megabyte, which
    /// the engine's store then holds whole. Before this it was handed 2,688
    /// bytes, with the rest over the next 78 seconds.
    #[test]
    fn a_one_megabyte_t42_file_is_whole_in_the_store_at_once() {
        let (file, numbers) = recording(RECORDING_BYTES);
        assert!(file.len() >= RECORDING_BYTES, "{} bytes", file.len());
        let bytes = file.clone();
        let session = Session::with_reader(Bookmarks::default(), Box::new(move |_: &Path| Ok(Source::from_bytes(bytes.clone()))));
        let mut core = test_core();
        core.session = session;
        core.session.open("/recordings/service.t42").unwrap();
        let channel = core.session.tabs[0].channel.id;
        let written = decoder_watched(&mut core, channel);

        let started = std::time::Instant::now();
        core.tick().unwrap();
        core.flush().unwrap();
        let handed = started.elapsed();
        let mut bytes = Vec::new();
        while let Ok(chunk) = written.recv_timeout(Duration::from_millis(50)) {
            bytes.extend(chunk);
        }
        let mut engine = decoder::Engine::with_store(MAX_STORE_PAGES);
        let mut arrived = BTreeSet::new();
        let mut input = bytes.as_slice();
        let (mut data, mut ended) = (0, false);
        let decoding = std::time::Instant::now();
        while let Some(message) = read_from_core(&mut input).unwrap() {
            let pages = match message {
                ToDecoder::Data(bytes) => {
                    data += bytes.len();
                    engine.feed(&bytes)
                }
                ToDecoder::End => {
                    ended = true;
                    engine.finish()
                }
                _ => Vec::new(),
            };
            arrived.extend(pages.into_iter().map(|(page, _)| page));
        }
        let decoded = decoding.elapsed();
        assert_eq!(data, file.len(), "the whole file in one tick");
        assert!(ended, "and its end");
        assert_eq!(arrived, numbers, "every page is in the store");
        assert!(handed + decoded < AT_ONCE, "handed over in {handed:?} and decoded in {decoded:?}");
        println!("{} bytes, {} pages: handed over in {handed:?}, decoded in {decoded:?}", file.len(), numbers.len());
    }

    /// DEC-1762: a file larger than may be queued goes in steps of the
    /// queue's room, so a decoder busy with it is never taken for one that
    /// has stopped reading, and none of it is kept twice: at once, a file
    /// past `MAX_KEPT_BYTES` would be past it before its decoder had said
    /// its store was full, and that store could never grow.
    #[test]
    fn a_larger_file_goes_in_steps_and_is_not_kept_twice() {
        let file = vec![0_u8; 2 * MOST_FILE_QUEUED + 1];
        let bytes = file.clone();
        let session = Session::with_reader(Bookmarks::default(), Box::new(move |_: &Path| Ok(Source::from_bytes(bytes.clone()))));
        let mut core = test_core();
        core.session = session;
        core.session.open("/recordings/service.t42").unwrap();
        let channel = core.session.tabs[0].channel.id;
        let written = decoder_watched(&mut core, channel);
        let mut steps = Vec::new();
        while core.session.tabs[0].channel.source.has_more() {
            core.next_load = std::time::Instant::now();
            core.tick().unwrap();
            core.flush().unwrap();
            let mut bytes = Vec::new();
            while let Ok(chunk) = written.recv_timeout(Duration::from_millis(50)) {
                bytes.extend(chunk);
            }
            let mut input = bytes.as_slice();
            let mut step = 0;
            while let Some(message) = read_from_core(&mut input).unwrap() {
                if let ToDecoder::Data(bytes) = message {
                    step += bytes.len();
                }
            }
            steps.push(step);
        }
        assert_eq!(steps.iter().sum::<usize>(), file.len(), "all of it: {steps:?}");
        assert!(steps.len() > 1 && steps.iter().all(|&step| step < MOST_FILE_QUEUED + MAX_CHUNK), "in steps: {steps:?}");
        assert!(core.sleep.has_everything(channel) && core.sleep.kept(channel).is_empty(), "the file is its own copy");
    }

    /// Decision 53: the trusted core waits for an event until its next
    /// deadline, which is the sleep check alone while nothing is fed, no
    /// typing waits and the console is closed; the console open, it is
    /// the loading pace. Before this, two threads woke it every 200 ms and
    /// every 60 s for the life of the process.
    #[test]
    fn the_core_wakes_only_for_work_it_has() {
        let mut core = test_core();
        let now = std::time::Instant::now();
        let idle = core.next_wake().unwrap();
        assert!(idle >= now + SLEEP_CHECK.saturating_sub(Duration::from_secs(1)), "the sleep check alone: {:?}", idle - now);
        core.console.set_open(true);
        assert!(core.next_wake().unwrap() <= std::time::Instant::now(), "the console open: due now");
        core.tick().unwrap();
        let paced = core.next_wake().unwrap();
        assert!(paced > std::time::Instant::now() && paced <= std::time::Instant::now() + LOAD_INTERVAL, "then at the loading pace");
        core.console.set_open(false);
        assert!(core.next_wake().unwrap() >= now + SLEEP_CHECK.saturating_sub(Duration::from_secs(1)), "closed: the sleep check alone");
    }
}
