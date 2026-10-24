//! The hostile harness's watch on the broker (finding F-98): the core runs
//! against `confine`'s stand-in broker, which keeps every request and what
//! the core writes to each helper; these are the checks of them.

#![allow(clippy::arithmetic_side_effects, reason = "a test's counts")]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use confine::broker::stand_in::{StandIn, Started, Written};
use confine::broker::{Broker, Program, Request};
use protocol::decoder::{FIRST_STORE_PAGES, FromDecoder, MAX_STORE_PAGES, ToDecoder, read_from_core};
use protocol::fetcher::{FromFetcher, read_to_fetcher};

use super::super::Core;

/// What opens each file channel's page text: the path the broker was asked
/// to read follows, in hex, then [`MARK_END`].
const MARK: &str = "READ:";
const MARK_END: char = '.';
/// How long the stand-in waits, once the core has gone, for every pipe the
/// core held to close: far more than its writing threads take to end.
const FINISH_DEADLINE: Duration = Duration::from_secs(30);

/// A path's bytes in hex: the mark of its file's page.
fn hex(path: &Path) -> String {
    use std::fmt::Write as _;
    path.as_os_str().as_encoded_bytes().iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02x}");
        text
    })
}

/// What the stand-in broker reads for any file channel: one TTI page whose
/// text names, in hex, the path read, so that any bytes a decoder is given
/// say which channel's file they came from.
fn file_for(path: &Path) -> Vec<u8> {
    format!("PN,10000\r\nOL,1,{MARK}{}{MARK_END}\r\n", hex(path)).into_bytes()
}

/// The marks in `bytes`: the paths, in hex, of the files they came from.
fn marks(bytes: &[u8]) -> Vec<String> {
    let text = String::from_utf8_lossy(bytes);
    text.split(MARK).skip(1).filter_map(|after| after.split_once(MARK_END).map(|(path, _)| path.to_owned())).collect()
}

/// What a helper of the stand-in says first: a channel's decoder and a
/// fetcher that they are confined, as the real ones do (finding F-12); a
/// bookmark reader ends at once, having read nothing; the others say
/// nothing and stay.
fn first_words(helper: &Started) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    match helper.program {
        Program::Decoder if is_bookmark_reader(helper) => return None,
        Program::Decoder => protocol::decoder::write_to_core(&mut bytes, &FromDecoder::Confined).ok()?,
        Program::Fetcher => protocol::fetcher::write_to_core(&mut bytes, &FromFetcher::Confined).ok()?,
        Program::Interface | Program::Keyboard | Program::Sound | Program::Updater => {}
    }
    Some(bytes)
}

fn is_bookmark_reader(helper: &Started) -> bool {
    helper.arguments == ["--bookmarks"]
}

/// Whether the core makes a new generation of a channel's helper when it
/// starts this one: a channel's decoder or its fetcher.
fn has_generation(helper: &Started) -> bool {
    helper.program == Program::Fetcher || (helper.program == Program::Decoder && !is_bookmark_reader(helper))
}

/// The broker the harness's core is given, and what it watches.
pub(super) struct Watch {
    stand_in: StandIn,
    /// The channel each channel's helper was started for, by the
    /// stand-in's number, and the mark of its file, for a file channel.
    channels: BTreeMap<u32, (u32, Option<String>)>,
    /// The helpers started whose channel could not be told: one started
    /// and ended within one event.
    unmapped: usize,
    /// How many of the stand-in's helpers are in `channels` or `unmapped`.
    seen: usize,
    /// Hand-offs, helper starts and file reads seen, for the harness's
    /// count of what it reached.
    pub(super) hand_offs: usize,
    pub(super) starts: usize,
    pub(super) reads: usize,
}

/// What the harness found in what the core wrote to its helpers.
#[derive(Debug, Default)]
pub(super) struct Sent {
    /// Messages read by a decoder's or fetcher's own reader.
    pub(super) messages: usize,
    /// Channel decoders whose input was checked for other channels' bytes.
    pub(super) decoders: usize,
    /// File pages a decoder of their own channel was given.
    pub(super) own_files: usize,
}

impl Watch {
    /// A stand-in broker; the core reads file channels through it, as
    /// `run` makes it do.
    pub(super) fn start() -> (Option<Arc<Broker>>, Self) {
        let (broker, stand_in) = StandIn::start(Box::new(|path| Ok(file_for(path))), Box::new(first_words)).unwrap();
        (Some(broker), Self { stand_in, channels: BTreeMap::new(), unmapped: 0, seen: 0, hand_offs: 0, starts: 0, reads: 0 })
    }

    /// The requests since the last call.
    pub(super) fn take(&mut self) -> Vec<Request> {
        let requests = self.stand_in.take_requests();
        for request in &requests {
            match request {
                Request::Spawn { .. } => self.starts += 1,
                Request::OpenWeb(_) => self.hand_offs += 1,
                Request::Read(_) => self.reads += 1,
                _ => {}
            }
        }
        requests
    }

    /// Checks what an intent asked of the broker: a page intent asks for
    /// nothing; a hand-off follows only a confirmation in the strip, and
    /// only of a web address; every helper is one of the browser's roles,
    /// with arguments the core gives.
    pub(super) fn check_intent(&mut self, core: &Core, page_intent: bool, confirmation: bool, described: &str) {
        let requests = self.take();
        if page_intent {
            for request in &requests {
                assert!(
                    !matches!(request, Request::Spawn { .. } | Request::Read(_) | Request::OpenWeb(_)),
                    "a page intent asked the broker for {request:?}: {described}"
                );
            }
        }
        for request in &requests {
            if let Request::OpenWeb(address) = request {
                assert!(confirmation, "{described} handed {address} to the web browser without a confirmation in the strip");
                assert!(address.starts_with("https://") || address.starts_with("http://"), "{described} handed on {address}, not a web address");
            }
        }
        check_starts(core, &requests, described);
    }

    /// Checks what the core's timed work and flush asked of the broker,
    /// which no intent of the interface's chose: no hand-off, and helpers
    /// only of the roles.
    pub(super) fn check_timed(&mut self, core: &Core, described: &str) {
        let requests = self.take();
        for request in &requests {
            assert!(!matches!(request, Request::OpenWeb(_)), "the core's timed work after {described} handed a page on: {request:?}");
        }
        check_starts(core, &requests, described);
    }

    /// Notes the channel of each helper started since the last call: the
    /// core numbers its channels' helpers' generations in the order it
    /// starts them, from `generation`, the core's before the event.
    pub(super) fn note_channels(&mut self, core: &Core, generation: u64) {
        let started = self.stand_in.started();
        let mut next = generation;
        for helper in started.iter().skip(self.seen).filter(|helper| has_generation(helper)) {
            next = next.wrapping_add(1);
            let channel = if helper.program == Program::Decoder {
                core.decoders.iter().find(|(_, decoder)| decoder.generation() == next).map(|(channel, _)| *channel)
            } else {
                core.fetchers.iter().find(|(_, fetcher)| fetcher.generation() == next).map(|(channel, _)| *channel)
            };
            match channel {
                Some(channel) => {
                    let file = core
                        .session
                        .all_tabs()
                        .find(|tab| tab.channel.id == channel)
                        .and_then(|tab| tab.address())
                        .and_then(|address| address.file.as_deref())
                        .map(hex);
                    self.channels.insert(helper.id, (channel, file));
                }
                None => self.unmapped += 1,
            }
        }
        self.seen = started.len();
    }

    /// Once the core has gone: reads what it wrote to each helper with the
    /// helper's own reader, and checks that each decoder of a channel was
    /// given that channel's bytes only.
    pub(super) fn finish(self) -> Sent {
        let written = self.stand_in.finish(FINISH_DEADLINE).unwrap();
        let mut sent = Sent::default();
        for Written { helper, bytes, cut } in written {
            assert!(!cut, "the core wrote more than the stand-in keeps to {helper:?}");
            match helper.program {
                Program::Decoder => {
                    let messages = decoder_messages(&helper, &bytes);
                    sent.messages += messages.len();
                    if is_bookmark_reader(&helper) {
                        assert!(
                            messages.iter().all(|message| matches!(message, ToDecoder::Data(_) | ToDecoder::End)),
                            "a bookmark reader was told more than its file"
                        );
                        continue;
                    }
                    let Some((channel, file)) = self.channels.get(&helper.id) else { continue };
                    let mut data = Vec::new();
                    for message in &messages {
                        if let ToDecoder::Data(bytes) | ToDecoder::Whole { bytes, .. } = message {
                            data.extend_from_slice(bytes);
                        }
                    }
                    for mark in marks(&data) {
                        assert_eq!(Some(&mark), file.as_ref(), "the decoder of channel {channel} was given the bytes of another channel's file");
                        sent.own_files += 1;
                    }
                    sent.decoders += 1;
                }
                Program::Fetcher => {
                    let mut input = bytes.as_slice();
                    loop {
                        match read_to_fetcher(&mut input) {
                            Ok(Some(_)) => sent.messages += 1,
                            Ok(None) => break,
                            Err(error) => panic!("the core sent the fetcher {helper:?} what its reader refuses: {error}"),
                        }
                    }
                }
                Program::Interface | Program::Keyboard | Program::Sound | Program::Updater => {}
            }
        }
        assert!(self.unmapped < self.channels.len(), "the channels of {} helpers could not be told, of {}", self.unmapped, self.channels.len());
        sent
    }
}

/// Every message the core wrote to the decoder `helper`, each read by the
/// decoder's own reader, which refuses a page or sub-page its store cannot
/// hold.
fn decoder_messages(helper: &Started, bytes: &[u8]) -> Vec<ToDecoder> {
    let mut input = bytes;
    let mut messages = Vec::new();
    loop {
        match read_from_core(&mut input) {
            Ok(Some(message)) => messages.push(message),
            Ok(None) => return messages,
            Err(error) => panic!("the core sent the decoder {helper:?} what its reader refuses: {error}"),
        }
    }
}

/// Every helper started is one of the roles the core starts on an
/// interface's intent, with the arguments the core gives it.
fn check_starts(core: &Core, requests: &[Request], described: &str) {
    for request in requests {
        let Request::Spawn { program, local_network, arguments } = request else { continue };
        match program {
            // Each is started once, as the browser starts.
            Program::Interface | Program::Sound => panic!("{described} started the {}", program.name()),
            Program::Decoder => {
                let store = match arguments.as_slice() {
                    [flag] if flag == "--bookmarks" => true,
                    [flag, pages] if flag == "--store-pages" => {
                        pages.parse::<usize>().is_ok_and(|pages| (FIRST_STORE_PAGES..=MAX_STORE_PAGES).contains(&pages))
                    }
                    _ => false,
                };
                assert!(store, "{described} started a decoder with {arguments:?}");
                assert!(!local_network, "{described} let a decoder reach the local network");
            }
            Program::Fetcher => {
                let address = arguments.first().map(String::as_str).unwrap_or_default();
                assert!(
                    core.session.all_tabs().any(|tab| tab.channel.web.as_deref() == Some(address)),
                    "{described} started a fetcher for {address}, which no tab holds"
                );
                assert_eq!(
                    *local_network,
                    arguments.iter().any(|argument| argument == "--allow-local"),
                    "{described}: a fetcher's local network as not asked for"
                );
            }
            Program::Keyboard | Program::Updater => {}
        }
    }
}

/// Every decoder and fetcher the core holds, once an event is handled, is
/// for a channel a tab holds.
pub(super) fn check_helpers(core: &Core, described: &str) {
    for channel in core.decoders.keys().chain(core.fetchers.keys()) {
        assert!(core.session.all_tabs().any(|tab| tab.channel.id == *channel), "{described} left a helper of channel {channel}, which no tab holds");
    }
}
